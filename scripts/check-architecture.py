#!/usr/bin/env python3
"""Executable architecture rules (AGENTS.md rules 3 and 5, tasks T001-032).

Reads `cargo metadata` and fails when:
  * the domain crate depends on a forbidden infrastructure/format crate, or on a
    non-allowlisted crate at all;
  * a layer depends on a layer it must not know about (application -> adapters, ...);
  * a package other than the test package depends on the `iceberg` crates, whatever the kind;
  * the lake runner reaches the container engine, directly or through any crate it links (ADR-0079, rule 2);
  * the workspace dependency graph has a cycle;
  * the lease row of a scheduler shard (`scheduler_shards`) is not a leaf lock (ADR-0050 decision 7, ADR-0020): the SQL
    literals of `crates/infra-postgres/src` that name it take it `FOR SHARE` (a fence) or, in `partitioning.rs` only, `FOR UPDATE`
    with `SKIP LOCKED` (the keeper, which never waits); `partitioning.rs` names no run table; and the two fences stay `FOR SHARE`;
  * a `run.finished` body written by the PostgreSQL adapter (the message subscribers are sent when a run ends) is built after
    `add_schedule_origin`, in the same function (FR-018-046, D2143): a writer that forgets it tells the subscribers of a run a
    schedule started nothing about the schedule, and nothing else would fail;
  * the run-input validator (`crates/infra-schema/src/run_input/`, design 043) names `jsonschema` (a `use`, an `extern crate` or
    a qualified path): it is the oracle of a test only, never on the path (D043-074, ADR-0090);
  * `tokio` is a dependency of `sagat-application` with a default feature or a feature other than `rt`, or a build dependency,
    or `tokio` is named in `crates/application/src/**` outside `collection_source.rs` and test modules: the runtime of the
    run-input gate lives in the adapter behind the port, and the `rt` exception serves design 042's parse alone (ADR-0092);
  * a composition root under `apps/*/src` builds a `RunService`, `Scheduler`, `EventDispatcher` or `ScheduleService` without
    `.with_run_inputs` in its builder chain (ADR-0092 decision 2, D043-161);
  * an `INSERT INTO runs` is written outside its three places (`create` in `crates/infra-postgres/src/execution.rs`,
    `continue_as_new` and `continue_evaluated` in `crates/infra-postgres/src/chain_progress.rs`), counted per function
    (FR-043-016): every run row is written where the input of the run is a `CheckedRunInput`;
  * the legacy continue-as-new (`.continue_as_new(`, which carries an input nobody checked) is called outside the tests, other
    than by the one forwarder, `continue_as_new` in `crates/infra-postgres/src/chain_progress.rs` (FR-043-016, D043-122);
  * the `test-support` feature of `sagat-application` (the constructor of `CheckedRunInput` that skips the gate) is enabled
    anywhere but a `[dev-dependencies]` entry (FR-043-016);
  * a `CheckedRunInput` is built (a struct expression or an associated function call) outside
    `crates/application/src/run_input.rs` and the tests, or has a `Default` or a `Deserialize` anywhere (FR-043-016, D043-045).

Usage: python3 scripts/check-architecture.py [--metadata FILE] [--postgres-src DIR] [--run-input-src DIR] [--rust-root DIR]
  --metadata FILE     read a saved `cargo metadata --format-version 1` JSON instead of
                      running cargo (used by the negative tests).
  --postgres-src DIR  the source directory of the PostgreSQL adapter (default: crates/infra-postgres/src of this repository;
                      the negative tests give a small tree of their own).
  --run-input-src DIR the source directory of the run-input validator (default: crates/infra-schema/src/run_input of this
                      repository; the negative tests give their own).
  --rust-root DIR     the root whose `crates/*/src` and `apps/*/src` the rules of the run creators read (default: this
                      repository; the negative tests give a small tree of their own).
"""
import json
import pathlib
import re
import subprocess
import sys

# Crates the domain may use (normal dependencies only; dev-dependencies are not checked).
DOMAIN_ALLOWED = {"thiserror", "time", "uuid", "zeroize"}
# Never allowed in the domain, even if someone adds them to DOMAIN_ALLOWED by mistake.
DOMAIN_FORBIDDEN = {
    "axum", "hyper", "tower", "tower-http", "reqwest", "tonic", "prost",
    "sqlx", "tokio-postgres", "postgres", "diesel", "sea-orm",
    "async-nats", "nats", "redis", "rdkafka", "aws-sdk-sqs", "kube", "k8s-openapi",
    "serde_json", "tokio", "tracing-opentelemetry", "opentelemetry",
}

# The `iceberg` crate and its `iceberg-*` siblings may be used by the test package only (any kind).
ICEBERG_CRATE = "iceberg"
ICEBERG_TEST_ONLY_PACKAGES = {"sagat-tests"}

# ADR-0079, rule 2: the lake runner holds no container-engine access. The sandbox runner is the one process that links the engine
# (`bollard`, through the crate that wraps it), and the lake runner asks it for an attempt over a socket instead.
LAKE_RUNNER_PACKAGES = {"sagat-database-runner", "sagat-infra-lake"}
CONTAINER_ENGINE_CRATES = {"bollard", "sagat-infra-sandbox"}

# Crates that are application services (they orchestrate ports) rather than adapters or utilities.
APPLICATION_CRATES = {"scheduler", "saga"}

# Layer of each workspace package, derived from its directory under the repo root.
def layer_of(manifest_path: str, root: str) -> str:
    rel = manifest_path[len(root):].lstrip("/") if manifest_path.startswith(root) else manifest_path
    parts = rel.split("/")
    if parts[0] == "apps":
        return "app"
    if parts[0] == "crates":
        name = parts[1]
        if name == "domain":
            return "domain"
        if name == "application":
            return "application"
        if name.startswith("infra-"):
            return "adapter"
        if name.startswith("api-"):
            return "adapter"
        if name in APPLICATION_CRATES:
            return "application"  # services built on the ports: they know the domain and the ports only
        return "support"  # telemetry, ... (no extra restriction beyond acyclicity)
    return "other"

# layer -> layers it may depend on
ALLOWED_LAYER_DEPS = {
    "domain": {"domain"},
    "application": {"domain", "application", "support"},
    "support": {"domain", "support"},
    "adapter": {"domain", "application", "support", "adapter"},
    "app": {"domain", "application", "support", "adapter", "app"},
    "other": {"domain", "application", "support", "adapter", "app", "other"},
}


def load(metadata_file):
    if metadata_file:
        with open(metadata_file) as f:
            return json.load(f)
    out = subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        check=True, capture_output=True, text=True,
    ).stdout
    return json.loads(out)

