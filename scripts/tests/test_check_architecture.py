"""Regression tests for scripts/check-architecture.py (run: python3 -m unittest discover scripts/tests)."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check-architecture.py"


def pkg(name, path, deps):
    """A dependency is a name (a normal dependency) or a (name, kind) pair, kind being "dev" or "build"."""
    return {
        "id": name,
        "name": name,
        "manifest_path": f"/r/{path}/Cargo.toml",
        "dependencies": [
            {"name": d, "kind": None} if isinstance(d, str) else {"name": d[0], "kind": d[1]}
            for d in deps
        ],
    }


def run_check(packages):
    metadata = {
        "workspace_root": "/r",
        "workspace_members": [p["id"] for p in packages],
        "packages": packages,
    }
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(metadata, f)
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--metadata", f.name], capture_output=True, text=True
    )
    pathlib.Path(f.name).unlink()
    return result.returncode, result.stdout


class ArchitectureRules(unittest.TestCase):
    def test_valid_layering_passes(self):
        code, out = run_check([
            pkg("d", "crates/domain", ["uuid", "time", "thiserror"]),
            pkg("a", "crates/application", ["d"]),
            pkg("p", "crates/infra-postgres", ["a", "d"]),
            pkg("bin", "apps/control-plane", ["a", "p", "d"]),
        ])
        self.assertEqual(code, 0, out)

    def test_domain_must_not_depend_on_infrastructure_crates(self):
        for forbidden in ("sqlx", "axum", "async-nats", "redis", "kube", "serde_json"):
            with self.subTest(forbidden=forbidden):
                code, out = run_check([pkg("d", "crates/domain", ["uuid", forbidden])])
                self.assertEqual(code, 1)
                self.assertIn(f"must not depend on '{forbidden}'", out)

    def test_domain_rejects_unlisted_crates(self):
        code, out = run_check([pkg("d", "crates/domain", ["left-pad"])])
        self.assertEqual(code, 1)
        self.assertIn("not in the domain allowlist", out)

    def test_application_must_not_depend_on_adapters(self):
        code, out = run_check([
            pkg("d", "crates/domain", []),
            pkg("a", "crates/application", ["d", "p"]),
            pkg("p", "crates/infra-postgres", ["a"]),
        ])
        self.assertEqual(code, 1)
        self.assertIn("layer violation", out)

    def test_the_scheduler_is_an_application_service_not_an_adapter(self):
        # It may use the domain and the ports...
        code, out = run_check([
            pkg("d", "crates/domain", []),
            pkg("a", "crates/application", ["d"]),
            pkg("s", "crates/scheduler", ["a", "d"]),
            pkg("bin", "apps/execution-engine", ["s", "a", "d", "p"]),
            pkg("p", "crates/infra-postgres", ["a", "d"]),
        ])
        self.assertEqual(code, 0, out)

    def test_the_scheduler_must_not_know_the_adapters(self):
        code, out = run_check([
            pkg("d", "crates/domain", []),
            pkg("a", "crates/application", ["d"]),
            pkg("p", "crates/infra-postgres", ["a", "d"]),
            pkg("s", "crates/scheduler", ["a", "d", "p"]),
        ])
        self.assertEqual(code, 1)
        self.assertIn("layer violation", out)
        self.assertIn("'s' (application) must not depend on 'p' (adapter)", out)

    def test_adapters_may_not_be_depended_on_by_the_application_layer_through_the_scheduler(self):
        code, out = run_check([
            pkg("d", "crates/domain", []),
            pkg("a", "crates/application", ["d", "s"]),
            pkg("s", "crates/scheduler", ["d"]),
        ])
        # application -> application is allowed; this documents that it is not an adapter edge.
        self.assertEqual(code, 0, out)

    def test_the_iceberg_crate_is_allowed_as_a_test_only_dependency(self):
        # The independent reader of feature 035 (D035-026) lives in the test package only.
        code, out = run_check([
            pkg("d", "crates/domain", []),
            pkg("sagat-tests", "tests", [("iceberg", "dev"), ("iceberg-catalog-rest", "dev")]),
        ])
        self.assertEqual(code, 0, out)

    def test_the_iceberg_crate_must_not_reach_the_platform(self):
        # Writes go through the Python runtime (ADR-0071); the Rust crate is the independent reader and
        # may not become a normal, dev or build dependency of anything that ships.
        for crate in ("iceberg", "iceberg-catalog-rest"):
            for kind in (None, "dev", "build"):
                for where in ("crates/infra-lake", "apps/control-plane", "crates/application"):
                    with self.subTest(crate=crate, kind=kind, where=where):
                        deps = [(crate, kind)] if kind else [crate]
                        code, out = run_check([pkg("p", where, deps)])
                        self.assertEqual(code, 1, out)
                        self.assertIn(f"'p' must not depend on '{crate}'", out)
                        self.assertIn("test-only", out)

    # ADR-0079 rule 2: the lake runner holds no container-engine access. The engine is `bollard` and the crate that wraps it,
    # `sagat-infra-sandbox`, and the sandbox runner is the only process that links them.
    LAKE_RUNNER = "sagat-database-runner"

    def test_the_lake_runner_may_use_what_is_not_the_engine(self):
        code, out = run_check([
            pkg("sagat-domain", "crates/domain", []),
            pkg("sagat-infra-lake", "crates/infra-lake", ["sagat-domain", "hyper"]),
            pkg(self.LAKE_RUNNER, "apps/database-runner", ["sagat-infra-lake", "sagat-domain", "tonic"]),
            # the process that does hold the engine is another one
            pkg("sagat-infra-sandbox", "crates/infra-sandbox", ["bollard", "sagat-domain"]),
            pkg("sagat-sandbox-runner", "apps/sandbox-runner", ["sagat-infra-sandbox", "sagat-infra-lake"]),
        ])
        self.assertEqual(code, 0, out)

    def test_the_lake_runner_must_not_link_the_engine_directly(self):
        for holder in (self.LAKE_RUNNER, "sagat-infra-lake"):
            for engine in ("bollard", "sagat-infra-sandbox"):
                with self.subTest(holder=holder, engine=engine):
                    where = "apps/database-runner" if holder == self.LAKE_RUNNER else "crates/infra-lake"
                    code, out = run_check([
                        pkg("sagat-infra-sandbox", "crates/infra-sandbox", ["bollard"]),
                        pkg(holder, where, [engine]),
                    ])
                    self.assertEqual(code, 1, out)
                    self.assertIn("ADR-0079", out)
                    self.assertIn(f"'{holder}' must not reach '{engine}'", out)

    def test_the_lake_runner_must_not_reach_the_engine_through_another_crate(self):
        code, out = run_check([
            pkg("sagat-infra-sandbox", "crates/infra-sandbox", ["bollard"]),
            pkg("sagat-handy", "crates/infra-handy", ["sagat-infra-sandbox"]),
            pkg("sagat-middle", "crates/infra-middle", ["sagat-handy"]),
            pkg(self.LAKE_RUNNER, "apps/database-runner", ["sagat-middle"]),
        ])
        self.assertEqual(code, 1, out)
        # the message names the way in, so the dependency to cut can be seen
        self.assertIn(
            f"'{self.LAKE_RUNNER}' must not reach 'sagat-infra-sandbox' ({self.LAKE_RUNNER} -> sagat-middle -> sagat-handy -> sagat-infra-sandbox)",
            out,
        )
        self.assertIn(f"'{self.LAKE_RUNNER}' must not reach 'bollard'", out)

    def test_a_build_dependency_of_the_lake_runner_counts_and_a_dev_dependency_does_not(self):
        code, out = run_check([pkg(self.LAKE_RUNNER, "apps/database-runner", [("bollard", "build")])])
        self.assertEqual(code, 1, out)
        self.assertIn(f"'{self.LAKE_RUNNER}' must not reach 'bollard'", out)
        # a test of the runner may hold an engine double: nothing that ships links it
        code, out = run_check([pkg(self.LAKE_RUNNER, "apps/database-runner", [("bollard", "dev")])])
        self.assertEqual(code, 0, out)

    def test_a_rule_that_names_a_package_the_workspace_no_longer_has_is_an_error_and_not_a_pass(self):
        # A renamed lake runner would otherwise switch the rule off without a sound. A graph that is not the platform's (no `sagat-application`) is
        # not asked: the other tests of this file use made-up crates.
        code, out = run_check([
            pkg("sagat-application", "crates/application", []),
            pkg("sagat-infra-lake", "crates/infra-lake", []),
        ])
        self.assertEqual(code, 1, out)
        self.assertIn("'sagat-database-runner' is named by ADR-0079 and the workspace has no such package", out)

    def test_cycles_are_reported(self):
        code, out = run_check([
            pkg("x", "crates/telemetry", ["y"]),
            pkg("y", "crates/scheduler", ["x"]),
        ])
        self.assertEqual(code, 1)
        self.assertIn("dependency cycle", out)


# The lease row of a scheduler shard is a leaf lock (ADR-0050 decision 7, ADR-0020): a fence takes it FOR SHARE, the keeper FOR UPDATE
# SKIP LOCKED and nothing else, and the keeper reads no run.
KEEPER = """
    async fn acquire() {
        sqlx::query(
            "WITH free AS ( \\
                 SELECT shard FROM scheduler_shards \\
                  WHERE expires_at <= clock_timestamp() ORDER BY shard FOR UPDATE SKIP LOCKED) \\
             UPDATE scheduler_shards s SET owner = $1 FROM free WHERE s.shard = free.shard",
        );
        sqlx::query("SELECT shard FROM scheduler_shards");
    }
