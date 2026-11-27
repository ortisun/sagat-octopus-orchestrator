#!/usr/bin/env python3
"""Decide what the CI runs: the tests of what a change touches, or the whole stack for a release.

Usage: python3 scripts/select-test-targets.py [--event E] [--ref-name N] [--ref-type T] [--base-ref B]
                                              --base SHA [--head REV] [--root DIR]

The rule (docs/constitution.md, CI gates; docs/test-strategy.md):

  * On a pull request the CI runs only the unit tests of the changed packages and of the packages that depend on them,
    the unit tests of the SDK, the Studio and the scripts whose files changed, and the integration suites (the
    `sagat-tests` targets) that were themselves changed. Lints and the other verification jobs do not run.
  * For a release the CI runs everything: every test, every lint, every verification job.

A release is a push to main, release/** or hotfix/**, a push of a tag v*, a pull request into one of those
branches, a manual run (workflow_dispatch), a change of .github/workflows (the workflow is then checked by what
it runs), and any run with no usable base commit (the safe default).

Scoped selection:
  unit tests    the workspace packages that contain a changed file and every package that depends on one of them,
                directly or not (a break in a core package shows in the tests of what is built on it). A change of
                Cargo.toml, Cargo.lock, rust-toolchain.toml, .cargo/, clippy.toml, rustfmt.toml or vendor/ selects every
                package.
  integration   a `sagat-tests` target runs only when it was itself changed: (a) a file of its directory, or its
                [[test]] entry, changed; or (b) a harness it drives changed (frontend/e2e, sdks/*/conformance, examples/,
                runners/lake/ for the lake suites); or (c) for the budget of the hot path, a file of
                crates/infra-postgres/ or of the claim and command path of crates/application changed
                (OD-2026-11-12-4); or (d) for a contract suite (tests/contract/), a contract file under
                specs/<feature>/contracts/ that the suite reads changed (constitution 4.6.0). Nothing else selects a
                suite: not a changed package that the suite uses or that depends on it, and not a shared input
                (tests/support, tests/fixtures, runners/ other than runners/lake/, a dependency of tests/Cargo.toml,
                Cargo.lock): the integration suites are expensive, and the release runs all of them.
  contracts     a changed file under specs/<feature>/contracts/ (Markdown included) selects the contract suites and the
                workspace packages whose Rust sources read it, found from their string literals (Readers), and
                nothing else; a package selected this way also selects its dependents, as a changed package does.
  SDKs, Studio  an SDK's unit tests run when a file under sdks/<language>/ changed; the Studio's when a file under
                frontend/ changed; the scripts' when a file under scripts/ changed; the lake runtime's (Python,
                its lock and its image) when a file other than Markdown under runners/lake/ changed.
  nothing       Markdown, docs/, tasks/, .claude/, infra/ and the rest select no test.

Shards. The selected suites do not run one after another in one CI step (with every suite selected, that step outgrew
its 110 minutes: run 44793165198). They are spread over parallel jobs, each of which builds `sagat-tests` once and runs
its own targets. The plan is greedy, longest suite first, into the least-loaded shard, by the measured duration of each
suite on the CI runner (scripts/integration-suite-durations.toml; a suite missing from it counts as
DEFAULT_SUITE_SECONDS). It starts with as many shards as the total needs at SHARD_BUDGET_SECONDS each and adds one while
a shard overflows the budget, up to MAX_SHARDS. Which suites run is decided above and is not changed by the plan.

It prints the decision and appends it to $GITHUB_OUTPUT:

  full=true|false   crates=<package names|ALL>   sdks=<languages>   frontend=true|false   scripts=true|false
  lake=true|false
  targets=<target names>   any=true|false   all=true|false
  shards=<JSON list of {"index", "count", "targets", "seconds"}, empty when no suite is selected>

Exit codes: 0 decision printed; 2 a manifest, a name or the duration table cannot be trusted.
"""
import argparse
import json
import math
import os
import re
import subprocess
import sys
import tomllib