# --- the lease row is a leaf lock (ADR-0050 decision 7, ADR-0020) ----------------------------------------------------------
# A row of `scheduler_shards` is taken `FOR SHARE` by every fence (shared locks never conflict) and `FOR UPDATE SKIP LOCKED` by the
# keeper (it never waits, and it takes no other row). Nothing waits on it while holding another lock, nothing holding it waits for
# another lock, so it is outside the slot order and takes no part in a cycle. The rule keeps it so: a statement that takes it more
# strongly, or one that waits for it, or a keeper that reads a run, is an error here before it is a deadlock in production.
LEASE_TABLE = "scheduler_shards"
LEASE_KEEPER_FILE = "partitioning.rs"
# (file, function): the statements of these on the lease row stay `FOR SHARE`.
LEASE_FENCES = (("execution.rs", "locked_scheduler_time_sql"), ("schedules.rs", "check_fence"))
STRONG_LOCK = re.compile(
    r"\bFOR\s+(?:NO\s+KEY\s+)?UPDATE\b|\bUPDATE\s+" + LEASE_TABLE + r"\b|\bDELETE\s+FROM\s+" + LEASE_TABLE + r"\b"
    r"|\bINSERT\s+INTO\s+" + LEASE_TABLE + r"\b|\bTRUNCATE\b",
    re.IGNORECASE,
)


def rust_literals(text):
    """The string literals of Rust source, with the source masked: `(masked, [(offset, content)])`.

    The masked copy has the comments and the content of every string and character literal replaced by spaces (newlines and
    offsets kept), so that braces can be matched on it; a literal's content has its escapes read as far as SQL needs (a
    backslash before a newline joins the lines, as Rust does)."""
    masked = list(text)
    literals = []
    i, n = 0, len(text)

    def blank(a, b):
        for k in range(a, b):
            if masked[k] != "\n":
                masked[k] = " "

    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            j = n if j < 0 else j
            blank(i, j)
            i = j
        elif text.startswith("/*", i):
            j = text.find("*/", i + 2)
            j = n if j < 0 else j + 2
            blank(i, j)
            i = j
        elif c == "r" and re.match(r'r#*"', text[i:i + 8]) and (i == 0 or not (text[i - 1].isalnum() or text[i - 1] == "_")):
            hashes = len(re.match(r"r(#*)", text[i:]).group(1))
            start = i + 1 + hashes + 1
            end = text.find('"' + "#" * hashes, start)
            end = n if end < 0 else end
            literals.append((start, text[start:end]))
            blank(start, end)
            i = end + 1 + hashes
        elif c == '"':
            j, out = i + 1, []
            while j < n and text[j] != '"':
                if text[j] == "\\" and j + 1 < n:
                    if text[j + 1] == "\n":
                        j += 2
                        while j < n and text[j] in " \t\n":
                            j += 1
                        continue
                    out.append(text[j + 1])
                    j += 2
                    continue
                out.append(text[j])
                j += 1
            literals.append((i + 1, "".join(out)))
            blank(i + 1, j)
            i = j + 1
        elif c == "'":
            m = re.match(r"'(?:\\.[^']*|[^\\'])'", text[i:i + 12])
            if m:
                blank(i + 1, i + m.end() - 1)
                i += m.end()
            else:
                i += 1  # a lifetime
        else:
            i += 1
    return "".join(masked), literals


def function_span(masked, name):
    """The `(start, end)` offsets of the body of `fn name`, matching braces on the masked source; `None` when there is none."""
    m = re.search(r"\bfn\s+" + re.escape(name) + r"\b", masked)
    if not m:
        return None
    i = masked.find("{", m.end())
    if i < 0:
        return None
    depth = 0
    for j in range(i, len(masked)):
        if masked[j] == "{":
            depth += 1
        elif masked[j] == "}":
            depth -= 1
            if depth == 0:
                return i, j + 1
    return None


def lease_row_errors(src):
    """The errors of the lease-row rule over the PostgreSQL adapter's source directory `src`."""
    errors = []
    root = pathlib.Path(src)
    files = {}
    for path in sorted(root.rglob("*.rs")):
        text = path.read_text()
        masked, literals = rust_literals(text)
        files[path.relative_to(root).as_posix()] = (text, masked, literals)

    def line(text, offset):
        return text.count("\n", 0, offset) + 1

    for rel, (text, masked, literals) in files.items():
        for offset, sql in literals:
            if LEASE_TABLE not in sql:
                continue
            where = f"crates/infra-postgres/src/{rel}:{line(text, offset)}"
            if STRONG_LOCK.search(sql):
                if rel != LEASE_KEEPER_FILE:
                    errors.append(
                        f"{where}: a statement on `{LEASE_TABLE}` that locks the row more strongly than FOR SHARE belongs to "
                        f"{LEASE_KEEPER_FILE} only (the keeper): the lease row is a leaf lock (ADR-0050 decision 7)"
                    )
                elif not re.search(r"\bSKIP\s+LOCKED\b", sql, re.IGNORECASE):
                    errors.append(
                        f"{where}: a keeper statement on `{LEASE_TABLE}` takes the row without SKIP LOCKED: it would wait for a "
                        f"fence, and the lease row would stop being a leaf lock (ADR-0050 decision 7)"
                    )
    keeper = files.get(LEASE_KEEPER_FILE)
    if keeper is None:
        errors.append(f"the keeper's file {LEASE_KEEPER_FILE} is named by the lease-row rule and is not in {src}: the rule would check nothing")
    else:
        text, _, literals = keeper
        for offset, sql in literals:
            if re.search(r"\bruns\b", sql):
                errors.append(
                    f"crates/infra-postgres/src/{LEASE_KEEPER_FILE}:{line(text, offset)}: the keeper names `runs`: it must take no run row "
                    f"(the lease row is a leaf lock only while the keeper holds nothing else, ADR-0050 decision 7)"
                )
    for rel, name in LEASE_FENCES:
        found = files.get(rel)
        span = function_span(found[1], name) if found else None
        if span is None:
            errors.append(f"`{name}` ({rel}) is named by the lease-row rule and is not there: the rule would check nothing (rename it in LEASE_FENCES)")
            continue
        text, _, literals = found
        mine = [(o, q) for o, q in literals if span[0] <= o < span[1] and LEASE_TABLE in q]
        if not any(re.search(r"\bFOR\s+SHARE\b", q, re.IGNORECASE) for _, q in mine):
            errors.append(f"`{name}` ({rel}) has no `FOR SHARE` statement on `{LEASE_TABLE}`: a fence takes the lease row shared (ADR-0050 decision 7)")
        for offset, sql in mine:
            if STRONG_LOCK.search(sql):
                errors.append(
                    f"crates/infra-postgres/src/{rel}:{line(text, offset)}: `{name}` locks `{LEASE_TABLE}` more strongly than FOR SHARE: "
                    f"a fence is a shared lock, or the lease row stops being a leaf lock (ADR-0050 decision 7)"
                )
    return errors


