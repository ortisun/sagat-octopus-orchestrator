"""Tests for scripts/select-test-targets.py (run: python3 -m unittest discover scripts/tests).

Every case builds a throwaway git repository with a small Cargo workspace and a tests/Cargo.toml that mimics the
real one, commits a base, changes files, and asks the script what the CI must run.
"""
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
    "integration_iceberg_writes": "integration/iceberg-writes/main.rs",
    "e2e_iceberg_writes": "e2e/iceberg-writes/main.rs",
}
LAKE_SUITE_NAMES = {"integration_iceberg_writes", "e2e_iceberg_writes"}
# What each target's own sources name: integration_a -> domain, contract_a -> application, contract_rest_a -> leaf.
SOURCES = {
    "integration/a/main.rs": "use sagat_domain::Thing;\n",
    "contract/a/main.rs": "use sagat_application::Service;\n",
    "contract/a/rest/main.rs": "use sagat_leaf::Leaf;\n",
}
WORKSPACE = {
    "Cargo.toml": '[workspace]\nmembers = ["crates/domain", "crates/application", "crates/leaf", "apps/cli", "tests"]\n',
    "crates/domain/Cargo.toml": '[package]\nname = "sagat-domain"\n',
    "crates/application/Cargo.toml": '[package]\nname = "sagat-application"\n\n[dependencies]\nsagat-domain = { workspace = true }\n',
    "crates/leaf/Cargo.toml": '[package]\nname = "sagat-leaf"\n',
    "apps/cli/Cargo.toml": '[package]\nname = "sagat-cli"\n\n[dependencies]\nsagat-application.workspace = true\n',
}
SDK_SUITE_NAMES = {"sdk_conformance", "e2e_sdk_multilanguage", "fault_sdk_multilanguage"}
BROWSER_SUITE_NAMES = {"e2e_workflow_studio", "e2e_developer_experience", "e2e_database_connections"}


def manifest(targets=None, description="d", dependency='tempfile = "3"'):
    chosen = TARGETS if targets is None else targets
    head = (f'[package]\nname = "sagat-tests"\ndescription = "{description}"\nautotests = false\n\n'
            f"[dev-dependencies]\n{dependency}\n")
    return head + "".join(f'\n[[test]]\nname = "{n}"\npath = "{p}"\n' for n, p in chosen.items())


def git(root, *args):
    return subprocess.run([*GIT, *args], cwd=root, env={**os.environ, **GIT_ENV},
                          capture_output=True, text=True, check=True).stdout.strip()