TARGET_NAME = re.compile(r"[a-z][a-z0-9_]*")
RELEASE_BRANCH = re.compile(r"main|release/.+|hotfix/.+")
RELEASE_TAG = re.compile(r"v[0-9].*")
SHA = re.compile(r"[0-9a-f]{40}")
MANIFEST = "tests/Cargo.toml"
TESTS_PACKAGE = "sagat-tests"
SDK_LANGUAGES = ("typescript", "python", "go", "dotnet", "java", "rust")
BROWSER_SUITES = ("e2e_workflow_studio", "e2e_developer_experience", "e2e_database_connections")
SDK_SUITES = ("sdk_conformance", "e2e_sdk_multilanguage", "fault_sdk_multilanguage")
BUDGET_SUITES = ("integration_claim_transition_efficiency",)
LAKE_SUITES = ("integration_iceberg_writes", "fault_iceberg_writes", "e2e_iceberg_writes", "perf_iceberg_writes")
STEP_LOG_SUITES = ("integration_step_logs",)
STEP_LOG_SERVER_SUITES = STEP_LOG_SUITES + BUDGET_SUITES + ("contract_platform_foundation",)
ACKNOWLEDGEMENT_SUITES = ("integration_durable_execution", "contract_grpc_durable_execution")
HARNESSES = (
    (re.compile(r"frontend/e2e/"), BROWSER_SUITES),
    (re.compile(r"sdks/[a-z]+/conformance/"), SDK_SUITES),
    (re.compile(r"examples/"), ("e2e_developer_experience",)),
    # The lake runtime is driven by these suites; the contract suite reads the contract fragments, not the runtime.
    (re.compile(r"runners/lake/"), LAKE_SUITES),
    # The budget of the hot path (OD-2026-11-12-4, design 038): what a lease and a completion cost the database is a
    # property of the code that runs them, not of the suite that measures it, so the code selects the suite. Every
    # statement and trigger of the adapter can add to it, and so can the claim and the command path that drive it.
    (re.compile(r"crates/infra-postgres/|crates/application/src/(worker|execution)(\.rs$|/)|crates/application/src/runs\.rs$"),
     BUDGET_SUITES),
    # The step logs of design 041 (constitution 4.5.0, OD-2026-11-15-10; task T041-32). The server's step-log code and
    # the files that call it select the step-log suite, the isolation and budget suite that holds the quota-lease tests,
    # and the fitness tests of the domain's closed sets.
    (re.compile(r"crates/[a-z-]+/src/(.*/)?step_logs(\.rs$|/)"
                r"|crates/infra-postgres/migrations/[0-9]+_step_log[a-z_]*\.(up|down)\.sql$"
                r"|crates/api-grpc/src/(limits|service)\.rs$|crates/api-http/src/routes/(execution|scripts)\.rs$"
                r"|crates/application/src/scripts\.rs$|crates/infra-postgres/src/maintenance\.rs$"
                r"|crates/scheduler/src/passes\.rs$"),
     STEP_LOG_SERVER_SUITES),
    # The report path that stores a carry (design 041 section 2a): the report's transaction writes the attempt's last
    # records, so a change of it runs the step-log suite as well as the budget it already runs.
    (re.compile(r"crates/application/src/worker(\.rs$|/)|crates/infra-postgres/src/execution(\.rs$|/)"),
     STEP_LOG_SUITES),
    # The delivery acknowledgement of protocol 1.8 (feature 006, Task 006-049): its modules, each of its own so that its path
    # names it, and the binding checks it shares with the run transactions select the suites that hold its tests.
    (re.compile(r"crates/(application/src/worker|infra-postgres/src|api-grpc/src)/acknowledgement\.rs$"
                r"|crates/application/src/worker/binding_checks\.rs$"),
     ACKNOWLEDGEMENT_SUITES),
    # The worker protocol and each SDK's capture code: what a worker sends is checked by the server's suite and by the
    # six-language conformance.
    (re.compile(r"specs/001-platform-foundation/contracts/worker\.proto$"
                r"|sdks/typescript/src/worker/(step-logs|activity|host|grpc)\.ts$"
                r"|sdks/go/worker/(steplogs|activity|task|host|rpc)\.go$|sdks/go/internal/safelog/safelog\.go$"
                r"|sdks/python/sagat_octopus_orchestrator/worker/(_step_logs|host|activity|_grpc)\.py$"
                r"|sdks/dotnet/src/SagatOctopusOrchestrator\.Sdk/Workers/"
                r"(StepLogCapture|WorkerHost|WorkerRpc|ActivityExecutionContext)\.cs$"
                r"|sdks/dotnet/src/SagatOctopusOrchestrator\.Sdk/Internal/Safe\.cs$"
                r"|sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/"
                r"(StepLogCapture|ActivityLogger|WorkerHost|WorkerRpc)\.java$"
                r"|sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/Hooks\.java$"
                r"|sdks/rust/src/(step_logs|activity|worker|worker_rpc|capabilities)\.rs$"),
     STEP_LOG_SUITES + ("sdk_conformance",)),
)
# Inputs of every Rust package: a change selects the unit tests of every package, and no suite.
RUST_ROOT_INPUTS = re.compile(r"Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|clippy\.toml|rustfmt\.toml|\.cargo/.*|vendor/.*")
# Inputs of the integration suites that cannot be tied to one of them. They select no suite on their own: a pull
# request runs the suites that were changed, and the release runs the rest (OD-2026-11-06-2). The lake runtime is not
# one of them: it is tied to the lake suites (HARNESSES) and to its own job.
SHARED_INPUTS = re.compile(r"runners/(?!lake/).*")
# A contract file. It is not shared like the inputs above: a change of one selects the suites and the packages whose
# Rust sources read it (Readers), so that a pull request that changes only a contract still runs what checks it.
CONTRACT = re.compile(r"(specs/[^/]+/contracts)/(.+)")
# A path into some contracts directory (the file literal of a joined read must not be one: it names its own directory).
CONTRACT_PATH = re.compile(r"specs/[^/]+/contracts/")
# A string literal of Rust source: what a suite reads is named in one (a path, a directory, a file name), and a comment
# that names a contract is not a read.
STRING_LITERAL = re.compile(r'"((?:[^"\\\n]|\\.)*)"')
LAKE_RUNTIME = re.compile(r"runners/lake/.*")
MARKDOWN = re.compile(r".*\.md")
# The measured duration of each suite on the CI runner, by which the shards are balanced.
DURATIONS = "scripts/integration-suite-durations.toml"
# The test time one shard is planned for. It is the longest suite (integration_workflow_primitives, up to 1,796
# seconds): no plan can be shorter than that suite, and a shard more than the time needs costs one more build of
# `sagat-tests` (17 to 29 minutes on the runner) for nothing. A shard is then the build, these 30 minutes and the preflight of its suites, well
# inside the 110 minutes of its step.
SHARD_BUDGET_SECONDS = 1800
# At most this many shards (parallel jobs, each with its own build), whatever the table says.
MAX_SHARDS = 6
# A suite the table does not know yet counts as this long: more than all but the four longest suites measured.
DEFAULT_SUITE_SECONDS = 600
# What a suite with no measured time (a target whose tests are all #[ignore]) still costs: it is built and started. The
# floor also spreads such suites over the shards instead of piling them into the least-loaded one.
MIN_SUITE_SECONDS = 5