# --- every `run.finished` body says the schedule that started the run (FR-018-046, D2143) ----------------------------------------
# The body of the message subscribers are sent when a run ends is written in several places of the adapter (the run transaction, the
# collection finish, the loop terminal path, the chain's final head). Each adds `scheduleId` and `nominalTime` for a run a schedule
# started by calling `add_schedule_origin` on its `data` before it builds the body. A writer added later, or an edit that drops the
# call, would send the subscribers of a scheduled run a body that says nothing of its schedule, and no other test fails for it.
RUN_FINISHED_BODY = re.compile(r'"type"\s*:\s*"run\.finished"')
SCHEDULE_ORIGIN = "add_schedule_origin"


def enclosing_function(masked, offset):
    """The `(name, start, end)` of the innermost `fn` of the masked source that contains `offset`; `None` when there is none."""
    best = None
    for m in re.finditer(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)", masked[:offset]):
        span = function_span(masked[m.start():], m.group(1))
        if span is not None and m.start() + span[0] <= offset < m.start() + span[1]:
            best = (m.group(1), m.start() + span[0], m.start() + span[1])
    return best


def run_finished_errors(src):
    """The errors of the `run.finished` rule over the PostgreSQL adapter's source directory `src`."""
    errors = []
    root = pathlib.Path(src)
    found = 0
    defined = False
    for path in sorted(root.rglob("*.rs")):
        text = path.read_text()
        masked, _ = rust_literals(text)
        rel = path.relative_to(root).as_posix()
        if re.search(r"\bfn\s+" + SCHEDULE_ORIGIN + r"\b", masked):
            defined = True
        for m in RUN_FINISHED_BODY.finditer(text):
            if masked[m.start()] != '"':
                continue  # in a comment
            found += 1
            where = f"crates/infra-postgres/src/{rel}:{text.count(chr(10), 0, m.start()) + 1}"
            inside = enclosing_function(masked, m.start())
            if inside is None or SCHEDULE_ORIGIN + "(" not in masked[inside[1]:m.start()]:
                errors.append(
                    f"{where}: a `run.finished` body is built without `{SCHEDULE_ORIGIN}` before it in the same function: the "
                    f"subscribers of a run a schedule started would be told nothing about the schedule (FR-018-046)"
                )
    if not defined:
        errors.append(f"`{SCHEDULE_ORIGIN}` is named by the `run.finished` rule and is not defined in {src}: the rule would check nothing")
    if found == 0:
        errors.append(f"no `run.finished` body was found in {src}: the rule would check nothing (is the literal spelled otherwise?)")
    return errors


# --- the run-input validator is the platform's own (design 043, D043-074) -----------------------------------------------------
# `jsonschema` is the oracle of the validator's property test and the compiler of the scripts' schemas; on the path of a run input it
# would bring back what four reviews could not bound. Any mention of it in the code of the module (comments and strings aside) fails.
JSONSCHEMA_NAME = re.compile(r"\bjsonschema\b")


def run_input_errors(src):
    """The errors of the `jsonschema` rule over the run-input validator's source directory `src`."""
    root = pathlib.Path(src)
    files = sorted(root.rglob("*.rs")) if root.is_dir() else []
    if not files:
        return [f"the run-input validator is named by the jsonschema rule and {src} holds no Rust source: the rule would check nothing"]
    errors = []
    for path in files:
        text = path.read_text()
        masked, _ = rust_literals(text)
        for m in JSONSCHEMA_NAME.finditer(masked):
            line = text.count(chr(10), 0, m.start()) + 1
            errors.append(
                f"crates/infra-schema/src/run_input/{path.relative_to(root).as_posix()}:{line}: the run-input validator names "
                f"`jsonschema`; it is the oracle of a test only, never on the path (D043-074, ADR-0090)"
            )
    return errors


# --- the application holds no runtime (D043-071, D043-077) --------------------------------------------------------------------
APPLICATION_PACKAGE = "sagat-application"


def application_runtime_errors(pkgs):
    errors = []
    for p in pkgs.values():
        if p["name"] != APPLICATION_PACKAGE:
            continue
        for d in p["dependencies"]:
            # Design 042 parses a blob source's array in a blocking task (`collection_source.rs`), the one use the
            # application makes of a runtime: `tokio` with no feature but `rt` is that and nothing more (D043-150, G043-051).
            # The gate of design 043 holds no runtime whatever this allows: `run_input.rs` names none (`run_input_runtime_errors`).
            features = set(d.get("features") or [])
            if d["name"] == "tokio" and d.get("kind") in (None, "normal") and not d.get("uses_default_features", True) \
                    and features <= {"rt"}:
                continue
            if d["name"] == "tokio" and d.get("kind") in (None, "normal", "build"):
                errors.append(
                    f"'{APPLICATION_PACKAGE}' must not have `tokio` as a {d.get('kind') or 'normal'} dependency: the gate is plain async "
                    f"code over its port and the runtime lives in the adapter (D043-071, D043-077); a dev-dependency is fine"
                )
    return errors


# The one file of `sagat-application` that may name `tokio`: design 042's blocking parse of a blob source's array
# (ADR-0092). The gate of design 043 is plain async code over its port, and every other module of the application too.
APPLICATION_RUNTIME_HOME = "crates/application/src/collection_source.rs"


def run_input_runtime_errors(root):
    """`sagat-application` names `tokio` only in `collection_source.rs` and in test modules (ADR-0092, D043-071, D043-154)."""
    base = pathlib.Path(root)
    errors = []
    for path in sorted(base.glob("crates/application/src/**/*.rs")):
        rel = path.relative_to(base).as_posix()
        if rel == APPLICATION_RUNTIME_HOME:
            continue
        text = path.read_text()
        masked = without_test_modules(rust_literals(text)[0])
        errors.extend(
            f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: `sagat-application` names `tokio` outside "
            f"`{APPLICATION_RUNTIME_HOME}`: the `rt` exception of ADR-0092 serves design 042's blocking parse alone, and "
            f"the gate is plain async code over its port (D043-071)"
            for m in re.finditer(r"\btokio\b", masked)
        )
    return errors


