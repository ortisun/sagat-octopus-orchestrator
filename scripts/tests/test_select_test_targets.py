"""Tests for scripts/select-test-targets.py (run: python3 -m unittest discover scripts/tests).

Every case builds a throwaway git repository whose tests/Cargo.toml mimics the real one, commits a base,
changes files, and asks the script which integration test targets must run.
"""
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "select-test-targets.py"
GIT_ENV = {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]
ZEROS = "0" * 40

TARGETS = {
    "integration_a": "integration/a/main.rs",
    "contract_a": "contract/a/main.rs",
    "contract_rest_a": "contract/a/rest/main.rs",
    "fault_b": "fault/b/main.rs",
    "e2e_workflow_studio": "e2e/workflow-studio/main.rs",
    "e2e_developer_experience": "e2e/developer-experience/main.rs",
    "e2e_database_connections": "e2e/database-connections/main.rs",
    "sdk_conformance": "contract/sdk-conformance/main.rs",
    "e2e_sdk_multilanguage": "e2e/sdk-multilanguage/main.rs",
    "fault_sdk_multilanguage": "fault/sdk-multilanguage/main.rs",
}


def manifest(targets=None, description="d", dependency='tempfile = "3"'):
    chosen = TARGETS if targets is None else targets
    head = (f'[package]\nname = "sagat-tests"\ndescription = "{description}"\nautotests = false\n\n'
            f"[dev-dependencies]\n{dependency}\n")
    return head + "".join(f'\n[[test]]\nname = "{n}"\npath = "{p}"\n' for n, p in chosen.items())


def git(root, *args):
    return subprocess.run([*GIT, *args], cwd=root, env={**os.environ, **GIT_ENV},
                          capture_output=True, text=True, check=True).stdout.strip()