class Untrusted(Exception):
    pass


def git(root, *args):
    return subprocess.run(["git", "-C", root, *args], capture_output=True)


def read_toml(path):
    with open(path, "rb") as handle:
        return tomllib.load(handle)


class Workspace:
    """The packages of the root workspace: where they live, what they are called in code and who depends on whom."""

    def __init__(self, root):
        data = read_toml(os.path.join(root, "Cargo.toml"))
        self.directory, self.ident, self.depends_on = {}, {}, {}
        for member in data["workspace"]["members"]:
            manifest = read_toml(os.path.join(root, member, "Cargo.toml"))
            name = manifest["package"]["name"]
            self.directory[name] = member + "/"
            self.ident[manifest.get("lib", {}).get("name", name.replace("-", "_"))] = name
            self.depends_on[name] = self._dependencies(manifest)
        known = set(self.directory)
        for name in self.depends_on:
            self.depends_on[name] &= known

    @staticmethod
    def _dependencies(manifest):
        found = set()
        keys = ("dependencies", "dev-dependencies", "build-dependencies")
        tables = [manifest.get(key, {}) for key in keys]
        tables += [target.get(key, {}) for target in manifest.get("target", {}).values() for key in keys]
        for table in tables:
            for key, value in table.items():
                found.add(value["package"] if isinstance(value, dict) and "package" in value else key)
        return found

    def owner(self, rel):
        owners = [name for name, directory in self.directory.items() if rel.startswith(directory)]
        return max(owners, key=lambda name: len(self.directory[name])) if owners else None

    def dependents(self, names):
        """These packages and every package that depends on any of them, directly or not."""
        reached = set(names)
        grew = True
        while grew:
            grew = False
            for name, deps in self.depends_on.items():
                if name not in reached and deps & reached:
                    reached.add(name)
                    grew = True
        return reached