# --- every run is created with a checked input (design 043, FR-043-016) -------------------------------------------------------
# the table name bare, quoted or qualified by its schema (`public.runs`, `"runs"`, `\"public\".\"runs\"`)
RUN_INSERT = re.compile(r'\bINSERT\s+INTO\s+(?:\\?"?public\\?"?\s*\.\s*)?\\?"?runs\b', re.IGNORECASE)
# (file, function) of the one forwarder that may call the legacy continue-as-new: `PgRunStore`'s `ChainProgressStore`
# implementation, which hands it to `PgChainProgressStore` (D043-122); keyed by file and function, so that a function of
# that name anywhere else is not exempt (architecture review of 2026-11-15, Major 2).
LEGACY_FORWARDER = ("crates/infra-postgres/src/chain_progress.rs", "continue_as_new")
# (file, function) -> how many `INSERT INTO runs` it may hold: counted per function, so that a moved line is not a failure.
RUN_INSERT_PLACES = {
    ("crates/infra-postgres/src/execution.rs", "create"): 1,
    ("crates/infra-postgres/src/chain_progress.rs", "continue_as_new"): 1,
    ("crates/infra-postgres/src/chain_progress.rs", "continue_evaluated"): 1,
}
LEGACY_CONTINUATION = re.compile(r"\.\s*continue_as_new\s*\(")
CHECKED_INPUT = "CheckedRunInput"
CHECKED_INPUT_HOME = "crates/application/src/run_input.rs"
TEST_SUPPORT_FEATURE = "test-support"


def without_test_modules(masked):
    """The masked source with every `#[cfg(test)]` item (a module, a function) blanked, newlines kept."""
    out = list(masked)
    for m in re.finditer(r"#\[cfg\(test\)\]", masked):
        i = masked.find("{", m.end())
        semi = masked.find(";", m.end())
        if i < 0 or (0 <= semi < i):
            continue
        depth = 0
        for j in range(i, len(masked)):
            if masked[j] == "{":
                depth += 1
            elif masked[j] == "}":
                depth -= 1
                if depth == 0:
                    for k in range(m.start(), j + 1):
                        if out[k] != "\n":
                            out[k] = " "
                    break
    return "".join(out)


def production_sources(root):
    """`(relative path, text, masked without test modules, literals)` of the Rust sources under `crates/*/src` and `apps/*/src`."""
    base = pathlib.Path(root)
    for top in ("crates", "apps"):
        for path in sorted(base.glob(f"{top}/*/src/**/*.rs")):
            text = path.read_text()
            masked, literals = rust_literals(text)
            yield path.relative_to(base).as_posix(), text, without_test_modules(masked), literals


def run_creator_errors(root):
    """The errors of the rules that keep every run created with a checked input, over the tree at `root`."""
    errors = []
    counts = {}
    for rel, text, masked, literals in production_sources(root):
        for offset, sql in literals:
            # a literal inside a test module was blanked with it, its opening quote included
            if not RUN_INSERT.search(sql) or masked[offset - 1] != '"':
                continue
            inside = enclosing_function(masked, offset)
            key = (rel, inside[0] if inside else None)
            counts[key] = counts.get(key, 0) + 1
            if counts[key] > RUN_INSERT_PLACES.get(key, 0):
                errors.append(
                    f"{rel}:{text.count(chr(10), 0, offset) + 1}: an `INSERT INTO runs` outside its three places (in "
                    f"`{key[1]}`): a run row is written only where its input is a `CheckedRunInput` (FR-043-016, ADR-0090)"
                )
        for m in LEGACY_CONTINUATION.finditer(masked):
            inside = enclosing_function(masked, m.start())
            if inside is not None and (rel, inside[0]) == LEGACY_FORWARDER:
                continue  # the forwarder of the trait method to its implementation
            errors.append(
                f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: the legacy continue-as-new is called outside the tests: it "
                f"carries an input nobody checked; use the evaluated continuation (FR-043-016, ADR-0090 decision 1)"
            )
        for m in re.finditer(r"\b" + CHECKED_INPUT + r"\b", masked):
            line = text.count(chr(10), 0, m.start()) + 1
            after = masked[m.end():m.end() + 64]
            # the path that qualifies the name (`crate::run_input::`) is not what decides whether it is a construction
            before = re.sub(r"(?:\b\w+\s*::\s*)+$", "", masked[max(0, m.start() - 96):m.start()]).rstrip()
            associated = re.match(r"\s*::\s*[A-Za-z_]", after)
            struct_expression = re.match(r"\s*\{", after) and not re.search(
                r"(->|\bimpl|\bfor|\bstruct|\benum|\btrait|\bdyn|&|<|,|:)$", before
            )
            if (associated or struct_expression) and rel != CHECKED_INPUT_HOME:
                errors.append(
                    f"{rel}:{line}: a `{CHECKED_INPUT}` is built outside {CHECKED_INPUT_HOME}: only the gate builds one "
                    f"(FR-043-016, D043-032)"
                )
        for m in re.finditer(r"#\[derive\(([^)]*)\)\]\s*(?:#\[[^\]]*\]\s*)*(?:pub(?:\([^)]*\))?\s+)?struct\s+" + CHECKED_INPUT + r"\b", masked):
            if re.search(r"\b(Default|Deserialize)\b", m.group(1)):
                errors.append(
                    f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: `{CHECKED_INPUT}` derives `Default` or `Deserialize`: an "
                    f"input that no gate checked could be made from nothing or from bytes (D043-045)"
                )
        for m in re.finditer(r"\bimpl\b[^{;]*\b(Default|Deserialize)\b[^{;]*\bfor\s+(?:[\w:]+::)?" + CHECKED_INPUT + r"\b", masked):
            errors.append(
                f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: `{CHECKED_INPUT}` implements `{m.group(1)}`: an input that "
                f"no gate checked could be made from nothing or from bytes (D043-045)"
            )
    # a migration writes no run either: a function or a trigger that inserts one would create it unchecked
    base = pathlib.Path(root)
    for path in sorted(base.glob("crates/infra-postgres/migrations/*.up.sql")):
        text = path.read_text()
        for m in RUN_INSERT.finditer(text):
            errors.append(
                f"{path.relative_to(base).as_posix()}:{text.count(chr(10), 0, m.start()) + 1}: a migration writes an "
                f"`INSERT INTO runs`: a run row is written only where its input is a `CheckedRunInput` (FR-043-016)"
            )
    for key, allowed in RUN_INSERT_PLACES.items():
        if counts.get(key, 0) == 0 and (pathlib.Path(root) / key[0]).exists():
            errors.append(
                f"`{key[1]}` ({key[0]}) is named by the run-insert rule and holds no `INSERT INTO runs`: the rule would check "
                f"nothing (rename it in RUN_INSERT_PLACES)"
            )
    if not any((pathlib.Path(root) / key[0]).exists() for key in RUN_INSERT_PLACES):
        errors.append(f"no file named by the run-insert rule is under {root}: the rule would check nothing")
    return errors