"""
FENCE = """
    pub async fn locked_scheduler_time_sql() {
        sqlx::query("SELECT shard FROM scheduler_shards WHERE shard=$1 FOR SHARE");
        sqlx::query("SELECT s.expires_at FROM scheduler_shards s");
    }
"""
CHECK_FENCE = """
    pub async fn check_fence() {
        sqlx::query("SELECT shard FROM scheduler_shards WHERE shard = $1 FOR SHARE");
    }
"""


def run_lease_check(files):
    """The checker over a small PostgreSQL adapter tree made of `files` ({name: source}), with a valid dependency graph."""
    with tempfile.TemporaryDirectory() as tmp:
        src = pathlib.Path(tmp) / "src"
        src.mkdir()
        for name, source in files.items():
            if name.startswith("../"):
                (pathlib.Path(tmp) / name[3:]).parent.mkdir(parents=True, exist_ok=True)
                (pathlib.Path(tmp) / name[3:]).write_text(source)
                continue
            (src / name).parent.mkdir(parents=True, exist_ok=True)
            (src / name).write_text(source)
        metadata = {
            "workspace_root": "/r",
            "workspace_members": ["d"],
            "packages": [pkg("d", "crates/domain", [])],
        }
        meta = pathlib.Path(tmp) / "metadata.json"
        meta.write_text(json.dumps(metadata))
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--metadata", str(meta), "--postgres-src", str(src)],
            capture_output=True,
            text=True,
        )
        return result.returncode, result.stdout


ORIGIN = """
    pub(crate) async fn add_schedule_origin(conn: &mut PgConnection, data: &mut Value) {
        sqlx::query("SELECT 1");
    }
"""
WRITER = """
    async fn outbox_run_finished(conn: &mut PgConnection) {
        let mut data = json!({"runId": "r"});
        add_schedule_origin(&mut *conn, &mut data).await;
        let body = json!({"id": "x", "type": "run.finished", "data": data});
    }