def target_directories(targets):
    """{target name: the directory of its main file, relative to the root and ending in /}."""
    return {name: "tests/" + path.rsplit("/", 1)[0] + "/" if "/" in path else "tests/" for name, path in targets.items()}


def head_targets(root):
    """{target name: path relative to tests/} from the manifest in the working tree."""
    try:
        targets = {entry["name"]: entry["path"] for entry in read_toml(os.path.join(root, MANIFEST))["test"]}
    except (OSError, ValueError, KeyError, TypeError):
        raise Untrusted("tests/Cargo.toml cannot be read as a list of [[test]] targets")
    for name, path in targets.items():
        if not (isinstance(name, str) and TARGET_NAME.fullmatch(name) and isinstance(path, str)):
            raise Untrusted("a [[test]] target has a name outside [a-z][a-z0-9_]* or a path that is not text")
    return targets


def parse_manifest(text):
    data = tomllib.loads(text)
    tests = {entry["name"]: entry for entry in data.get("test", [])}
    rest = {key: value for key, value in data.items() if key != "test"}
    rest.get("package", {}).pop("description", None)
    return rest, tests


def manifest_selection(root, base, targets, reasons):
    """Targets whose [[test]] entry is new or changed, or None when the manifests cannot be compared and every
    target must run (the safe default of a run that cannot know what changed)."""
    shown = git(root, "show", f"{base}:{MANIFEST}")
    try:
        if shown.returncode != 0:
            raise ValueError("the base has no manifest")
        _, old_tests = parse_manifest(shown.stdout.decode("utf-8"))
        with open(os.path.join(root, MANIFEST), encoding="utf-8") as handle:
            _, new_tests = parse_manifest(handle.read())
    except (OSError, ValueError, KeyError, TypeError, UnicodeDecodeError):
        reasons.append(("every target", f"{MANIFEST} cannot be compared with the base"))
        return None
    # A dependency or a setting of the crate (all of the manifest that is not a [[test]] entry) is shared by the
    # suites: it selects none of them.
    chosen = {name for name, entry in new_tests.items() if old_tests.get(name) != entry and name in targets}
    for name in sorted(chosen):
        reasons.append((name, f"{MANIFEST} adds or changes its [[test]] entry"))
    return chosen


def rust_sources(root, directory):
    """The .rs files under directory (relative to root), relative to root, in a stable order."""
    found = []
    for current, subdirs, files in os.walk(os.path.join(root, directory)):
        subdirs[:] = sorted(d for d in subdirs if d not in ("target", "node_modules", ".git"))
        found += [os.path.relpath(os.path.join(current, f), root) for f in sorted(files) if f.endswith(".rs")]
    return found