# Constitution 5.1.0, principle I: journal rows leave only by two audited paths (feature 020's per-run purge, design 049's
# partition retirement), both privileged, and the runtime role holds no DELETE or TRUNCATE on runs, steps, the journal
# or payloads. A migration (up or down) is where such a grant would come from, so every way of giving it is refused: a
# GRANT of DELETE, TRUNCATE or ALL on one of those tables (named, or through ALL TABLES or default privileges) to the
# runtime role or to PUBLIC, with or without GRANT OPTION or GRANTED BY; any role granted to the runtime role; a table
# given to the runtime role as owner; and such a GRANT built as dynamic SQL. A runtime check of the privileges the role
# really has (through memberships too) is feature 020's privilege sweep.
JOURNAL_TABLES = {"runs", "steps", "run_events", "payloads", "run_payloads"}
RUNTIME_GRANTEES = {"sagat_app", "public"}
GRANT_STATEMENT = re.compile(r"\bgrant\s+(?P<privs>.*?)\s+on\s+(?P<target>.*?)\s+to\s+(?P<grantees>[^;]*);", re.I | re.S)
ROLE_GRANT = re.compile(r"\bgrant\s+(?P<roles>[\w\"]+(?:\s*,\s*[\w\"]+)*)\s+to\s+(?P<grantees>[^;]*);", re.I)
OWNER_TO_RUNTIME = re.compile(r"\bowner\s+to\s+\"?sagat_app\"?\b", re.I)
SQL_LITERAL = re.compile(r"'(?:[^']|'')*'")
DYNAMIC_GRANT = re.compile(r"\bgrant\b.*\b(delete|truncate|all)\b.*\bto\s+\"?sagat_app\b", re.I | re.S)
GRANTEE_TAIL = re.compile(r"\b(with|granted\s+by)\b.*$", re.I | re.S)
JOURNAL_PARTITION = re.compile(r"(" + "|".join(sorted(["runs", "steps", "run_events", "payloads", "run_payloads"])) + r")_([0-9]{4}_[0-9]{2}|legacy)")
MEMBERSHIP_TO_RUNTIME = [
    re.compile(r"\bcreate\s+(role|group|user)\b[^;]*\b(role|user|admin)\s+\"?sagat_app\b", re.I),
    re.compile(r"\bcreate\s+(role|user)\s+\"?sagat_app\"?[^;]*\bin\s+(role|group)\b", re.I),
    re.compile(r"\balter\s+group\b[^;]*\badd\s+user\b[^;]*\bsagat_app\b", re.I),
    re.compile(r"\breassign\s+owned\s+by\b[^;]*\bto\s+\"?sagat_app\b", re.I),
]
FORMAT_GRANT = re.compile(r"\bformat\s*\(\s*'(?:[^']|'')*\bgrant\b(?:[^']|'')*'[^;]*", re.I)


def grantee_set(text):
    return {g.strip().strip('"').lower() for g in GRANTEE_TAIL.sub("", text).split(",") if g.strip()}


def journal_delete_grant_errors(root):
    errors = []
    base = pathlib.Path(root)
    for path in sorted(base.glob("crates/infra-postgres/migrations/*.sql")):
        text = path.read_text()
        rel = path.relative_to(base).as_posix()
        # comments and REVOKE statements carry no grant ("REVOKE GRANT OPTION FOR ..." is not one)
        masked = re.sub(r"/\*.*?\*/", lambda m: re.sub(r"[^\n]", " ", m.group(0)), text, flags=re.S)
        masked = re.sub(r"--[^\n]*", lambda m: " " * len(m.group(0)), masked)
        if not masked.rstrip().endswith(";"):
            masked = masked.rstrip() + ";"  # a last statement without its semicolon is still read
        masked = re.sub(r"\brevoke\b[^;]*;", lambda m: re.sub(r"[^\n]", " ", m.group(0)), masked, flags=re.I)
        line = lambda offset: text.count(chr(10), 0, offset) + 1
        why = "journal rows leave only by the two audited privileged paths of principle I (constitution 5.1.0)"
        for m in GRANT_STATEMENT.finditer(masked):
            if not re.search(r"\b(delete|truncate|all)\b", m.group("privs"), re.I):
                continue
            target = re.sub(r"\s+", " ", m.group("target").strip().lower())
            if re.search(r"\ball tables\b", target) or target == "tables":
                hit = "every table"
            else:
                names = {n.strip().strip('"').split(".")[-1].strip('"')
                         for n in re.sub(r"^table\s+", "", target).split(",")}
                hit = ", ".join(sorted(n for n in names if n in JOURNAL_TABLES or JOURNAL_PARTITION.fullmatch(n)))
            if hit and grantee_set(m.group("grantees")) & RUNTIME_GRANTEES:
                privileges = re.sub(r"\s+", " ", m.group("privs").strip())
                errors.append(f"{rel}:{line(m.start())}: a migration grants {privileges} on {hit} to the runtime role: {why}")
        for m in ROLE_GRANT.finditer(masked):
            if re.search(r"\bon\b", m.group(0), re.I):
                continue
            if grantee_set(m.group("grantees")) & RUNTIME_GRANTEES:
                errors.append(f"{rel}:{line(m.start())}: a migration grants the role {m.group('roles')} to the runtime role, "
                              f"which would carry that role's privileges: {why}")
        for m in OWNER_TO_RUNTIME.finditer(masked):
            errors.append(f"{rel}:{line(m.start())}: a migration makes the runtime role the owner of a relation, which may "
                          f"delete from it: {why}")
        for pattern in MEMBERSHIP_TO_RUNTIME:
            for m in pattern.finditer(masked):
                errors.append(f"{rel}:{line(m.start())}: a migration makes the runtime role a member of a role, or the "
                              f"owner of another role's objects, which would carry their privileges: {why}")
        for m in FORMAT_GRANT.finditer(masked):
            if re.search(r"\bsagat_app\b", m.group(0)) and re.search(r"\b(delete|truncate|all)\b", m.group(0), re.I):
                errors.append(f"{rel}:{line(m.start())}: a migration builds a GRANT of DELETE, TRUNCATE or ALL to the "
                              f"runtime role as dynamic SQL: {why}")
        for m in SQL_LITERAL.finditer(masked):
            if FORMAT_GRANT.search(masked[max(0, m.start() - 40):m.end()]):
                continue  # already judged as a whole format() call above
            if not DYNAMIC_GRANT.search(m.group(0)):
                continue
            errors.append(f"{rel}:{line(m.start())}: a migration builds a GRANT of DELETE, TRUNCATE or ALL to the runtime role "
                          f"as dynamic SQL: {why}")
    return errors


