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

    def test_cycles_are_reported(self):
        code, out = run_check([
            pkg("x", "crates/telemetry", ["y"]),
            pkg("y", "crates/scheduler", ["x"]),
        ])
        self.assertEqual(code, 1)
        self.assertIn("dependency cycle", out)


if __name__ == "__main__":
    unittest.main()