class Readers:
    """Who reads a contract file, derived from the Rust sources of the head, never from a hand list.

    The readers are the contract suites (the `sagat-tests` targets under tests/contract/, the guards of the contracts)
    and the workspace packages; an integration, fault, e2e or performance suite that also reads a contract stays a
    suite that only its own changes select (OD-2026-11-06-2). A reader reads `specs/<feature>/contracts/<file>` when a
    string literal of its sources is a path that ends in `specs/<feature>/contracts/<file>`, or when one is a path that
    ends in the directory `specs/<feature>/contracts` and another is `<file>` or ends in `/<file>` (a helper that joins
    the two). A literal that only mentions a contract inside a sentence is not a path to it, and a comment is not a
    literal. The literals of a suite are those of the files its directory owns (the deepest target directory, as for a
    changed file) and of a module of tests/support that it names (`contract::`, or `mod support` for
    tests/support/mod.rs); those of a package are those of every .rs file under its directory, `build.rs` and
    `include_str!` included. A reader that builds the path some other way is not found:
    `scripts/tests/test_select_test_targets.py` fails when a literal of the real repository is a path into a contracts
    directory that reads nothing, so that such a reader is seen."""

    def __init__(self, root, targets, workspace):
        self.root = root
        directories = target_directories(targets)
        self.literals = {}
        owned = {name: [] for name in targets}
        support = []
        for rel in rust_sources(root, "tests"):
            owners = [name for name, directory in directories.items() if rel.startswith(directory)]
            if owners:
                depth = max(len(directories[name]) for name in owners)
                for name in owners:
                    if len(directories[name]) == depth:
                        owned[name].append(rel)
            elif rel.startswith("tests/support/"):
                support.append(rel)
        texts = {}
        for rel in [r for files in owned.values() for r in files] + support:
            texts[rel] = self._read(rel)
        modules = {}
        for rel in support:
            stem = os.path.splitext(os.path.basename(rel))[0]
            name = os.path.basename(os.path.dirname(rel)) if stem == "mod" else stem
            pattern = re.compile(rf"\bmod\s+{name}\b" if stem == "mod" else rf"\b{name}::")
            modules[rel] = pattern
        for name, files in owned.items():
            if not targets[name].startswith("contract/"):
                continue
            text = "\n".join(texts[rel] for rel in files)
            literals = set(STRING_LITERAL.findall(text))
            for rel, pattern in modules.items():
                if pattern.search(text):
                    literals |= set(STRING_LITERAL.findall(texts[rel]))
            self.literals[("target", name)] = literals
        for package, directory in workspace.directory.items():
            if package == TESTS_PACKAGE:
                continue
            text = "\n".join(self._read(rel) for rel in rust_sources(root, directory))
            self.literals[("package", package)] = set(STRING_LITERAL.findall(text))

    def _read(self, rel):
        try:
            with open(os.path.join(self.root, rel), encoding="utf-8", errors="replace") as handle:
                return handle.read()
        except OSError:
            return ""

    def of(self, rel):
        """The (kind, name) readers of the contract file rel; empty when rel is not a contract file."""
        match = CONTRACT.fullmatch(rel)
        if not match:
            return set()
        directory, name = match.groups()
        found = set()
        for reader, literals in self.literals.items():
            whole = any(literal.endswith(f"{directory}/{name}") for literal in literals)
            joined = any(literal.rstrip("/").endswith(directory) for literal in literals) and any(
                literal == name or (literal.endswith("/" + name) and not CONTRACT_PATH.search(literal))
                for literal in literals)
            if whole or joined:
                found.add(reader)
        return found


def load_durations(root):
    """{suite: seconds} from the duration table of the head; empty when the head has no table (every suite then counts
    as the default)."""
    path = os.path.join(root, DURATIONS)
    if not os.path.exists(path):
        return {}
    try:
        seconds = read_toml(path)["seconds"]
        if not isinstance(seconds, dict):
            raise TypeError("[seconds] is not a table")
        for name, value in seconds.items():
            if not TARGET_NAME.fullmatch(name) or isinstance(value, bool) or not isinstance(value, int) or value < 0:
                raise ValueError(f"{name} = {value!r}")
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise Untrusted(f"{DURATIONS} cannot be read as [seconds] <suite> = <whole seconds>: {error}")
    return seconds


def plan_shards(names, durations):
    """The selected suites spread over shards: greedy, longest first, into the least-loaded shard (ties to the lower
    index). Deterministic: the same names and table give the same plan in any order."""
    weight = {name: max(durations.get(name, DEFAULT_SUITE_SECONDS), MIN_SUITE_SECONDS) for name in set(names)}
    if not weight:
        return []
    order = sorted(weight, key=lambda name: (-weight[name], name))
    count = min(MAX_SHARDS, max(1, math.ceil(sum(weight.values()) / SHARD_BUDGET_SECONDS)))
    while True:
        loads, members = [0] * count, [[] for _ in range(count)]
        for name in order:
            least = min(range(count), key=lambda i: (loads[i], i))
            loads[least] += weight[name]
            members[least].append(name)
        # A shard over the budget that is not a single long suite: one shard more may bring it under.
        if count >= MAX_SHARDS or all(load <= SHARD_BUDGET_SECONDS or len(m) == 1 for load, m in zip(loads, members)):
            break
        count += 1
    return [{"index": i + 1, "count": count, "targets": " ".join(sorted(members[i])), "seconds": loads[i]}
            for i in range(count)]