# The creators a composition root builds, each of which holds the unwired gate until it is given one (D043-120): a
# service built without `.with_run_inputs` refuses every start of a version with an `inputSchema` as busy, and an engine
# creator retries it forever (architecture review of 2026-11-15, Major 3).
GATED_CONSTRUCTORS = re.compile(r"\b(RunService|Scheduler|EventDispatcher|ScheduleService)\s*::\s*new\s*\(")


def balanced_end(masked, i):
    """The index after the group that opens at `masked[i]` (`(`), or `None` when it does not close."""
    depth = 0
    for j in range(i, len(masked)):
        if masked[j] in "([{":
            depth += 1
        elif masked[j] in ")]}":
            depth -= 1
            if depth == 0:
                return j + 1
    return None


def gate_wiring_errors(root):
    """Every creator a composition root under `apps/*/src` builds is given the gate in its builder chain."""
    errors = []
    base = pathlib.Path(root)
    for path in sorted(base.glob("apps/*/src/**/*.rs")):
        rel = path.relative_to(base).as_posix()
        text = path.read_text()
        masked = without_test_modules(rust_literals(text)[0])
        for m in GATED_CONSTRUCTORS.finditer(masked):
            end = balanced_end(masked, m.end() - 1)
            methods = []
            while end is not None:
                chained = re.match(r"\s*\.\s*(\w+)\s*\(", masked[end:])
                if not chained:
                    break
                methods.append(chained.group(1))
                end = balanced_end(masked, end + chained.end() - 1)
            if "with_run_inputs" not in methods:
                errors.append(
                    f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: `{m.group(1)}::new` is not given the run-input gate "
                    f"(`.with_run_inputs`) in its builder chain: without it every start of a version with an `inputSchema` "
                    f"is refused as busy (D043-120)"
                )
    return errors


# Feature 009, decision D60 (gap G041-028): a lookup that found nothing in the caller's tenant is denied and audited by one
# function, `tenant_boundary::deny_unresolved` (or `deny_unless_own`, for a check that can also find the caller's own resource,
# or `deny_unresolved_among`, for a call that named several),
# so that the caller's tenant gets the same record for a missing and a foreign identifier and only the platform's security audit
# names the foreign one. A path that wrote its own record would bring the existence oracle back. So, in production sources:
# every call of a check for another tenant's resource (a method named `*_outside`, `run_tenancy` or `*another_tenant*`) is an
# argument of one of the two functions, unless it is the forwarder of a method of the same name; and the platform's action
# (`AuditAction::CrossTenantAccessDenied`, the text `access.denied.cross_tenant`) and the tenant's record
# (`AuditAction::ResourceNotFoundDenied`) are named only by the domain, which builds both from one `BoundaryDenial`.
BOUNDARY_HOME = "crates/application/src/tenant_boundary.rs"
BOUNDARY_DOMAIN = "crates/domain/src/audit.rs"
BOUNDARY_CHECK_NAME = r"\w+_outside|run_tenancy|\w*another_tenant\w*"
BOUNDARY_CHECK_CALL = re.compile(r"(?<![\w])(" + BOUNDARY_CHECK_NAME + r")\s*\(")
# what records a denial by itself: a store method of the `record_*` family, or a record built by hand
BOUNDARY_RECORDS = re.compile(r"\brecord_\w*\s*\(|\bAuditRecord\b")
BOUNDARY_WORD = re.compile(r"(?<![\w])cross_tenant(?![\w])")
BOUNDARY_FUNCTION_CALL = re.compile(r"\b(deny_unresolved|deny_unresolved_among|deny_unless_own)\s*\(")
BOUNDARY_NAMES = re.compile(r"\b(CrossTenantAccessDenied|ResourceNotFoundDenied)\b")
BOUNDARY_TEXT = "access.denied.cross_tenant"


def tenant_boundary_errors(root):
    """The errors of the rule that keeps one way of auditing a lookup across the tenant boundary (D60, G041-028)."""
    errors = []
    base = pathlib.Path(root)
    if (base / BOUNDARY_DOMAIN).exists() and not (base / BOUNDARY_HOME).exists():
        errors.append(f"{BOUNDARY_HOME} does not exist: the tenant-boundary rule would check nothing (D60)")
    for rel, text, masked, literals in production_sources(root):
        spans = []
        for m in BOUNDARY_FUNCTION_CALL.finditer(masked):
            if re.search(r"\bfn\s+$", masked[max(0, m.start() - 16):m.start()]):
                continue  # the declaration
            end = balanced_end(masked, m.end() - 1)
            if end is not None:
                spans.append((m.end() - 1, end))
                if BOUNDARY_RECORDS.search(masked[m.end() - 1:end]):
                    errors.append(
                        f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: an argument of `{m.group(1)}` records something "
                        f"itself: the shared function is the one that records a lookup across the tenant boundary (D60)"
                    )
        for m in BOUNDARY_CHECK_CALL.finditer(masked):
            if rel.startswith("crates/domain/"):
                break  # the domain reads no store, so a name of its own (`ended_from_outside`) is no check
            if re.search(r"\bfn\s+$", masked[max(0, m.start() - 16):m.start()]):
                continue  # the declaration
            inside = enclosing_function(masked, m.start())
            if (
                inside is not None
                and re.fullmatch(BOUNDARY_CHECK_NAME, inside[0])
                and not BOUNDARY_RECORDS.search(masked[inside[1]:inside[2]])
            ):
                continue  # an implementation of a check, which records nothing
            if any(start < m.start() < end for start, end in spans):
                continue
            errors.append(
                f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: `{m.group(1)}` is called outside `deny_unresolved`, "
                f"`deny_unresolved_among` and `deny_unless_own`: a check for another tenant's resource is handed to the one function that audits it the "
                f"same way for a missing and a foreign identifier (D60, G041-028)"
            )
        if rel == BOUNDARY_DOMAIN:
            continue
        for m in BOUNDARY_NAMES.finditer(masked):
            errors.append(
                f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: `{m.group(1)}` is named outside {BOUNDARY_DOMAIN}: the records "
                f"of a lookup across the tenant boundary are built only from a `BoundaryDenial`, so that the caller's tenant "
                f"gets the same record for a missing and a foreign identifier (D60, G041-028)"
            )
        for offset, content in literals:
            if BOUNDARY_WORD.search(content) and masked[offset - 1] in "\"#":
                errors.append(
                    f"{rel}:{text.count(chr(10), 0, offset) + 1}: the text `cross_tenant` (as in `{BOUNDARY_TEXT}`) is written outside "
                    f"{BOUNDARY_DOMAIN}: the platform's record is built only from a `BoundaryDenial` (D60, G041-028)"
                )
    return errors