"""


# The claim's marks of protocol 1.8, as the three lease statements write them (Task 006-047): `{a}` is the statement's alias.
CLAIM_MARKS_SQL = """ \\
    lease_hand_back_generation = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND {a}generation IS DISTINCT FROM $8 \\
        THEN CASE WHEN $34 THEN $8 END ELSE {a}lease_hand_back_generation END, \\
    lease_claimed_at = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND {a}generation IS DISTINCT FROM $8 \\
        THEN CASE WHEN $34 THEN clock_timestamp() END ELSE {a}lease_claimed_at END, \\
    lease_claim_incarnation = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND {a}generation IS DISTINCT FROM $8 \\
        THEN CASE WHEN $34 THEN (SELECT pg_postmaster_start_time()) END ELSE {a}lease_claim_incarnation END, \\
    lease_offered_generation = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND {a}generation IS DISTINCT FROM $8 \\
        THEN CASE WHEN $34 AND $35 AND {a}provisional_withdrawn_attempt IS DISTINCT FROM $7 \\
                       AND LEAST($23, COALESCE($24, $23)) >= $36 THEN $8 END \\
        ELSE {a}lease_offered_generation END, \\
    lease_acknowledged_at = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND {a}generation IS DISTINCT FROM $8 \\
        THEN NULL ELSE {a}lease_acknowledged_at END, \\
    lease_capability_generation = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND {a}generation IS DISTINCT FROM $8 \\
        THEN NULL ELSE {a}lease_capability_generation END"""
ATTEMPT_FLAG_SQL = (
    "provisional_withdrawn_attempt = CASE WHEN $40 AND {a}status = 'LEASED' AND {a}generation = $41 "
    "THEN {a}attempt ELSE {a}provisional_withdrawn_attempt END"
)


def lease_statement(name, alias="", flag=ATTEMPT_FLAG_SQL, marks=CLAIM_MARKS_SQL, extra=""):
    """One lease statement as the rule of the marks wants it, with the claim's marks and the attempt's flag."""
    a = f"{alias}." if alias else ""
    table = f"steps AS {alias}" if alias else "steps"
    sets = ", ".join(part.format(a=a) for part in (marks, flag, extra) if part)
    return f"""
    async fn {name}() {{
        sqlx::query("INSERT INTO steps (tenant_id, run_id, status, generation) VALUES ($1, $2, $3, $4)");
        sqlx::query("UPDATE {table} SET status = $6, generation = $8, {sets} \\
             WHERE tenant_id = $1 AND run_id = $2");
    }}
"""


LEASE_STATEMENTS_FILES = {
    "execution.rs": FENCE + lease_statement("upsert_step") + lease_statement("upsert_scoped_instance", "s"),
    "execution/path.rs": lease_statement("upsert_path_instance", "s"),
}


def tree(keeper=KEEPER, execution=None, schedules=CHECK_FENCE, **more):
    """A valid tree: the lease row as the rule wants it, one `run.finished` writer that says its schedule, and the three lease
    statements writing the marks of protocol 1.8 in their allowed forms."""
    files = {
        "partitioning.rs": keeper,
        **LEASE_STATEMENTS_FILES,
        "schedules.rs": schedules,
        "outbound.rs": ORIGIN,
        "writer.rs": WRITER,
    }
    if execution is not None:
        files["execution.rs"] = execution
    files.update(more)
    return files


class LeaseRowIsALeafLock(unittest.TestCase):
    def test_the_adapter_of_this_repository_passes(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("the lease row is a leaf lock", result.stdout)

    def test_fences_shared_and_a_keeper_that_never_waits_pass(self):
        code, out = run_lease_check(tree())
        self.assertEqual(code, 0, out)

    def test_a_fence_that_takes_the_row_for_update_is_refused(self):
        # holds_shard turned into FOR UPDATE: it would wait for every other fence and for the keeper
        code, out = run_lease_check(tree(execution=FENCE.replace("FOR SHARE", "FOR UPDATE")))
        self.assertEqual(code, 1, out)
        self.assertIn("belongs to partitioning.rs only", out)
        self.assertIn("`locked_scheduler_time_sql` (execution.rs) has no `FOR SHARE` statement", out)

    def test_check_fence_that_takes_the_row_for_no_key_update_is_refused(self):
        code, out = run_lease_check(tree(schedules=CHECK_FENCE.replace("FOR SHARE", "FOR NO KEY UPDATE")))
        self.assertEqual(code, 1, out)
        self.assertIn("belongs to partitioning.rs only", out)

    def test_a_keeper_statement_without_skip_locked_is_refused(self):
        code, out = run_lease_check(tree(keeper=KEEPER.replace(" SKIP LOCKED", "")))
        self.assertEqual(code, 1, out)
        self.assertIn("without SKIP LOCKED", out)

    def test_an_update_of_the_row_outside_the_keeper_is_refused(self):
        other = 'fn renew() { sqlx::query("UPDATE scheduler_shards SET expires_at = now() WHERE shard = $1"); }'
        code, out = run_lease_check(tree(**{"somewhere.rs": other}))
        self.assertEqual(code, 1, out)
        self.assertIn("somewhere.rs:1", out)
        self.assertIn("belongs to partitioning.rs only", out)

    def test_a_keeper_that_reads_a_run_is_refused(self):
        code, out = run_lease_check(tree(keeper=KEEPER + ' fn x() { sqlx::query("SELECT 1 FROM runs WHERE id = $1"); }'))
        self.assertEqual(code, 1, out)
        self.assertIn("names `runs`", out)

    def test_comments_and_text_that_are_not_sql_do_not_trip_the_rule(self):
        commented = KEEPER + "\n// a takeover waits: UPDATE scheduler_shards ... FOR UPDATE, the runs it fences\n/* FOR UPDATE scheduler_shards runs */\n"
        code, out = run_lease_check(tree(keeper=commented, **{"other.rs": 'const A: &str = "a run, runs";'}))
        self.assertEqual(code, 0, out)

    def test_a_rule_that_names_a_function_that_is_gone_is_an_error_and_not_a_pass(self):
        code, out = run_lease_check(tree(execution="pub async fn something_else() {}"))
        self.assertEqual(code, 1, out)
        self.assertIn("`locked_scheduler_time_sql` (execution.rs) is named by the lease-row rule and is not there", out)
        code, out = run_lease_check({"execution.rs": FENCE, "schedules.rs": CHECK_FENCE})
        self.assertEqual(code, 1, out)
        self.assertIn("partitioning.rs is named by the lease-row rule", out)


class EveryRunFinishedBodySaysItsSchedule(unittest.TestCase):
    """FR-018-046, D2143: each place that builds the `run.finished` body calls `add_schedule_origin` on its data first."""

    def test_the_adapter_of_this_repository_passes(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("every run.finished body says its schedule", result.stdout)

    def test_a_writer_that_calls_it_before_the_body_passes(self):
        code, out = run_lease_check(tree())
        self.assertEqual(code, 0, out)

    def test_a_writer_that_leaves_the_call_out_is_refused_and_named(self):
        bare = WRITER.replace("        add_schedule_origin(&mut *conn, &mut data).await;\n", "")
        self.assertNotEqual(bare, WRITER)
        code, out = run_lease_check(tree(**{"writer.rs": bare}))
        self.assertEqual(code, 1, out)
        self.assertIn("writer.rs:4", out)
        self.assertIn("a `run.finished` body is built without `add_schedule_origin`", out)

    def test_a_writer_added_later_without_the_call_is_refused_while_the_first_stays_fine(self):
        fifth = 'async fn another(conn: &mut PgConnection) { let body = json!({"type": "run.finished"}); }'
        code, out = run_lease_check(tree(**{"fifth.rs": fifth}))
        self.assertEqual(code, 1, out)
        self.assertIn("fifth.rs:1", out)
        self.assertNotIn("writer.rs", out)

    def test_a_call_after_the_body_does_not_count(self):
        body = '        let body = json!({"id": "x", "type": "run.finished", "data": data});\n'
        call = "        add_schedule_origin(&mut *conn, &mut data).await;\n"
        late = WRITER.replace(call + body, body + call)
        self.assertNotEqual(late, WRITER)
        code, out = run_lease_check(tree(**{"writer.rs": late}))
        self.assertEqual(code, 1, out)
        self.assertIn("without `add_schedule_origin` before it in the same function", out)

    def test_a_call_in_another_function_does_not_count(self):
        split = (
            "async fn origin_of(conn: &mut PgConnection, data: &mut Value) { add_schedule_origin(&mut *conn, data).await; }\n"
            'async fn body(data: Value) { let body = json!({"type": "run.finished", "data": data}); }\n'
        )
        code, out = run_lease_check(tree(**{"writer.rs": split}))
        self.assertEqual(code, 1, out)
        self.assertIn("writer.rs:2", out)

    def test_the_words_in_a_comment_or_a_string_are_not_a_writer(self):
        noise = (
            '// "type": "run.finished" is what the subscribers are sent\n'
            '/* json!({"type": "run.finished"}) */\n'
            'const NOTE: &str = "the body {\\"type\\": \\"run.finished\\"} says its schedule";\n'
        )
        code, out = run_lease_check(tree(**{"noise.rs": noise}))
        self.assertEqual(code, 0, out)

    def test_the_sql_name_of_the_event_is_not_a_body(self):
        insert = "async fn hint() { sqlx::query(\"INSERT INTO event_outbox (event_type) VALUES ('run.finished')\"); }"
        code, out = run_lease_check(tree(**{"hint.rs": insert}))
        self.assertEqual(code, 0, out)

    def test_a_rule_that_finds_no_writer_or_no_function_is_an_error_and_not_a_pass(self):
        code, out = run_lease_check(tree(**{"writer.rs": "async fn nothing() {}"}))
        self.assertEqual(code, 1, out)
        self.assertIn("no `run.finished` body was found", out)
        code, out = run_lease_check(tree(**{"outbound.rs": "async fn nothing() {}"}))
        self.assertEqual(code, 1, out)
        self.assertIn("`add_schedule_origin` is named by the `run.finished` rule and is not defined", out)




def run_input_check(files, packages=None):
    """The checker over a run-input validator tree made of `files` ({name: source}), and a dependency graph of `packages`."""
    with tempfile.TemporaryDirectory() as tmp:
        src = pathlib.Path(tmp) / "run_input"
        src.mkdir()
        for name, source in files.items():
            (src / name).write_text(source)
        packages = packages or [pkg("d", "crates/domain", [])]
        metadata = {
            "workspace_root": "/r",
            "workspace_members": [p["id"] for p in packages],
            "packages": packages,
        }
        meta = pathlib.Path(tmp) / "metadata.json"
        meta.write_text(json.dumps(metadata))
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--metadata", str(meta), "--run-input-src", str(src)],
            capture_output=True,
            text=True,
        )
        return result.returncode, result.stdout


VALIDATOR = "pub fn validate(input: &[u8]) -> bool { let _ = input; true }\n"


class TheRunInputValidatorIsThePlatformsOwn(unittest.TestCase):
    """Design 043, D043-074: `jsonschema` stays out of `crates/infra-schema/src/run_input/`."""

    def test_the_validator_of_this_repository_passes(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("no jsonschema in the run-input validator", result.stdout)

    def test_a_validator_without_it_passes(self):
        code, out = run_input_check({"mod.rs": VALIDATOR})
        self.assertEqual(code, 0, out)

    def test_a_use_of_jsonschema_is_refused_and_named(self):
        code, out = run_input_check({"mod.rs": VALIDATOR, "validate.rs": "use jsonschema::Validator;\nfn f() {}\n"})
        self.assertEqual(code, 1, out)
        self.assertIn("crates/infra-schema/src/run_input/validate.rs:1", out)
        self.assertIn("D043-074", out)

    def test_a_qualified_path_and_an_extern_crate_are_refused(self):
        for source in (
            "fn f(s: &serde_json::Value) -> bool {\n    ::jsonschema::is_valid(s, s)\n}\n",
            "extern crate jsonschema;\n",
            "fn f() { let o = jsonschema :: options(); }\n",
        ):
            with self.subTest(source=source):
                code, out = run_input_check({"compile.rs": source})
                self.assertEqual(code, 1, out)
                self.assertIn("compile.rs", out)

    def test_the_name_in_a_comment_or_a_string_is_not_a_use(self):
        code, out = run_input_check({
            "mod.rs": "//! `jsonschema` is the oracle of a test only.\n/* jsonschema */\nconst A: &str = \"jsonschema\";\n" + VALIDATOR,
        })
        self.assertEqual(code, 0, out)

    def test_a_rule_over_a_directory_with_no_source_is_an_error_and_not_a_pass(self):
        code, out = run_input_check({})
        self.assertEqual(code, 1, out)
        self.assertIn("the rule would check nothing", out)


class TheApplicationHoldsNoRuntime(unittest.TestCase):
    """Design 043, D043-077: `tokio` is at most a dev-dependency of `sagat-application`."""

    def packages(self, tokio):
        """A workspace whose application has `tokio` as `tokio` says: a dependency name or a (name, kind) pair."""
        return [
            pkg("sagat-domain", "crates/domain", []),
            pkg("sagat-application", "crates/application", ["sagat-domain", tokio]),
            # named by the lake-runner rule whenever the application is in the graph
            pkg("sagat-database-runner", "apps/database-runner", ["sagat-application"]),
            pkg("sagat-infra-lake", "crates/infra-lake", ["sagat-application"]),
        ]

    def test_a_dev_dependency_passes(self):
        code, out = run_input_check({"mod.rs": VALIDATOR}, self.packages(("tokio", "dev")))
        self.assertEqual(code, 0, out)

    def test_a_normal_dependency_is_refused(self):
        code, out = run_input_check({"mod.rs": VALIDATOR}, self.packages("tokio"))
        self.assertEqual(code, 1, out)
        self.assertIn("must not have `tokio` as a normal dependency", out)

    def test_rt_alone_without_default_features_passes_and_any_more_is_refused(self):
        def tokio(features, defaults=False):
            packages = self.packages("tokio")
            packages[1]["dependencies"][1].update({"features": features, "uses_default_features": defaults})
            return packages
        code, out = run_input_check({"mod.rs": VALIDATOR}, tokio(["rt"]))
        self.assertEqual(code, 0, out)
        for refused in (tokio(["rt", "sync"]), tokio(["time"]), tokio(["rt"], defaults=True)):
            code, out = run_input_check({"mod.rs": VALIDATOR}, refused)
            self.assertEqual(code, 1, out)

    def test_the_gate_naming_a_runtime_is_refused(self):
        code, out = creator_check(creator_tree(**{
            "crates/application/src/run_input.rs": GATE + "async fn f() { tokio::task::yield_now().await; }\n",
        }))
        self.assertEqual(code, 1, out)
        self.assertIn("crates/application/src/run_input.rs:3: `sagat-application` names `tokio`", out)

    def test_tokio_is_named_only_by_the_blob_source_parse_and_the_tests(self):
        """ADR-0092 (architecture review of 2026-11-15, Blocker 1): the `rt` exception serves one file."""
        code, out = creator_check(creator_tree(**{
            "crates/application/src/runs.rs": "async fn f() { tokio::task::spawn_blocking(|| ()).await; }\n",
        }))
        self.assertEqual(code, 1, out)
        self.assertIn("crates/application/src/runs.rs:1: `sagat-application` names `tokio`", out)
        self.assertIn("ADR-0092", out)
        code, out = creator_check(creator_tree(**{
            "crates/application/src/collection_source.rs": "async fn f() { tokio::task::spawn_blocking(|| ()).await; }\n",
            "crates/application/src/runs.rs": "//! not `tokio`\n#[cfg(test)]\nmod tests {\n    #[tokio::test]\n    async fn t() {}\n}\n",
        }))
        self.assertEqual(code, 0, out)

    def test_a_build_dependency_is_refused(self):
        code, out = run_input_check({"mod.rs": VALIDATOR}, self.packages(("tokio", "build")))
        self.assertEqual(code, 1, out)
        self.assertIn("as a build dependency", out)

    def test_the_application_of_this_repository_passes(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("the application's runtime confined to one file", result.stdout)


def creator_check(files, packages=None):
    """The checker over a tree of Rust sources `files` ({relative path: source}) and a dependency graph of `packages`."""
    with tempfile.TemporaryDirectory() as tmp:
        root = pathlib.Path(tmp) / "repo"
        for rel, source in files.items():
            path = root / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(source)
        run_input = pathlib.Path(tmp) / "run_input"
        run_input.mkdir()
        (run_input / "mod.rs").write_text(VALIDATOR)
        packages = packages or [pkg("d", "crates/domain", [])]
        metadata = {
            "workspace_root": "/r",
            "workspace_members": [p["id"] for p in packages],
            "packages": packages,
        }
        meta = pathlib.Path(tmp) / "metadata.json"
        meta.write_text(json.dumps(metadata))
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--metadata", str(meta), "--run-input-src", str(run_input), "--rust-root", str(root)],
            capture_output=True,
            text=True,
        )
        return result.returncode, result.stdout


EXECUTION = 'async fn create() {\n    sqlx::query("INSERT INTO runs (id) VALUES ($1)");\n}\n'
CHAINS = (
    'async fn continue_as_new() {\n    let q = "INSERT INTO runs (id) VALUES ($1)";\n}\n'
    'async fn continue_evaluated() {\n    let q = "INSERT INTO runs \\\n     (id) VALUES ($1)";\n}\n'
)
GATE = "pub struct CheckedRunInput { version: u8 }\nfn check() -> CheckedRunInput { CheckedRunInput { version: 1 } }\n"


def creator_tree(**extra):
    files = {
        "crates/infra-postgres/src/execution.rs": EXECUTION,
        "crates/infra-postgres/src/chain_progress.rs": CHAINS,
        "crates/application/src/run_input.rs": GATE,
    }
    files.update(extra)
    return files


class EveryRunIsCreatedWithACheckedInput(unittest.TestCase):
    """Design 043, FR-043-016: the run-insert, legacy-continuation, test-support and construction rules."""

    def test_the_three_places_and_the_gate_pass(self):
        code, out = creator_check(creator_tree())
        self.assertEqual(code, 0, out)
        self.assertIn("every run created with a checked input", out)

    def test_an_insert_in_another_function_is_refused_and_a_test_module_is_not_read(self):
        code, out = creator_check(creator_tree(**{
            "crates/infra-postgres/src/other.rs": 'fn shortcut() { let q = "insert into runs (id) values ($1)"; }\n',
        }))
        self.assertEqual(code, 1, out)
        self.assertIn("crates/infra-postgres/src/other.rs:1", out)
        self.assertIn("FR-043-016", out)
        code, out = creator_check(creator_tree(**{
            "crates/infra-postgres/src/other.rs": '#[cfg(test)]\nmod tests {\n    fn f() { let q = "INSERT INTO runs (id)"; }\n}\n',
        }))
        self.assertEqual(code, 0, out)

    def test_a_second_insert_in_an_allowed_function_is_refused_and_a_moved_one_is_not(self):
        code, out = creator_check(creator_tree(**{
            "crates/infra-postgres/src/execution.rs": EXECUTION.replace("}\n", '    let again = "INSERT INTO runs (id)";\n}\n'),
        }))
        self.assertEqual(code, 1, out)
        code, out = creator_check(creator_tree(**{"crates/infra-postgres/src/execution.rs": "\n\n\n" + EXECUTION}))
        self.assertEqual(code, 0, out)

    def test_a_place_that_is_gone_is_an_error_and_not_a_pass(self):
        code, out = creator_check(creator_tree(**{"crates/infra-postgres/src/execution.rs": "async fn create() {}\n"}))
        self.assertEqual(code, 1, out)
        self.assertIn("the rule would check nothing", out)

    def test_the_legacy_continuation_is_refused_outside_the_tests_but_a_forwarder_passes(self):
        code, out = creator_check(creator_tree(**{"crates/scheduler/src/passes.rs": "async fn go(s: S) { s.continue_as_new(&c).await; }\n"}))
        self.assertEqual(code, 1, out)
        self.assertIn("crates/scheduler/src/passes.rs:1: the legacy continue-as-new", out)
        code, out = creator_check(creator_tree(**{
            "crates/infra-postgres/src/chain_progress.rs": CHAINS
            + "async fn continue_as_new(&self, c: &C) { self.inner.continue_as_new(c).await }\n",
            "crates/scheduler/src/passes.rs": "#[cfg(test)]\nmod tests { fn t() { s.continue_as_new(&c); } }\n",
        }))
        self.assertEqual(code, 0, out)

    def test_a_forwarder_named_continue_as_new_elsewhere_is_refused(self):
        """Architecture review of 2026-11-15, Major 2: the exemption is the one forwarder's, by file and function."""
        for rel in ("crates/infra-postgres/src/forward.rs", "crates/scheduler/src/passes.rs"):
            with self.subTest(rel=rel):
                code, out = creator_check(creator_tree(**{
                    rel: "async fn continue_as_new(&self, c: &C) { self.inner.continue_as_new(c).await }\n",
                }))
                self.assertEqual(code, 1, out)
                self.assertIn(f"{rel}:1: the legacy continue-as-new", out)

    def test_a_qualified_or_quoted_table_name_and_a_migration_are_read_too(self):
        """Architecture review of 2026-11-15, Nit 16."""
        for sql in ("INSERT INTO public.runs (id) VALUES ($1)", 'INSERT INTO \\"runs\\" (id) VALUES ($1)',
                    'insert into \\"public\\".\\"runs\\" (id)'):
            with self.subTest(sql=sql):
                code, out = creator_check(creator_tree(**{
                    "crates/infra-postgres/src/other.rs": f'fn shortcut() {{ let q = "{sql}"; }}\n',
                }))
                self.assertEqual(code, 1, out)
                self.assertIn("crates/infra-postgres/src/other.rs:1", out)
        code, out = creator_check(creator_tree(**{
            "crates/infra-postgres/migrations/20990101000000_x.up.sql": "CREATE FUNCTION f() AS $$\n  INSERT INTO runs (id) VALUES (1);\n$$;\n",
        }))
        self.assertEqual(code, 1, out)
        self.assertIn("crates/infra-postgres/migrations/20990101000000_x.up.sql:2", out)
        code, out = creator_check(creator_tree(**{
            "crates/infra-postgres/migrations/20990101000000_x.up.sql": "INSERT INTO runs_archive (id) VALUES (1);\n",
        }))
        self.assertEqual(code, 0, out)

    def test_a_composition_root_wires_the_gate_into_every_creator_it_builds(self):
        """Architecture review of 2026-11-15, Major 3: a service built without `.with_run_inputs` holds the unwired gate,
        which refuses every start of a version with a schema as busy (D043-120)."""
        for ctor in ("RunService::new", "sagat_application::runs::RunService::new", "Scheduler::new",
                     "EventDispatcher::new", "ScheduleService::new"):
            with self.subTest(ctor=ctor):
                code, out = creator_check(creator_tree(**{
                    "apps/control-plane/src/lib.rs": f"fn build() -> S {{\n    {ctor}(a, b)\n        .with_lag_budget(x)\n}}\n",
                }))
                self.assertEqual(code, 1, out)
                self.assertIn("apps/control-plane/src/lib.rs:2: ", out)
                self.assertIn("with_run_inputs", out)
        code, out = creator_check(creator_tree(**{
            "apps/control-plane/src/lib.rs": (
                "fn build() -> S {\n    RunService::new(a, f(b, c))\n        .with_run_inputs(gate(m))\n        .with_lag_budget(x)\n}\n"
                "fn other() { let s = Scheduler::new(a).with_children(c).with_run_inputs(g); }\n"
                "#[cfg(test)]\nmod tests { fn t() { let _ = RunService::new(a); } }\n"
            ),
        }))
        self.assertEqual(code, 0, out)

    def test_a_test_probe_of_the_worker_api_is_refused_outside_the_tests(self):
        """Feature 006, D006-093 (security round 4 Minor 2): the probes hold claims until a test releases them."""
        for probe in (
            "with_delivery_probe",
            "with_commit_probe",
            "synchronous_commit_for_tests",
            # Task 006-049: the compiled minor the connection layer judges the reserve by, and the connection ordinal's hook
            "with_compiled_minor_for_tests",
            "with_connection_probe",
            "rollback_stalled_for_tests",
        ):
            with self.subTest(probe=probe):
                code, out = creator_check(creator_tree(**{
                    "apps/execution-engine/src/lib.rs": f"fn build() -> G {{\n    WorkerGrpc::new(a, b)\n        .{probe}(p)\n}}\n",
                }))
                self.assertEqual(code, 1, out)
                self.assertIn("apps/execution-engine/src/lib.rs:3: ", out)
                self.assertIn(probe, out)
        # security review round 2 of Task 006-049: a probe called through its type's path, or passed as a function, is a call
        for source in (
            "fn build() -> G {\n    WorkerApi::with_connection_probe(api, hook)\n}\n",
            "fn build() -> G {\n    apis.map(WorkerApi::with_compiled_minor_for_tests)\n}\n",
            "async fn end(tx: T) {\n    PgAcknowledgementTx::rollback_stalled_for_tests(tx).await\n}\n",
        ):
            with self.subTest(source=source):
                code, out = creator_check(creator_tree(**{"apps/execution-engine/src/lib.rs": source}))
                self.assertEqual(code, 1, out)
                self.assertIn("apps/execution-engine/src/lib.rs:2: ", out)
        code, out = creator_check(creator_tree(**{
            "crates/api-grpc/src/service.rs": (
                "/// set it with `WorkerGrpc::with_commit_probe`\n"
                "pub fn with_commit_probe(mut self, probe: P) -> Self { self }\n"
                "pub fn with_delivery_probe(mut self, probe: P) -> Self { self }\n"
                "#[cfg(test)]\nmod tests { fn t() { let _ = g.with_commit_probe(p); } }\n"
            ),
        }))
        self.assertEqual(code, 0, out)

    def test_a_checked_input_built_outside_the_gate_is_refused(self):
        for source in (
            "fn f() -> X { CheckedRunInput { version: 1 } }\n",
            "fn f() -> X { sagat_application::run_input::CheckedRunInput { version: 1 } }\n",
            "fn f() -> X { CheckedRunInput::unchecked(v, b) }\n",
            "fn f() -> X { crate::run_input::CheckedRunInput :: unchecked(v, b) }\n",
        ):
            with self.subTest(source=source):
                code, out = creator_check(creator_tree(**{"crates/scheduler/src/events.rs": source}))
                self.assertEqual(code, 1, out)
                self.assertIn("crates/scheduler/src/events.rs:1: a `CheckedRunInput` is built outside", out)

    def test_naming_the_type_is_not_building_it(self):
        code, out = creator_check(creator_tree(**{
            "crates/application/src/runs.rs": (
                "pub struct NewRun { pub input: crate::run_input::CheckedRunInput, }\n"
                "fn input(&self) -> &crate::run_input::CheckedRunInput { &self.input }\n"
                "fn f(x: Vec<CheckedRunInput>) {}\n"
                "#[cfg(test)]\nmod tests { fn t() { let _ = CheckedRunInput::unchecked(v, b); } }\n"
            ),
        }))
        self.assertEqual(code, 0, out)

    def test_a_default_or_a_deserialize_is_refused_even_in_the_gate(self):
        for source in (
            "#[derive(Clone, Default)]\npub struct CheckedRunInput { version: u8 }\n",
            "#[derive(serde::Deserialize)]\n#[serde(deny_unknown_fields)]\npub struct CheckedRunInput { version: u8 }\n",
            GATE + "impl Default for CheckedRunInput { fn default() -> Self { todo!() } }\n",
            GATE + "impl<'de> serde::Deserialize<'de> for CheckedRunInput { }\n",
        ):
            with self.subTest(source=source):
                code, out = creator_check(creator_tree(**{"crates/application/src/run_input.rs": source}))
                self.assertEqual(code, 1, out)
                self.assertIn("D043-045", out)

    def test_the_test_support_feature_belongs_in_dev_dependencies_only(self):
        def packages(dependency, features=None):
            application = pkg("sagat-application", "crates/application", [])
            user = pkg("sagat-tests", "tests", [])
            user["dependencies"] = [dependency]
            user["features"] = features or {}
            # named by the lake-runner rule whenever the application is in the graph
            return [
                application,
                user,
                pkg("sagat-database-runner", "apps/database-runner", ["sagat-application"]),
                pkg("sagat-infra-lake", "crates/infra-lake", ["sagat-application"]),
            ]

        dev = {"name": "sagat-application", "kind": "dev", "features": ["test-support"]}
        code, out = creator_check(creator_tree(), packages(dev))
        self.assertEqual(code, 0, out)
        normal = {"name": "sagat-application", "kind": None, "features": ["test-support"]}
        code, out = creator_check(creator_tree(), packages(normal))
        self.assertEqual(code, 1, out)
        self.assertIn("as a normal dependency", out)
        plain = {"name": "sagat-application", "kind": None, "features": []}
        code, out = creator_check(creator_tree(), packages(plain, {"testing": ["sagat-application/test-support"]}))
        self.assertEqual(code, 1, out)
        self.assertIn("feature `testing` enables", out)


class TheRuntimeRoleCannotDeleteTheJournal(unittest.TestCase):
    """Constitution 5.1.0, principle I: journal rows leave only by two audited paths, and the runtime role holds no DELETE
    on runs, steps, the journal or payloads (architecture review of 5.1.0, Major 6)."""

    MIGRATION = "crates/infra-postgres/migrations/20990101000000_x.up.sql"

    def test_the_grants_the_runtime_role_has_pass(self):
        code, out = creator_check(creator_tree(**{
            self.MIGRATION: "GRANT SELECT, INSERT, UPDATE ON runs, steps TO sagat_app;\n"
                            "GRANT SELECT, INSERT ON run_events, payloads TO sagat_app;\n"
                            "GRANT SELECT, INSERT, UPDATE, DELETE ON event_outbox TO sagat_app;\n",
        }))
        self.assertEqual(code, 0, out)
        self.assertIn("the runtime role cannot delete the journal", out)

    def test_a_delete_truncate_or_all_grant_on_a_journal_table_is_refused(self):
        for sql in ("GRANT SELECT, DELETE ON runs TO sagat_app;",
                    "GRANT TRUNCATE ON TABLE public.run_events TO sagat_app;",
                    "grant all privileges on steps, other to sagat_app;",
                    'GRANT DELETE ON "payloads" TO sagat_app, sagat_other;',
                    "GRANT DELETE ON ALL TABLES IN SCHEMA public TO sagat_app;",
                    "ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT DELETE ON TABLES TO sagat_app;",
                    "GRANT DELETE ON runs TO PUBLIC;",
                    "GRANT DELETE ON runs TO sagat_app WITH GRANT OPTION;",
                    "GRANT DELETE ON runs TO sagat_app GRANTED BY sagat_owner;",
                    "GRANT DELETE ON run_payloads TO sagat_app;",
                    "GRANT sagat_owner TO sagat_app;",
                    "ALTER TABLE runs OWNER TO sagat_app;",
                    "DO $$ BEGIN EXECUTE format('GRANT DELETE ON %I TO sagat_app', 'runs'); END $$;",
                    "GRANT sagat_owner TO sagat_app WITH ADMIN OPTION;",
                    "GRANT sagat_owner TO sagat_app WITH INHERIT TRUE, SET TRUE;",
                    "CREATE ROLE sagat_purger NOLOGIN ROLE sagat_app;",
                    "ALTER GROUP sagat_owner ADD USER sagat_app;",
                    "REASSIGN OWNED BY sagat_owner TO sagat_app;",
                    "GRANT DELETE ON run_events_2026_12 TO sagat_app;",
                    "DO $$ BEGIN EXECUTE format('GRANT %s ON %I TO %I', 'DELETE', 'runs', 'sagat_app'); END $$;",
                    "/* a block comment */ GRANT DELETE ON runs TO sagat_app"):
            with self.subTest(sql=sql):
                code, out = creator_check(creator_tree(**{self.MIGRATION: "-- a comment\n" + sql + "\n"}))
                self.assertEqual(code, 1, out)
                self.assertIn(f"{self.MIGRATION}:2", out)
                self.assertIn("principle I", out)

    def test_a_down_migration_is_read_too(self):
        down = self.MIGRATION.replace(".up.sql", ".down.sql")
        code, out = creator_check(creator_tree(**{down: "GRANT TRUNCATE ON steps TO sagat_app;\n"}))
        self.assertEqual(code, 1, out)
        self.assertIn(f"{down}:1", out)

    def test_a_grant_on_another_table_or_a_comment_passes(self):
        for sql in ("GRANT DELETE ON runs_archive TO sagat_app;", "-- GRANT DELETE ON runs TO sagat_app;",
                    "GRANT DELETE ON runs TO sagat_owner;", "REVOKE DELETE ON runs FROM sagat_app;",
                    "REVOKE GRANT OPTION FOR SELECT ON runs FROM sagat_app;\nGRANT SELECT ON steps TO sagat_app;",
                    "/* GRANT DELETE ON runs TO sagat_app; */ SELECT 1;",
                    "GRANT DELETE ON run_events_archive TO sagat_app;"):
            with self.subTest(sql=sql):
                code, out = creator_check(creator_tree(**{self.MIGRATION: sql + "\n"}))
                self.assertEqual(code, 0, out)



CAPABILITY_SOURCE = '''
impl Capability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SchedulesV1 => "schedules-v1",
            Self::RunInputValidationV1 => crate::run_input::CAPABILITY,
            Self::LeaseWithdrawalV1 => "lease-withdrawal-v1",
        }
    }
}
'''
RUN_INPUT_SOURCE = 'pub const CAPABILITY: &str = "run-input-validation-v1";\n'


def capability_migration(values):
    listed = ", ".join(f"'{v}'" for v in values)
    return (
        "ALTER TABLE serving_instances DROP CONSTRAINT serving_instances_capabilities_known,\n"
        f"    ADD CONSTRAINT serving_instances_capabilities_known CHECK (capabilities <@ ARRAY[{listed}]::text[]\n"
        "    AND cardinality(capabilities) <= 32) NOT VALID;\n"
        "ALTER TABLE platform_requirements DROP CONSTRAINT platform_requirements_capability_known,\n"
        f"    ADD CONSTRAINT platform_requirements_capability_known CHECK (capability IN ({listed})) NOT VALID;\n"
        "ALTER TABLE platform_requirement_history DROP CONSTRAINT platform_requirement_history_capability_known,\n"
        f"    ADD CONSTRAINT platform_requirement_history_capability_known CHECK (capability IN ({listed}));\n"
    )


def run_capability_check(migrations, source=CAPABILITY_SOURCE):
    """The capability-list rule over a tree: `migrations` maps a file name to its text."""
    with tempfile.TemporaryDirectory() as root:
        base = pathlib.Path(root)
        (base / "crates/application/src").mkdir(parents=True)
        (base / "crates/application/src/instances.rs").write_text(source)
        (base / "crates/application/src/run_input.rs").write_text(RUN_INPUT_SOURCE)
        folder = base / "crates/infra-postgres/migrations"
        folder.mkdir(parents=True)
        for name, text in migrations.items():
            (folder / name).write_text(text)
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--capability-root", root], capture_output=True, text=True
        )
        return result.returncode, result.stdout