class Scope(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = pathlib.Path(self._tmp.name)
        git(self.root, "init", "-q", "-b", "develop")
        for rel, text in WORKSPACE.items():
            self.write(rel, text)
        self.write("tests/Cargo.toml", manifest())
        for path in TARGETS.values():
            self.write(f"tests/{path}", SOURCES.get(path, "// main\n"))
        self.write("tests/support/mod.rs", "// support names every crate: use sagat_domain; use sagat_leaf;\n")
        self.write("tests/fixtures/data.json", "{}\n")
        self.write("tests/AGENTS.md", "# tests\n")
        self.write("crates/domain/src/lib.rs", "// lib\n")
        self.write("crates/application/src/lib.rs", "// lib\n")
        self.write("crates/leaf/src/lib.rs", "// lib\n")
        self.write("apps/cli/src/main.rs", "// main\n")
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

    def run_select(self, event="pull_request", ref_name="feature/x", ref_type="branch", base_ref="develop",
                   base="BASE", output=None):
        base = self.base if base == "BASE" else base
        env = {k: v for k, v in os.environ.items() if not k.startswith("GITHUB_")}
        env.update(GIT_ENV)
        if output:
            env["GITHUB_OUTPUT"] = str(output)
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root), "--event", event, "--ref-name", ref_name,
             "--ref-type", ref_type, "--base-ref", base_ref, "--base", base],
            capture_output=True, text=True, env=env)

    def decide(self, **kwargs):
        result = self.run_select(**kwargs)
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        values = {}
        for line in result.stdout.splitlines():
            key, sep, value = line.partition("=")
            if sep and key in {"full", "crates", "sdks", "frontend", "scripts", "lake", "targets", "any", "all"}:
                values[key] = value
        return values

    def chosen(self, **kwargs):
        return set(self.decide(**kwargs)["targets"].split())

    # -- which files select which integration targets -------------------------------------
    def test_a_modified_file_in_a_target_directory_selects_that_target_only(self):
        self.write("tests/fault/b/steps.rs", "// new\n")
        self.write("tests/fault/b/main.rs", "// changed\n")
        self.commit()
        self.assertEqual(self.chosen(), {"fault_b"})

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
        self.write("tests/fault/b/moved.rs", "// m\n")
        self.base = self.commit("with moved")
        git(self.root, "mv", "tests/fault/b/moved.rs", "tests/integration/a/moved.rs")
        self.commit()
        self.assertEqual(self.chosen(), {"integration_a", "fault_b"})

    def test_shared_support_and_fixtures_select_every_target(self):
        for rel in ("tests/support/mod.rs", "tests/fixtures/data.json"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                self.assertEqual(self.chosen(), set(TARGETS))
                self.base = git(self.root, "rev-parse", "HEAD")

    # -- the lake runtime (runners/lake) is not a shared input of the suites -------------------
    def test_a_change_under_runners_lake_opens_the_lake_gate_and_selects_only_the_lake_suites(self):
        self.write("runners/lake/sagat_lake/__init__.py", "x = 1\n")
        self.commit()
        decision = self.decide()
        self.assertEqual(decision["lake"], "true")
        self.assertEqual(set(decision["targets"].split()), LAKE_SUITE_NAMES)
        self.assertEqual(decision["all"], "false")

    def test_the_other_runners_still_select_every_target_and_leave_the_lake_gate_closed(self):
        for rel in ("runners/go/Dockerfile", "runners/dotnet/Dockerfile.runtime"):
            with self.subTest(rel=rel):
                self.write(rel, "FROM scratch\n")
                self.commit()
                decision = self.decide()
                self.assertEqual(set(decision["targets"].split()), set(TARGETS))
                self.assertEqual(decision["lake"], "false")

    def test_a_markdown_file_of_the_lake_runtime_selects_nothing(self):
        self.write("runners/lake/README.md", "# lake\n")
        self.commit()
        decision = self.decide()
        self.assertEqual(decision["lake"], "false")
        self.assertEqual(decision["targets"], "")

    def test_a_change_elsewhere_leaves_the_lake_gate_closed(self):
        self.write("crates/domain/src/lib.rs", "// changed\n")
        self.commit()
        self.assertEqual(self.decide()["lake"], "false")

    def test_a_release_opens_the_lake_gate(self):
        for kwargs in ({"event": "push", "ref_name": "main", "base_ref": ""},
                       {"event": "workflow_dispatch", "ref_name": "develop", "base_ref": ""},
                       {"event": "push", "ref_name": "v1.0.0", "ref_type": "tag", "base_ref": ""}):
            with self.subTest(**kwargs):
                self.assertEqual(self.decide(**kwargs)["lake"], "true")

    def test_runners_and_contract_files_select_every_target(self):
        for rel in ("runners/go/Dockerfile", "specs/001-platform-foundation/contracts/openapi.yaml"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                self.assertEqual(self.chosen(), set(TARGETS))
                self.assertEqual(self.decide()["full"], "false")
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_documentation_selects_nothing_at_all(self):
        self.write("README.md", "changed\n")
        self.write("docs/guide.md", "x\n")
        self.write("tasks/reviews/r.md", "x\n")
        self.write("tests/AGENTS.md", "# changed\n")
        self.write("crates/domain/README.md", "x\n")
        self.write("infra/helm/values.yaml", "a: 1\n")
        self.commit()
        decision = self.decide()
        self.assertEqual(
            decision,
            {"full": "false", "crates": "", "sdks": "", "frontend": "false", "scripts": "false", "lake": "false",
             "targets": "", "any": "false", "all": "false"})

    def test_frontend_journeys_select_the_browser_suites(self):
        self.write("frontend/e2e/journeys/login.spec.ts", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(), BROWSER_SUITE_NAMES)

    def test_sdk_conformance_programs_select_the_sdk_suites(self):
        self.write("sdks/go/conformance/worker.go", "package x\n")
        self.commit()
        self.assertEqual(self.chosen(), SDK_SUITE_NAMES)

    def test_examples_select_the_developer_experience_suite(self):
        self.write("examples/hello-world/typescript/index.ts", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(), {"e2e_developer_experience"})

    # -- the manifest ---------------------------------------------------------------------
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
        result = self.run_select()
        self.assertEqual(result.returncode, 2, result.stdout)
        self.assertNotIn("rm -rf", result.stdout)

    # -- unit tests of the changed packages and of their dependents -------------------------
    def test_a_changed_package_nobody_depends_on_gets_its_own_unit_tests_only(self):
        self.write("crates/leaf/src/lib.rs", "// changed\n")
        self.commit()
        self.assertEqual(self.decide()["crates"], "sagat-leaf")

    def test_a_changed_core_package_pulls_in_the_unit_tests_of_its_dependents(self):
        self.write("crates/domain/src/lib.rs", "// changed\n")
        self.commit()
        # application depends on domain, and the cli on application: a break in domain shows in their tests.
        self.assertEqual(set(self.decide()["crates"].split()), {"sagat-domain", "sagat-application", "sagat-cli"})

    def test_the_integration_crate_is_never_a_dependent_whose_unit_tests_run(self):
        # In the real workspace the integration crate depends on every package, so it would always be "affected".
        self.write("tests/Cargo.toml", manifest(dependency='tempfile = "3"\nsagat-domain = { workspace = true }'))
        self.write("crates/domain/src/lib.rs", "// changed\n")
        self.commit()
        self.assertNotIn("sagat-tests", self.decide()["crates"].split())

    def test_a_changed_middle_package_pulls_in_what_depends_on_it_and_not_what_it_depends_on(self):
        self.write("crates/application/src/lib.rs", "// changed\n")
        self.commit()
        self.assertEqual(set(self.decide()["crates"].split()), {"sagat-application", "sagat-cli"})

    def test_two_changed_packages_are_both_listed(self):
        self.write("crates/leaf/src/lib.rs", "// changed\n")
        self.write("apps/cli/src/main.rs", "// changed\n")
        self.commit()
        self.assertEqual(set(self.decide()["crates"].split()), {"sagat-leaf", "sagat-cli"})

    def test_a_workspace_wide_input_selects_every_package_and_every_target(self):
        for rel in ("Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "vendor/rskafka/src/lib.rs"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                decision = self.decide()
                self.assertEqual(decision["crates"], "ALL")
                self.assertEqual(decision["all"], "true")
                self.assertEqual(set(decision["targets"].split()), set(TARGETS))
                self.assertEqual(decision["full"], "false")
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_the_tests_package_is_never_listed_for_unit_tests(self):
        self.write("tests/fault/b/x.rs", "// x\n")
        self.commit()
        self.assertEqual(self.decide()["crates"], "")

    # -- integration suites run only for what changed ---------------------------------------
    def test_a_changed_package_selects_no_integration_suite_even_one_that_names_it(self):
        # contract_rest_a names sagat_leaf in its sources; its own files did not change, so it does not run.
        self.write("crates/leaf/src/lib.rs", "// changed\n")
        self.commit()
        self.assertEqual(self.chosen(), set())

    def test_a_changed_core_package_selects_no_suite_that_names_it_or_its_dependents(self):
        self.write("crates/domain/src/lib.rs", "// changed\n")
        self.commit()
        # integration_a names sagat_domain and contract_a names sagat_application, which depends on it.
        self.assertEqual(self.chosen(), set())

    def test_an_app_that_no_suite_names_selects_no_suite(self):
        self.write("apps/cli/src/main.rs", "// changed\n")
        self.commit()
        self.assertEqual(self.chosen(), set())

    def test_a_changed_package_and_a_changed_suite_select_that_suite_only(self):
        self.write("crates/domain/src/lib.rs", "// changed\n")
        self.write("tests/fault/b/x.rs", "// x\n")
        self.commit()
        decision = self.decide()
        self.assertEqual(set(decision["targets"].split()), {"fault_b"})
        self.assertEqual(set(decision["crates"].split()), {"sagat-domain", "sagat-application", "sagat-cli"})

    def test_the_log_says_why_a_suite_was_selected(self):
        self.write("tests/fault/b/x.rs", "// x\n")
        self.commit()
        result = self.run_select()
        self.assertIn("select fault_b: tests/fault/b/x.rs", result.stdout)

    # -- SDKs, Studio and scripts ----------------------------------------------------------
    def test_an_sdk_change_selects_that_languages_unit_tests(self):
        self.write("sdks/python/src/client.py", "x = 1\n")
        self.write("sdks/go/client.go", "package x\n")
        self.commit()
        self.assertEqual(set(self.decide()["sdks"].split()), {"python", "go"})

    def test_the_studio_and_the_scripts_are_flagged_when_their_files_change(self):
        self.write("frontend/src/app.ts", "// x\n")
        self.write("scripts/check-x.py", "x = 1\n")
        self.commit()
        decision = self.decide()
        self.assertEqual((decision["frontend"], decision["scripts"]), ("true", "true"))

    def test_a_change_elsewhere_flags_neither(self):
        self.write("crates/leaf/src/lib.rs", "// changed\n")
        self.commit()
        decision = self.decide()
        self.assertEqual((decision["sdks"], decision["frontend"], decision["scripts"]), ("", "false", "false"))

    # -- what a release runs -----------------------------------------------------------------
    def assert_full(self, decision):
        self.assertEqual(decision["full"], "true")
        self.assertEqual(decision["crates"], "ALL")
        self.assertEqual(set(decision["sdks"].split()), {"typescript", "python", "go", "dotnet", "java", "rust"})
        self.assertEqual((decision["frontend"], decision["scripts"], decision["all"]), ("true", "true", "true"))
        self.assertEqual(set(decision["targets"].split()), set(TARGETS))

    def test_a_manual_run_is_a_release(self):
        self.assert_full(self.decide(event="workflow_dispatch", base=""))

    def test_a_push_to_a_release_branch_is_a_release(self):
        for ref in ("main", "release/1.0.0", "hotfix/1.0.1"):
            with self.subTest(ref=ref):
                self.assert_full(self.decide(event="push", ref_name=ref))

    def test_a_release_tag_is_a_release(self):
        self.assert_full(self.decide(event="push", ref_name="v1.2.0", ref_type="tag"))

    def test_another_tag_is_not(self):
        self.write("README.md", "changed\n")
        self.commit()
        self.assertEqual(self.decide(event="push", ref_name="nightly", ref_type="tag")["full"], "false")

    def test_a_pull_request_into_main_or_a_release_branch_is_a_release(self):
        for base_ref in ("main", "release/1.0.0", "hotfix/1.0.1"):
            with self.subTest(base_ref=base_ref):
                self.assert_full(self.decide(base_ref=base_ref))

    def test_a_pull_request_into_develop_and_a_push_to_develop_are_not(self):
        self.write("README.md", "changed\n")
        self.commit()
        self.assertEqual(self.decide(base_ref="develop")["full"], "false")
        self.assertEqual(self.decide(event="push", ref_name="develop", base_ref="")["full"], "false")

    def test_a_change_of_the_workflow_runs_the_whole_workflow(self):
        self.write(".github/workflows/ci.yml", "name: ci\n")
        self.commit()
        self.assert_full(self.decide())

    def test_a_missing_or_unknown_base_is_a_release(self):
        for base in ("", ZEROS, "f" * 40):
            with self.subTest(base=base):
                self.assert_full(self.decide(base=base))

    # -- output ------------------------------------------------------------------------------
    def test_the_github_output_file_gets_the_whole_decision(self):
        self.write("crates/leaf/src/lib.rs", "// changed\n")
        self.write("tests/contract/a/rest/x.rs", "// x\n")
        self.commit()
        out = self.root / "gh_output.txt"
        self.run_select(output=out)
        text = out.read_text()
        for expected in ("full=false\n", "crates=sagat-leaf\n", "sdks=\n", "frontend=false\n", "scripts=false\n",
                         "targets=contract_rest_a\n", "any=true\n", "all=false\n"):
            self.assertIn(expected, text)


if __name__ == "__main__":
    unittest.main()
