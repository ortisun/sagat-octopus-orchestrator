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


if __name__ == "__main__":
    unittest.main()