ALL_THREE = ["schedules-v1", "run-input-validation-v1", "lease-withdrawal-v1"]


class CapabilityListsFollowTheCode(unittest.TestCase):
    """Feature 006, decision D006-033: the newest definition of each capability check lists exactly `Capability::ALL`."""

    def test_the_repository_passes(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("the capability checks list the code's capabilities", result.stdout)

    def test_the_newest_migration_listing_every_capability_passes(self):
        code, out = run_capability_check({
            "20270101000000_a.up.sql": capability_migration(["schedules-v1"]),
            "20270102000000_b.up.sql": capability_migration(ALL_THREE),
            # the down of b restores the list before b, and a down is never the newest definition
            "20270102000000_b.down.sql": capability_migration(["schedules-v1"]),
        })
        self.assertEqual(code, 0, out)

    def test_a_newer_migration_that_drops_a_capability_fails(self):
        # the merge order of two branches: the older-numbered one lands second and is applied last by nobody, but the
        # newest by version is the one a fresh database ends with
        code, out = run_capability_check({
            "20270102000000_b.up.sql": capability_migration(ALL_THREE),
            "20270103000000_c.up.sql": capability_migration(["schedules-v1", "run-input-validation-v1"]),
        })
        self.assertEqual(code, 1, out)
        self.assertIn("lease-withdrawal-v1", out)
        self.assertIn("serving_instances_capabilities_known", out)

    def test_a_list_with_a_capability_the_code_does_not_know_fails(self):
        code, out = run_capability_check({
            "20270102000000_b.up.sql": capability_migration(ALL_THREE + ["flow-control-rate-v1"]),
        })
        self.assertEqual(code, 1, out)
        self.assertIn("flow-control-rate-v1", out)

    def test_a_down_that_restores_another_list_than_the_one_before_its_up_fails(self):
        # decision D006-083: the rollback of c must restore b's list, or it erases b's capability
        code, out = run_capability_check({
            "20270101000000_a.up.sql": capability_migration(["schedules-v1"]),
            "20270102000000_b.up.sql": capability_migration(["schedules-v1", "run-input-validation-v1"]),
            "20270102000000_b.down.sql": capability_migration(["schedules-v1"]),
            "20270103000000_c.up.sql": capability_migration(ALL_THREE),
            "20270103000000_c.down.sql": capability_migration(["schedules-v1"]),
        })
        self.assertEqual(code, 1, out)
        self.assertIn("20270103000000_c.down.sql", out)
        self.assertNotIn("20270102000000_b.down.sql", out)

    def test_a_down_that_does_not_restore_the_list_at_all_fails(self):
        code, out = run_capability_check({
            "20270101000000_a.up.sql": capability_migration(["schedules-v1"]),
            "20270102000000_b.up.sql": capability_migration(ALL_THREE),
            "20270102000000_b.down.sql": "SELECT 1;\n",
        })
        self.assertEqual(code, 1, out)
        self.assertIn("20270102000000_b.down.sql", out)

    def test_a_tree_without_the_checks_is_an_error_and_not_a_pass(self):
        code, out = run_capability_check({"20270102000000_b.up.sql": "SELECT 1;\n"})
        self.assertEqual(code, 1, out)
        self.assertIn("would check nothing", out)


if __name__ == "__main__":
    unittest.main()


class MarksOfProtocol18HaveAllowedForms(unittest.TestCase):
    """Feature 006, Task 006-047 (decisions D006-042, D006-066, D006-107): no statement assigns a mark from a value it read
    earlier; the attempt's flag has one fenced form, in the three lease statements only."""

    def check(self, **files):
        return run_lease_check(tree(**files))

    def refused(self, needle, **files):
        code, out = self.check(**files)
        self.assertEqual(code, 1, out)
        self.assertIn(needle, out)
        return out

    def test_the_adapter_of_this_repository_passes(self):
        result = subprocess.run([sys.executable, str(SCRIPT)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stdout)
        self.assertIn("every mark of 1.8 written in an allowed form", result.stdout)

    def test_the_allowed_forms_pass(self):
        later = """
    async fn acknowledge() {
        sqlx::query("UPDATE steps SET lease_acknowledged_at = clock_timestamp() WHERE lease_offered_generation = generation");
    }
    async fn record_capability() {
        sqlx::query("UPDATE steps s SET lease_capability_generation = s.generation WHERE s.lease_capability_generation IS NULL");
    }
    async fn withdraw() {
        sqlx::query("UPDATE steps SET status = 'READY', withdrawn_lease_token_hash = lease_token_hash, \
            withdrawn_on_hand_back = $5 WHERE tenant_id = $1");
    }
    async fn heartbeat() {
        sqlx::query("UPDATE steps SET lease_expires_at = $3, lease_acknowledged_at = lease_acknowledged_at WHERE tenant_id = $1");
    }
"""
        code, out = self.check(**{"later.rs": later, "../migrations/1_marks.up.sql": "ALTER TABLE steps ADD COLUMN provisional_withdrawn_attempt integer;\n-- provisional_withdrawn_attempt = $1 in a comment\n"})
        self.assertEqual(code, 0, out)

    def test_a_bound_flag_form_is_refused(self):
        bound = lease_statement(
            "upsert_path_instance", "s",
            marks=CLAIM_MARKS_SQL.replace(
                "lease_offered_generation = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND {a}generation IS DISTINCT FROM $8",
                "lease_offered_generation = CASE WHEN $35",
            ),
        )
        self.refused("is assigned on a bound flag alone", **{"execution/path.rs": bound})
        # a condition that only looks like a new lease, the generation test or'ed with a flag, is a bound flag too
        ored = lease_statement(
            "upsert_path_instance", "s",
            marks=CLAIM_MARKS_SQL.replace(
                "lease_claimed_at = CASE WHEN $22::bytea IS NOT NULL AND $6 = 'LEASED' AND",
                "lease_claimed_at = CASE WHEN $35 OR",
            ),
        )
        self.refused("`lease_claimed_at` is assigned on a bound flag alone", **{"execution/path.rs": ored})

    def test_a_read_back_form_is_refused(self):
        for statement in (
            "UPDATE steps SET lease_acknowledged_at = $3 WHERE tenant_id = $1",
            "UPDATE steps SET lease_claimed_at = COALESCE($30, lease_claimed_at) WHERE tenant_id = $1",
            "UPDATE steps s SET lease_capability_generation = $4 WHERE s.tenant_id = $1",
        ):
            with self.subTest(statement=statement):
                source = f'async fn later() {{ sqlx::query("{statement}"); }}'
                self.refused("(a read-back form)", **{"later.rs": source})

    def test_a_claim_value_read_earlier_is_refused(self):
        stale = lease_statement(
            "upsert_step",
            marks=CLAIM_MARKS_SQL.replace("THEN CASE WHEN $34 THEN clock_timestamp() END", "THEN $37"),
        )
        self.refused("`lease_claimed_at` gets `$37` on a new lease", **{"execution.rs": FENCE + stale + lease_statement("upsert_scoped_instance", "s")})

    def test_one_of_the_withdrawn_pair_without_the_other_is_refused(self):
        for statement in (
            "UPDATE steps SET status = 'READY', withdrawn_lease_token_hash = lease_token_hash WHERE tenant_id = $1",
            "UPDATE steps SET status = 'READY', withdrawn_on_hand_back = $5 WHERE tenant_id = $1",
        ):
            with self.subTest(statement=statement):
                source = f'async fn withdraw() {{ sqlx::query("{statement}"); }}'
                self.refused("is assigned without the other", **{"later.rs": source})
        bound_hash = 'async fn withdraw() { sqlx::query("UPDATE steps SET withdrawn_lease_token_hash = $4, withdrawn_on_hand_back = $5"); }'
        self.refused("the withdrawal writes the hash from the row it holds", **{"later.rs": bound_hash})

    def test_the_attempt_flag_outside_the_three_lease_statements_is_refused(self):
        source = (
            'async fn withdraw() { sqlx::query("UPDATE steps SET provisional_withdrawn_attempt = CASE WHEN $3 AND '
            "status = 'LEASED' AND generation = $4 THEN attempt ELSE provisional_withdrawn_attempt END\"); }"
        )
        self.refused("is assigned outside the three lease statements", **{"later.rs": source})
        read = 'async fn gauge() { sqlx::query("SELECT count(*) FROM steps WHERE provisional_withdrawn_attempt = attempt"); }'
        self.refused("is named outside the three lease statements", **{"later.rs": read})
        migration = "UPDATE steps SET provisional_withdrawn_attempt = attempt WHERE status = 'LEASED';\n"
        self.refused("is named outside the three lease statements", **{"../migrations/2_flag.up.sql": migration})

    def flag(self, form):
        statement = lease_statement("upsert_scoped_instance", "s", flag=form)
        return {"execution.rs": FENCE + lease_statement("upsert_step") + statement}

    def test_a_bound_attempt_is_refused(self):
        self.refused(
            "is given a bound attempt",
            **self.flag("provisional_withdrawn_attempt = CASE WHEN $40 AND {a}status = 'LEASED' AND {a}generation = $41 THEN $7 ELSE {a}provisional_withdrawn_attempt END"),
        )

    def test_a_branch_that_yields_null_is_refused(self):
        for form in (
            "provisional_withdrawn_attempt = CASE WHEN $40 AND {a}status = 'LEASED' AND {a}generation = $41 THEN {a}attempt END",
            "provisional_withdrawn_attempt = CASE WHEN $40 AND {a}status = 'LEASED' AND {a}generation = $41 THEN {a}attempt ELSE NULL END",
            "provisional_withdrawn_attempt = NULL",
        ):
            with self.subTest(form=form):
                self.refused("yields NULL", **self.flag(form))

    def test_a_form_without_the_old_row_fence_is_refused(self):
        for form in (
            "provisional_withdrawn_attempt = CASE WHEN $40 THEN {a}attempt ELSE {a}provisional_withdrawn_attempt END",
            "provisional_withdrawn_attempt = CASE WHEN $40 AND {a}generation = $41 THEN {a}attempt ELSE {a}provisional_withdrawn_attempt END",
            "provisional_withdrawn_attempt = CASE WHEN $40 AND {a}status = 'LEASED' THEN {a}attempt ELSE {a}provisional_withdrawn_attempt END",
        ):
            with self.subTest(form=form):
                self.refused("without the old-row fence", **self.flag(form))

    def test_a_second_assignment_in_one_statement_is_refused(self):
        twice = ATTEMPT_FLAG_SQL + ", " + ATTEMPT_FLAG_SQL
        self.refused("is assigned twice in one statement", **self.flag(twice))

    def test_an_insert_that_names_a_mark_is_refused(self):
        source = 'async fn copy() { sqlx::query("INSERT INTO steps (tenant_id, lease_claimed_at) VALUES ($1, $2)"); }'
        self.refused("an `INSERT INTO steps` names lease_claimed_at", **{"later.rs": source})

    def test_a_mark_assigned_in_a_migration_from_a_value_is_refused(self):
        migration = "CREATE FUNCTION f(p bigint) RETURNS void LANGUAGE sql AS $$ UPDATE steps SET lease_offered_generation = p $$;\n"
        self.refused("(a read-back form)", **{"../migrations/3_f.up.sql": migration})

    def test_the_claims_form_outside_the_lease_statements_is_refused(self):
        source = (
            'async fn reclaim() { sqlx::query("UPDATE steps SET lease_acknowledged_at = CASE WHEN $22::bytea IS NOT NULL '
            "AND generation IS DISTINCT FROM $8 THEN NULL ELSE lease_acknowledged_at END\"); }"
        )
        self.refused("is written outside the three lease statements", **{"later.rs": source})

    def test_a_lease_statement_that_does_not_write_a_mark_or_is_gone_is_an_error(self):
        missing = lease_statement(
            "upsert_path_instance", "s",
            marks=CLAIM_MARKS_SQL.split(", \\\n    lease_capability_generation")[0],
        )
        self.refused("`upsert_path_instance` (execution/path.rs) does not write `lease_capability_generation`", **{"execution/path.rs": missing})
        self.refused("`upsert_path_instance` is named by the rule of the marks and is found 0 times", **{"execution/path.rs": "async fn other() {}"})