# Feature 006, decisions D006-008, D006-092 and D006-093: the delivery probe and the commit probe hold a claim (the commit probe
# inside its open run transaction, with the run's locks) until a test lets it go. They are for the tests only: no production
# source may call them; the tests package and a crate's own test modules may.
TEST_PROBE_CALL = re.compile(r"\.\s*(with_delivery_probe|with_commit_probe)\s*\(")


def test_probe_errors(root):
    """A call of a test-only probe of the worker API in a production source under `crates/*/src` or `apps/*/src`."""
    errors = []
    for rel, text, masked, _ in production_sources(root):
        for m in TEST_PROBE_CALL.finditer(masked):
            errors.append(
                f"{rel}:{text.count(chr(10), 0, m.start()) + 1}: `{m.group(1)}` is called outside the tests: the probe holds "
                f"claims until a test releases them, and production never sets one (D006-093)"
            )
    return errors


def test_support_errors(pkgs):
    """`sagat-application`'s `test-support` feature enabled outside `[dev-dependencies]`, as a dependency or by a feature."""
    errors = []
    for p in pkgs.values():
        for d in p["dependencies"]:
            if d["name"] == APPLICATION_PACKAGE and TEST_SUPPORT_FEATURE in d.get("features", []) and d.get("kind") != "dev":
                errors.append(
                    f"'{p['name']}' enables `{APPLICATION_PACKAGE}/{TEST_SUPPORT_FEATURE}` as a {d.get('kind') or 'normal'} "
                    f"dependency: it builds a `CheckedRunInput` no gate checked, and belongs in `[dev-dependencies]` only (FR-043-016)"
                )
        for feature, enables in (p.get("features") or {}).items():
            for item in enables:
                if item.replace("?", "") == f"{APPLICATION_PACKAGE}/{TEST_SUPPORT_FEATURE}":
                    errors.append(
                        f"'{p['name']}' feature `{feature}` enables `{APPLICATION_PACKAGE}/{TEST_SUPPORT_FEATURE}`: it belongs in "
                        f"`[dev-dependencies]` only (FR-043-016)"
                    )
    return errors


# Feature 006, decision D006-033: the three checks of the instance-capability registry list the capabilities by hand, and two
# branches (feature 022 and feature 006) redefine them with only their own values each. A fresh database ends with the newest
# definition by version, so that one must list exactly what the code lists (`Capability::as_str` over `Capability::ALL`):
# whichever branch merges second takes the union in its own migration, or this fails.
CAPABILITY_SOURCE = "crates/application/src/instances.rs"
CAPABILITY_MIGRATIONS = "crates/infra-postgres/migrations"
CAPABILITY_CHECKS = (
    "serving_instances_capabilities_known",
    "platform_requirements_capability_known",
    "platform_requirement_history_capability_known",
)


def code_capabilities(root):
    """The tokens of `Capability::as_str`, a constant resolved in the application crate; None when the body is not found."""
    base = pathlib.Path(root)
    text = (base / CAPABILITY_SOURCE).read_text(encoding="utf-8")
    impl = text.find("impl Capability {")
    if impl < 0:
        return None
    body = text.find("fn as_str(", impl)
    end = text.find("\n    }\n", body)
    if body < 0 or end < 0:
        return None
    tokens = set()
    for arm in re.finditer(r"Self::\w+\s*=>\s*([^,\n]+),", text[body:end]):
        value = arm.group(1).strip()
        if value.startswith('"'):
            tokens.add(value.strip('"'))
            continue
        name = value.split("::")[-1]
        found = None
        for source in sorted((base / "crates/application/src").rglob("*.rs")):
            match = re.search(rf'pub const {re.escape(name)}: &str = "([^"]+)";', source.read_text(encoding="utf-8"))
            if match:
                found = match.group(1)
                break
        if found is None:
            return None
        tokens.add(found)
    return tokens


def defined_capabilities(text, check):
    """The quoted values of the last `CONSTRAINT <check> CHECK (...)` of a migration (inline or after `ADD`), or None."""
    listed = None
    for found in re.finditer(rf"CONSTRAINT {check} CHECK \(", text):
        depth, i = 1, found.end()
        while i < len(text) and depth:
            depth += {"(": 1, ")": -1}.get(text[i], 0)
            i += 1
        listed = set(re.findall(r"'([^']+)'", text[found.end():i]))
    return listed


def capability_list_errors(root):
    errors = []
    code = code_capabilities(root)
    if not code:
        return [f"`Capability::as_str` was not found in {CAPABILITY_SOURCE}: the capability-list rule would check nothing"]
    folder = pathlib.Path(root) / CAPABILITY_MIGRATIONS
    migrations = sorted(folder.glob("*.up.sql")) if folder.is_dir() else []
    for check in CAPABILITY_CHECKS:
        newest = None
        for migration in migrations:
            listed = defined_capabilities(migration.read_text(encoding="utf-8"), check)
            if listed is None:
                continue
            # decision D006-083: the down of this migration restores the list in force before it (none for the first)
            down = migration.with_name(migration.name.replace(".up.sql", ".down.sql"))
            restored = defined_capabilities(down.read_text(encoding="utf-8"), check) if down.is_file() else None
            before = newest[1] if newest else None
            if restored != before:
                errors.append(
                    f"{down.name} restores {sorted(restored) if restored is not None else 'no list'} of {check}, but the "
                    f"list before {migration.name} is {sorted(before) if before is not None else 'none'}: a rollback "
                    "must restore the list in force before its migration (decision D006-083)"
                )
            newest = (migration.name, listed)
        if newest is None:
            errors.append(f"no migration under {CAPABILITY_MIGRATIONS} defines {check}: the capability-list rule would check nothing")
            continue
        name, listed = newest
        missing, unknown = sorted(code - listed), sorted(listed - code)
        if missing or unknown:
            errors.append(
                f"{check}, last defined by {name}, does not list what `Capability::ALL` lists"
                + (f"; it drops {', '.join(missing)}" if missing else "")
                + (f"; it lists {', '.join(unknown)}, which the code does not know" if unknown else "")
                + ": the branch that merges second takes the union of both lists in its own, newest migration (decision D006-033)"
            )
    return errors