class Decision:
    def __init__(self, targets):
        self.targets = targets
        self.full = False
        self.crates, self.sdks, self.frontend, self.scripts, self.lake = set(), set(), False, False, False
        self.chosen, self.everything, self.reasons = set(), False, []

    def run_everything(self, why):
        self.full = True
        self.everything = True
        self.chosen = set(self.targets)
        self.reasons.append(("every target", why))


def changed_files(root, base, head):
    diff = git(root, "diff", "--name-only", "-z", "--no-renames", base, head)
    if diff.returncode != 0:
        return None
    return [name for name in diff.stdout.decode("utf-8", errors="replace").split("\0") if name]


def scope(root, base, head, targets, workspace):
    """The Decision for the files that differ between base and head."""
    decision = Decision(targets)
    files = changed_files(root, base, head)
    if files is None:
        decision.run_everything("the diff against the base could not be read")
        return decision
    if any(rel.startswith(".github/workflows/") for rel in files):
        decision.run_everything("the workflow itself changed, so it is checked by running all of it")
        return decision
    directories = target_directories(targets)
    changed_packages = set()
    readers = None

    def harnesses(rel):
        for pattern, names in HARNESSES:
            if pattern.match(rel):
                for name in names:
                    if name in targets:
                        decision.chosen.add(name)
                        decision.reasons.append((name, rel))

    for rel in files:
        if CONTRACT.fullmatch(rel):
            # A contract is read, not owned: it selects what reads it, Markdown included (an error table, a README the
            # suite parses), and nothing else. A contract nobody reads selects nothing; the release still checks it.
            readers = readers or Readers(root, targets, workspace)
            found = readers.of(rel)
            if not found:
                decision.reasons.append(("no suite", f"{rel} is a contract that no suite or package reads"))
            for kind, name in sorted(found):
                if kind == "target":
                    decision.chosen.add(name)
                    decision.reasons.append((name, f"{rel} is a contract it reads"))
                else:
                    changed_packages.add(name)
                    decision.reasons.append((name, f"{rel} is a contract the package reads"))
            # A harness may also be tied to a contract (the worker protocol of design 041, task T041-32).
            harnesses(rel)
            continue
        if MARKDOWN.fullmatch(rel):
            continue
        if RUST_ROOT_INPUTS.fullmatch(rel):
            decision.everything = True
            decision.reasons.append(("every package", f"{rel} is an input of every Rust package"))
            continue
        if SHARED_INPUTS.fullmatch(rel):
            # Said in the log so that nobody wonders why no suite ran; the file still goes through the rest, since a
            # runner may be a workspace package whose own unit tests (and its dependents') are due.
            decision.reasons.append(("no suite", f"{rel} is shared by the suites; the release runs them"))
        for language in SDK_LANGUAGES:
            if rel.startswith(f"sdks/{language}/"):
                decision.sdks.add(language)
        if rel.startswith("frontend/"):
            decision.frontend = True
        if rel.startswith("scripts/"):
            decision.scripts = True
        if LAKE_RUNTIME.fullmatch(rel):
            decision.lake = True
        harnesses(rel)
        if rel == MANIFEST:
            found = manifest_selection(root, base, targets, decision.reasons)
            if found is None:
                decision.chosen = set(targets)
            else:
                decision.chosen |= found
        elif rel.startswith("tests/"):
            owners = [name for name, directory in directories.items() if rel.startswith(directory)]
            if not owners:
                decision.reasons.append(("no suite", f"{rel} is shared by the suites; the release runs them"))
                continue
            depth = max(len(directories[name]) for name in owners)
            for name in sorted(n for n in owners if len(directories[n]) == depth):
                decision.chosen.add(name)
                decision.reasons.append((name, rel))
        else:
            owner = workspace.owner(rel)
            if owner and owner != TESTS_PACKAGE:
                changed_packages.add(owner)
    if decision.everything:
        decision.crates = {name for name in workspace.directory if name != TESTS_PACKAGE}
    else:
        # The changed packages and everything built on them. The integration crate is never one of them: its suites
        # are chosen by their own files.
        decision.crates = workspace.dependents(changed_packages) - {TESTS_PACKAGE}
    return decision


