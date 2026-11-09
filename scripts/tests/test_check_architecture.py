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


def tree(keeper=KEEPER, execution=FENCE, schedules=CHECK_FENCE, **more):
    return {"partitioning.rs": keeper, "execution.rs": execution, "schedules.rs": schedules, **more}


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


if __name__ == "__main__":
    unittest.main()