def main(argv):
    metadata_file = argv[argv.index("--metadata") + 1] if "--metadata" in argv else None
    postgres_src = (
        argv[argv.index("--postgres-src") + 1]
        if "--postgres-src" in argv
        else str(pathlib.Path(__file__).resolve().parents[1] / "crates" / "infra-postgres" / "src")
    )
    run_input_src = (
        argv[argv.index("--run-input-src") + 1]
        if "--run-input-src" in argv
        else str(pathlib.Path(__file__).resolve().parents[1] / "crates" / "infra-schema" / "src" / "run_input")
    )
    rust_root = (
        argv[argv.index("--rust-root") + 1]
        if "--rust-root" in argv
        else str(pathlib.Path(__file__).resolve().parents[1])
    )
    capability_root = (
        argv[argv.index("--capability-root") + 1]
        if "--capability-root" in argv
        else str(pathlib.Path(__file__).resolve().parents[1])
    )
    if "--capability-root" in argv:
        # the capability-list rule alone, over a tree a test built
        errors = capability_list_errors(capability_root)
        for e in errors:
            print(f"  - {e}")
        return 1 if errors else 0
    md = load(metadata_file)
    root = md["workspace_root"]
    ws_ids = set(md["workspace_members"])
    pkgs = {p["id"]: p for p in md["packages"] if p["id"] in ws_ids}
    by_name = {p["name"]: p for p in pkgs.values()}
    errors = []
    if metadata_file is None:
        errors.extend(capability_list_errors(capability_root))

    graph = {}
    for p in pkgs.values():
        deps = [d for d in p["dependencies"] if d.get("kind") in (None, "normal")]
        graph[p["name"]] = sorted({d["name"] for d in deps})

    # The Iceberg crates are the independent reader of feature 035 (D035-026): test-only. Writes run in the
    # Python lake runtime (ADR-0071), so no shipped package may link them, whatever the dependency kind.
    for p in pkgs.values():
        if p["name"] in ICEBERG_TEST_ONLY_PACKAGES:
            continue
        for d in sorted({d["name"] for d in p["dependencies"]}):
            if d == ICEBERG_CRATE or d.startswith(ICEBERG_CRATE + "-"):
                errors.append(
                    f"'{p['name']}' must not depend on '{d}': the Iceberg crates are test-only "
                    f"(the independent reader lives in the tests package; writes use the Python lake runtime, ADR-0071)"
                )

    # What the lake runner links, directly or through any crate it links: the way in is named, so the edge to cut can be seen. Normal and
    # build dependencies count (they end up in what ships or in how it is built); a dev dependency does not (nothing that ships links it).
    build_deps = {
        p["name"]: sorted({d["name"] for d in p["dependencies"] if d.get("kind") == "build"})
        for p in pkgs.values()
    }
    if "sagat-application" in graph:
        for missing in sorted(LAKE_RUNNER_PACKAGES - graph.keys()):
            errors.append(
                f"'{missing}' is named by ADR-0079 and the workspace has no such package: the rule would check nothing "
                f"(rename it in LAKE_RUNNER_PACKAGES with the package)"
            )
    for holder in sorted(LAKE_RUNNER_PACKAGES & graph.keys()):
        seen = {holder: [holder]}
        queue = [holder]
        while queue:
            current = queue.pop(0)
            for dep in graph.get(current, []) + (build_deps.get(current, []) if current == holder else []):
                if dep not in seen:
                    seen[dep] = seen[current] + [dep]
                    if dep in graph:
                        queue.append(dep)
        for engine in sorted(CONTAINER_ENGINE_CRATES & seen.keys()):
            errors.append(
                f"'{holder}' must not reach '{engine}' ({' -> '.join(seen[engine])}): the lake runner holds no container-engine access, "
                f"it asks the sandbox runner for an attempt (ADR-0079, rule 2)"
            )

    for name, deps in graph.items():
        layer = layer_of(by_name[name]["manifest_path"], root)
        if layer == "domain":
            for d in deps:
                if d in DOMAIN_FORBIDDEN:
                    errors.append(f"domain crate '{name}' must not depend on '{d}' (forbidden infrastructure/format crate)")
                elif d not in DOMAIN_ALLOWED:
                    errors.append(f"domain crate '{name}' depends on '{d}', which is not in the domain allowlist {sorted(DOMAIN_ALLOWED)}")
        for d in deps:
            if d in by_name:
                dl = layer_of(by_name[d]["manifest_path"], root)
                if dl not in ALLOWED_LAYER_DEPS[layer]:
                    errors.append(f"layer violation: '{name}' ({layer}) must not depend on '{d}' ({dl})")

    errors.extend(lease_row_errors(postgres_src))
    errors.extend(run_finished_errors(postgres_src))
    errors.extend(run_input_errors(run_input_src))
    errors.extend(application_runtime_errors(pkgs))
    errors.extend(run_creator_errors(rust_root))
    errors.extend(run_input_runtime_errors(rust_root))
    errors.extend(gate_wiring_errors(rust_root))
    errors.extend(test_support_errors(pkgs))
    errors.extend(test_probe_errors(rust_root))
    errors.extend(journal_delete_grant_errors(rust_root))
    errors.extend(tenant_boundary_errors(rust_root))

    # Cycle detection over workspace members (DFS, three colours).
    WHITE, GREY, BLACK = 0, 1, 2
    colour = {n: WHITE for n in graph}
    stack = []

    def visit(n):
        colour[n] = GREY
        stack.append(n)
        for d in graph[n]:
            if d not in graph:
                continue
            if colour[d] == GREY:
                cycle = stack[stack.index(d):] + [d]
                errors.append("dependency cycle: " + " -> ".join(cycle))
            elif colour[d] == WHITE:
                visit(d)
        stack.pop()
        colour[n] = BLACK

    for n in graph:
        if colour[n] == WHITE:
            visit(n)

    if errors:
        print("architecture check FAILED:")
        for e in errors:
            print(f"  - {e}")
        return 1
    print(f"architecture check passed ({len(graph)} workspace crates, layers respected, no cycles, the lease row is a leaf lock, every run.finished body says its schedule, "
          f"no jsonschema in the run-input validator, the application's runtime confined to one file, every creator given the gate, every run created with a checked input, "
          f"the capability checks list the code's capabilities, no test probe in production, "
          f"the runtime role cannot delete the journal, one way of auditing a lookup across the tenant boundary)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