def full_run_reason(event, ref_name, ref_type, base_ref, base, root):
    if event == "workflow_dispatch":
        return "a manual run"
    if event == "push" and ref_type == "tag" and RELEASE_TAG.fullmatch(ref_name):
        return f"the release tag {ref_name}"
    if event == "push" and ref_type != "tag" and RELEASE_BRANCH.fullmatch(ref_name):
        return f"a push to {ref_name}"
    if event == "pull_request" and RELEASE_BRANCH.fullmatch(base_ref):
        return f"a pull request into {base_ref}"
    if not SHA.fullmatch(base) or set(base) == {"0"}:
        return "no usable base commit"
    if git(root, "cat-file", "-e", f"{base}^{{commit}}").returncode != 0:
        return "the base commit is not in this clone"
    return None


def emit(decision, durations):
    if decision.full:
        crates, sdks = "ALL", list(SDK_LANGUAGES)
        frontend = scripts = lake = True
    else:
        crates = "ALL" if decision.everything else " ".join(sorted(decision.crates))
        sdks, frontend, scripts = sorted(decision.sdks), decision.frontend, decision.scripts
        lake = decision.lake
    shards = plan_shards(decision.chosen, durations)
    unknown = sorted(name for name in decision.chosen if name not in durations)
    if unknown:
        print(f"::warning::{DURATIONS} has no duration for {' '.join(unknown)}; planned at {DEFAULT_SUITE_SECONDS} seconds each")
    grouped = {}
    for name, why in decision.reasons:
        grouped.setdefault(name, []).append(why)
    for name, whys in grouped.items():
        more = f" (+{len(whys) - 1} more)" if len(whys) > 1 else ""
        print(f"select {name}: {whys[0]}{more}")

    def flag(value):
        return "true" if value else "false"

    lines = [
        f"full={flag(decision.full)}",
        f"crates={crates}",
        f"sdks={' '.join(sdks)}",
        f"frontend={flag(frontend)}",
        f"scripts={flag(scripts)}",
        f"lake={flag(lake)}",
        f"targets={' '.join(sorted(decision.chosen))}",
        f"any={flag(decision.chosen)}",
        f"all={flag(decision.everything)}",
        f"shards={json.dumps(shards)}",
    ]
    for shard in shards:
        print(f"shard {shard['index']} of {shard['count']} (about {shard['seconds']} seconds): {shard['targets']}")
    for line in lines:
        print(line)
    path = os.environ.get("GITHUB_OUTPUT")
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")


def main(argv):
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--root", default=os.getcwd())
    parser.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", ""))
    parser.add_argument("--ref-name", default=os.environ.get("GITHUB_REF_NAME", ""))
    parser.add_argument("--ref-type", default=os.environ.get("GITHUB_REF_TYPE", "branch"))
    parser.add_argument("--base-ref", default=os.environ.get("GITHUB_BASE_REF", ""))
    parser.add_argument("--base", default="")
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args(argv[1:])
    try:
        targets = head_targets(args.root)
        durations = load_durations(args.root)
        workspace = Workspace(args.root)
    except (Untrusted, OSError, KeyError, ValueError, TypeError) as error:
        message = error if isinstance(error, Untrusted) else "the workspace cannot be read"
        print(f"select-test-targets: {message}", file=sys.stderr)
        return 2
    reason = full_run_reason(args.event, args.ref_name, args.ref_type, args.base_ref, args.base, args.root)
    if reason:
        decision = Decision(targets)
        decision.run_everything(reason)
    else:
        decision = scope(args.root, args.base, args.head, targets, workspace)
    emit(decision, durations)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