class Selector(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = pathlib.Path(self._tmp.name)
        git(self.root, "init", "-q", "-b", "develop")
        self.write("tests/Cargo.toml", manifest())
        for path in TARGETS.values():
            self.write(f"tests/{path}", "// main\n")
        self.write("tests/support/mod.rs", "// support\n")
        self.write("tests/fixtures/data.json", "{}\n")
        self.write("tests/AGENTS.md", "# tests\n")
        self.write("crates/domain/src/lib.rs", "// lib\n")
        self.write("README.md", "r\n")
        self.base = self.commit("base")

    def write(self, rel, text):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self, message="change"):
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "--allow-empty", "-m", message)
        return git(self.root, "rev-parse", "HEAD")

    def select(self, event="pull_request", ref_name="feature/x", base="BASE", output=None):
        base = self.base if base == "BASE" else base
        env = {k: v for k, v in os.environ.items() if not k.startswith("GITHUB_")}
        env.update(GIT_ENV)
        if output:
            env["GITHUB_OUTPUT"] = str(output)
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root), "--event", event,
             "--ref-name", ref_name, "--base", base],
            capture_output=True, text=True, env=env)
        return result

    def chosen(self, **kwargs):
        result = self.select(**kwargs)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        line = next(l for l in result.stdout.splitlines() if l.startswith("targets="))
        return set(line[len("targets="):].split())

    # -- which files select which targets ------------------------------------------------
    def test_a_modified_file_in_a_target_directory_selects_that_target_only(self):
        self.write("tests/integration/a/steps.rs", "// new\n")
        self.write("tests/integration/a/main.rs", "// changed\n")
        self.commit()
        self.assertEqual(self.chosen(), {"integration_a"})

    def test_an_added_file_selects_its_target(self):
        self.write("tests/fault/b/new_case.rs", "// added\n")
        self.commit()
        self.assertEqual(self.chosen(), {"fault_b"})

    def test_a_deleted_file_selects_the_target_that_lost_it(self):
        self.write("tests/fault/b/old.rs", "// old\n")
        self.base = self.commit("with old")
        (self.root / "tests/fault/b/old.rs").unlink()
        self.commit()
        self.assertEqual(self.chosen(), {"fault_b"})

    def test_a_file_of_a_nested_target_selects_the_nested_target_not_its_parent(self):
        self.write("tests/contract/a/rest/battery.rs", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(), {"contract_rest_a"})

    def test_a_file_of_the_parent_directory_selects_the_parent_only(self):
        self.write("tests/contract/a/openapi.rs", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(), {"contract_a"})

    def test_a_rename_between_targets_selects_both(self):
        self.write("tests/integration/a/moved.rs", "// m\n")
        self.base = self.commit("with moved")
        git(self.root, "mv", "tests/integration/a/moved.rs", "tests/fault/b/moved.rs")
        self.commit()
        self.assertEqual(self.chosen(), {"integration_a", "fault_b"})

    def test_a_change_outside_the_tests_selects_nothing(self):
        self.write("crates/domain/src/lib.rs", "// changed\n")
        self.write("README.md", "changed\n")
        self.commit()
        result = self.select()
        self.assertEqual(self.chosen(), set())
        self.assertIn("any=false", result.stdout)

    def test_shared_support_and_fixtures_select_every_target(self):
        for rel in ("tests/support/mod.rs", "tests/fixtures/data.json"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                self.assertEqual(self.chosen(), set(TARGETS))
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_documentation_under_tests_selects_nothing(self):
        self.write("tests/AGENTS.md", "# changed\n")
        self.commit()
        self.assertEqual(self.chosen(), set())

    def test_frontend_journeys_select_the_browser_suites(self):
        self.write("frontend/e2e/journeys/login.spec.ts", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(), {"e2e_workflow_studio", "e2e_developer_experience", "e2e_database_connections"})

    def test_sdk_conformance_programs_select_the_sdk_suites(self):
        self.write("sdks/go/conformance/worker.go", "package x\n")
        self.commit()
        self.assertEqual(self.chosen(), {"sdk_conformance", "e2e_sdk_multilanguage", "fault_sdk_multilanguage"})

    # -- the manifest --------------------------------------------------------------------
    def test_a_new_test_entry_selects_that_target(self):
        extra = dict(TARGETS, new_suite="integration/new/main.rs")
        self.write("tests/Cargo.toml", manifest(extra))
        self.write("tests/integration/new/main.rs", "// n\n")
        self.commit()
        self.assertEqual(self.chosen(), {"new_suite"})

    def test_a_changed_path_selects_that_target(self):
        moved = dict(TARGETS, fault_b="fault/b2/main.rs")
        self.write("tests/Cargo.toml", manifest(moved))
        self.commit()
        self.assertEqual(self.chosen(), {"fault_b"})

    def test_a_description_only_change_selects_nothing(self):
        self.write("tests/Cargo.toml", manifest(description="another description"))
        self.commit()
        self.assertEqual(self.chosen(), set())

    def test_a_dependency_change_selects_every_target(self):
        self.write("tests/Cargo.toml", manifest(dependency='tempfile = "4"'))
        self.commit()
        self.assertEqual(self.chosen(), set(TARGETS))

    def test_an_unparseable_base_manifest_selects_every_target(self):
        self.write("tests/Cargo.toml", "not [valid toml")
        self.base = self.commit("broken base")
        self.write("tests/Cargo.toml", manifest())
        self.commit()
        self.assertEqual(self.chosen(), set(TARGETS))

    def test_a_target_name_outside_the_plain_pattern_fails(self):
        bad = dict(TARGETS, **{"bad name; rm -rf": "integration/bad/main.rs"})
        self.write("tests/Cargo.toml", manifest(bad))
        self.commit()
        result = self.select()
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertNotIn("rm -rf", result.stdout)

    # -- events --------------------------------------------------------------------------
    def test_a_manual_run_selects_every_target(self):
        self.assertEqual(self.chosen(event="workflow_dispatch", base=""), set(TARGETS))

    def test_a_push_to_a_release_branch_selects_every_target(self):
        for ref in ("main", "release/1.0.0", "hotfix/1.0.1"):
            with self.subTest(ref=ref):
                self.assertEqual(self.chosen(event="push", ref_name=ref), set(TARGETS))

    def test_a_push_to_develop_selects_by_change(self):
        self.write("tests/fault/b/x.rs", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(event="push", ref_name="develop"), {"fault_b"})

    def test_a_missing_or_unknown_base_selects_every_target(self):
        for base in ("", ZEROS, "f" * 40):
            with self.subTest(base=base):
                self.assertEqual(self.chosen(base=base), set(TARGETS))

    # -- output --------------------------------------------------------------------------
    def test_the_github_output_file_gets_the_selection(self):
        self.write("tests/fault/b/x.rs", "// x\n")
        self.commit()
        out = self.root / "gh_output.txt"
        self.select(output=out)
        text = out.read_text()
        self.assertIn("targets=fault_b\n", text)
        self.assertIn("any=true\n", text)
        self.assertIn("all=false\n", text)

    def test_the_log_says_why_a_target_was_selected(self):
        self.write("tests/fault/b/x.rs", "// x\n")
        self.commit()
        result = self.select()
        self.assertIn("fault_b", result.stdout)
        self.assertIn("tests/fault/b/x.rs", result.stdout)

    # -- the heavy product jobs (upgrade, compose smoke test, scale) -----------------------
    def code(self, **kwargs):
        result = self.select(**kwargs)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        return next(l for l in result.stdout.splitlines() if l.startswith("code=")) == "code=true"

    def test_a_change_to_documentation_and_process_files_alone_is_not_code(self):
        self.write("README.md", "changed\n")
        self.write("docs/guide.md", "x\n")
        self.write("tasks/reviews/r.md", "x\n")
        self.write(".claude/skills/x/SKILL.md", "x\n")
        self.write("tests/AGENTS.md", "changed\n")
        self.write("crates/domain/README.md", "x\n")
        self.write("scripts/tests/test_x.py", "x = 1\n")
        self.write("LICENSE", "x\n")
        self.commit()
        self.assertFalse(self.code())

    def test_any_other_file_makes_the_change_code(self):
        for rel in ("crates/domain/src/lib.rs", ".github/workflows/ci.yml", "infra/docker/compose.yaml",
                    "Cargo.lock", "frontend/package.json", "sdks/go/go.mod", "scripts/smoke-compose.sh",
                    "a_new_top_level_directory/file.txt", "tests/fault/b/x.rs"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                self.assertTrue(self.code())
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_documentation_mixed_with_code_is_code(self):
        self.write("docs/guide.md", "x\n")
        self.write("crates/domain/src/lib.rs", "// changed\n")
        self.commit()
        self.assertTrue(self.code())

    def test_an_empty_change_is_not_code(self):
        self.commit()
        self.assertFalse(self.code())

    def test_every_event_that_runs_everything_is_code(self):
        self.write("docs/guide.md", "x\n")
        self.commit()
        self.assertTrue(self.code(event="workflow_dispatch", base=""))
        self.assertTrue(self.code(event="push", ref_name="release/1.0.0"))
        self.assertTrue(self.code(base=""))
        self.assertTrue(self.code(base="f" * 40))

    def test_the_github_output_file_gets_the_code_flag(self):
        self.write("docs/guide.md", "x\n")
        self.commit()
        out = self.root / "gh_output.txt"
        self.select(output=out)
        self.assertIn("code=false\n", out.read_text())


if __name__ == "__main__":
    unittest.main()
