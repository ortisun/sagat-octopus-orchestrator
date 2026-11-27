"""Tests for scripts/select-test-targets.py (run: python3 -m unittest discover scripts/tests).

Every case builds a throwaway git repository with a small Cargo workspace and a tests/Cargo.toml that mimics the
real one, commits a base, changes files, and asks the script what the CI must run.
"""
import importlib.util
import json
import os
import pathlib
import re
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
    "integration_claim_transition_efficiency": "integration/claim-transition-efficiency/main.rs",
    "integration_step_logs": "integration/step-logs/main.rs",
    "contract_platform_foundation": "contract/platform-foundation/main.rs",
}
BUDGET = {"integration_claim_transition_efficiency"}
# The suites of the server's step-log code (constitution 4.5.0, OD-2026-11-15-10; design 041, task T041-32).
STEP_LOG_SERVER = {"integration_step_logs", "integration_claim_transition_efficiency", "contract_platform_foundation"}
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
            if sep and key in {"full", "crates", "sdks", "frontend", "scripts", "lake", "targets", "any", "all",
                                          "shards"}:
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

    def test_shared_support_and_fixtures_select_no_suite(self):
        # They are used by every suite, and no suite was changed: the release runs them all.
        for rel in ("tests/support/mod.rs", "tests/fixtures/data.json"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                decision = self.decide()
                self.assertEqual(decision["targets"], "")
                self.assertEqual(decision["any"], "false")
                self.assertEqual(decision["full"], "false")
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_a_shared_file_changed_with_a_suite_selects_that_suite_only(self):
        self.write("tests/support/mod.rs", "// changed\n")
        self.write("tests/integration/a/main.rs", "// changed\n")
        self.commit()
        self.assertEqual(self.chosen(), {"integration_a"})

    # -- the lake runtime (runners/lake) is not a shared input of the suites -------------------
    def test_a_change_under_runners_lake_opens_the_lake_gate_and_selects_only_the_lake_suites(self):
        self.write("runners/lake/sagat_lake/__init__.py", "x = 1\n")
        self.commit()
        decision = self.decide()
        self.assertEqual(decision["lake"], "true")
        self.assertEqual(set(decision["targets"].split()), LAKE_SUITE_NAMES)
        self.assertEqual(decision["all"], "false")

    def test_the_other_runners_select_no_suite_and_leave_the_lake_gate_closed(self):
        for rel in ("runners/go/Dockerfile", "runners/dotnet/Dockerfile.runtime"):
            with self.subTest(rel=rel):
                self.write(rel, "FROM scratch\n")
                self.commit()
                decision = self.decide()
                self.assertEqual(decision["targets"], "")
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

    def test_runners_and_contract_files_select_no_suite(self):
        for rel in ("runners/go/Dockerfile", "specs/001-platform-foundation/contracts/openapi.yaml"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                self.assertEqual(self.chosen(), set())
                self.assertEqual(self.decide()["full"], "false")
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_a_contract_file_changed_with_the_suite_that_checks_it_selects_that_suite_only(self):
        self.write("specs/001-platform-foundation/contracts/openapi.yaml", "changed\n")
        self.write("tests/fault/b/x.rs", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(), {"fault_b"})

    # -- a contract selects the suites and the packages that read it (review finding m7) -------
    CONTRACTS = "specs/001-platform-foundation/contracts"

    def readers(self):
        """contract_a names the full path, contract_rest_a the directory in one module and the file in another,
        sdk_conformance reads it through the shared support module, sagat-leaf compiles it in. integration_a reads it
        too, through the support module, but is not a contract suite; fault_b names the directory and another file."""
        self.write(f"{self.CONTRACTS}/openapi.yaml", "openapi: 3.1.0\n")
        self.write(f"{self.CONTRACTS}/worker.proto", "syntax = \"proto3\";\n")
        self.write(f"{self.CONTRACTS}/README.md", "# contracts\n")
        self.write("tests/contract/a/openapi.rs",
                   f'const PATH: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../{self.CONTRACTS}/openapi.yaml");\n')
        self.write("tests/contract/a/rest/main.rs",
                   f'use sagat_leaf::Leaf;\nconst CONTRACTS: &str = "{self.CONTRACTS}";\nmod read;\n')
        self.write("tests/contract/a/rest/read.rs", 'fn doc() { load(super::CONTRACTS, "openapi.yaml"); }\n')
        self.write("tests/support/contract.rs",
                   f'pub fn load() {{ read("/../{self.CONTRACTS}/openapi.yaml"); }}\n')
        self.write("tests/integration/a/main.rs",
                   "use sagat_domain::Thing;\nuse crate::support::{api::TestApi, contract::OpenApi};\n")
        self.write("tests/contract/sdk-conformance/main.rs",
                   "mod support;\nfn spec() { crate::support::contract::OpenApi::load(); }\n")
        self.write("tests/fault/b/main.rs",
                   f'fn proto() {{ compile(["worker.proto"], [root().join("{self.CONTRACTS}")]); }}\n')
        self.write("crates/leaf/build.rs",
                   f'const CONTRACTS: &str = "../../{self.CONTRACTS}";\nfn main() {{ f(format!("{{CONTRACTS}}/worker.proto")); }}\n')
        self.base = self.commit("readers")

    def test_a_contract_change_alone_selects_every_contract_suite_that_reads_that_file(self):
        self.readers()
        self.write(f"{self.CONTRACTS}/openapi.yaml", "openapi: 3.1.1\n")
        self.commit()
        decision = self.decide()
        # integration_a reads it too; it is not a contract suite, so only its own changes select it.
        self.assertEqual(set(decision["targets"].split()), {"contract_a", "contract_rest_a", "sdk_conformance"})
        self.assertEqual((decision["full"], decision["all"]), ("false", "false"))

    def test_a_contract_compiled_into_a_package_selects_its_unit_tests_and_its_dependents(self):
        self.readers()
        self.write("crates/domain/build.rs", f'fn main() {{ f("../../{self.CONTRACTS}/worker.proto"); }}\n')
        self.base = self.commit("the core package compiles it too")
        self.write(f"{self.CONTRACTS}/worker.proto", "syntax = \"proto3\"; // changed\n")
        self.commit()
        decision = self.decide()
        # fault_b compiles it too, and is not a contract suite. The worker protocol is also a harness of the step logs
        # (design 041, task T041-32), which selects their suite and the six-language conformance; no contract suite.
        self.assertEqual(decision["targets"], "integration_step_logs sdk_conformance")
        self.assertEqual(set(decision["crates"].split()), {"sagat-leaf", "sagat-domain", "sagat-application",
                                                           "sagat-cli"})

    def test_the_log_says_which_contract_selected_a_suite(self):
        self.readers()
        self.write(f"{self.CONTRACTS}/openapi.yaml", "openapi: 3.1.1\n")
        self.commit()
        self.assertIn(f"select contract_a: {self.CONTRACTS}/openapi.yaml is a contract it reads",
                      self.run_select().stdout)

    def test_a_contract_file_nobody_names_selects_nothing(self):
        self.readers()
        self.write(f"{self.CONTRACTS}/events.asyncapi.yaml", "asyncapi: 3.0.0\n")
        self.write("specs/018-other/contracts/openapi.yaml", "openapi: 3.1.0\n")
        self.commit()
        decision = self.decide()
        self.assertEqual((decision["targets"], decision["crates"]), ("", ""))

    def test_a_markdown_contract_selects_only_a_suite_that_reads_it(self):
        self.readers()
        self.write(f"{self.CONTRACTS}/README.md", "# changed\n")
        self.commit()
        self.assertEqual(self.chosen(), set())
        self.write("tests/contract/a/errors.rs", f'const TABLE: &str = "{self.CONTRACTS}/README.md";\n')
        self.base = self.commit("a reader of the README")
        self.write(f"{self.CONTRACTS}/README.md", "# changed again\n")
        self.commit()
        self.assertEqual(self.chosen(), {"contract_a"})

    def test_a_file_named_under_another_contracts_directory_is_not_a_read_of_this_one(self):
        self.readers()
        self.write("tests/contract/a/errors.rs", 'const TABLE: &str = "specs/014-other/contracts/README.md";\n')
        self.base = self.commit("a reader of another README")
        self.write(f"{self.CONTRACTS}/README.md", "# changed\n")
        self.commit()
        self.assertEqual(self.chosen(), set())

    def test_a_comment_or_a_sentence_that_names_a_contract_is_not_a_read(self):
        self.write("tests/contract/a/openapi.rs", f"// see {self.CONTRACTS}/openapi.yaml\n")
        self.write("crates/domain/src/lib.rs",
                   f'//! `{self.CONTRACTS}/openapi.yaml`\nconst HINT: &str = "the schema is {self.CONTRACTS}/openapi.yaml; see";\n')
        self.base = self.commit("comments")
        self.write(f"{self.CONTRACTS}/openapi.yaml", "openapi: 3.1.1\n")
        self.commit()
        decision = self.decide()
        self.assertEqual((decision["targets"], decision["crates"]), ("", ""))

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
             "targets": "", "any": "false", "all": "false", "shards": "[]"})

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

    def test_a_dependency_change_selects_no_target_that_was_not_itself_changed(self):
        self.write("tests/Cargo.toml", manifest(dependency='tempfile = "4"'))
        self.commit()
        self.assertEqual(self.chosen(), set())

    def test_a_dependency_change_with_a_changed_entry_selects_that_target_only(self):
        moved = dict(TARGETS, fault_b="fault/b2/main.rs")
        self.write("tests/Cargo.toml", manifest(moved, dependency='tempfile = "4"'))
        self.commit()
        self.assertEqual(self.chosen(), {"fault_b"})

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

    def test_a_workspace_wide_input_selects_every_package_and_no_suite(self):
        for rel in ("Cargo.lock", "rust-toolchain.toml", ".cargo/config.toml", "vendor/rskafka/src/lib.rs"):
            with self.subTest(rel=rel):
                self.write(rel, "changed " + rel + "\n")
                self.commit()
                decision = self.decide()
                self.assertEqual(decision["crates"], "ALL")
                self.assertEqual(decision["all"], "true")
                self.assertEqual(decision["targets"], "")
                self.assertEqual(decision["any"], "false")
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

    # -- the budget of the hot path (OD-2026-11-12-4) --------------------------------------
    def test_a_change_of_the_postgresql_adapter_selects_the_budget_suite_only(self):
        # The execution module of the adapter also selects the step-log suite (the report path that stores a carry):
        # test_the_report_path_that_stores_a_carry_selects_the_step_log_suite_and_the_budget.
        for rel in ("crates/infra-postgres/src/runs.rs", "crates/infra-postgres/src/workers/claim.rs",
                    "crates/infra-postgres/migrations/20991231000000_x.up.sql"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertEqual(self.chosen(), BUDGET)
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_a_change_of_the_claim_or_the_command_path_selects_the_budget_suite(self):
        for rel in ("crates/application/src/execution.rs", "crates/application/src/runs.rs"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertEqual(self.chosen(), BUDGET)
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_a_change_of_the_scheduler_the_reducer_or_the_gates_selects_the_budget_suite(self):
        # feature 022 (D022-030, T022-092): the passes, the reducer and the flow-control ports can each move what a lease,
        # a completion or an admission costs the database (the passes, `crates/scheduler/src/passes.rs`, also run the
        # step-log purge of design 041, so they select the step-log suites too: the step-log test below checks them)
        for rel in ("crates/scheduler/src/flow_control.rs", "crates/scheduler/src/lib.rs",
                    "crates/domain/src/execution/state.rs", "crates/domain/src/execution/flow_control.rs",
                    "crates/application/src/concurrency.rs", "crates/application/src/rate_limits.rs",
                    "crates/application/src/flow_control.rs", "crates/application/src/flow_control/ports.rs",
                    "crates/application/src/keyed_digest.rs", "crates/domain/src/workflow.rs",
                    "crates/domain/src/workflow/flow_control.rs"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertEqual(self.chosen(), BUDGET)
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_the_rest_of_the_domain_selects_no_suite(self):
        for rel in ("crates/domain/src/identity.rs", "crates/domain/src/execution.rs",
                    "crates/application/src/flow_control_api.rs"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertEqual(self.chosen(), set())
    # -- the step logs of design 041 (constitution 4.5.0, OD-2026-11-15-10) ----------------
    def test_the_servers_step_log_code_selects_the_step_log_suites_and_the_fitness_tests(self):
        for rel in ("crates/domain/src/step_logs.rs", "crates/application/src/step_logs.rs",
                    "crates/application/src/step_logs/carry.rs", "crates/infra-postgres/src/step_logs.rs",
                    "crates/api-grpc/src/step_logs.rs", "crates/api-http/src/routes/step_logs.rs",
                    "crates/infra-postgres/migrations/20270210000000_step_logs.up.sql",
                    "crates/infra-postgres/migrations/20270210000100_step_logs_validate.down.sql",
                    "crates/infra-postgres/migrations/20270201000000_step_log_quotas.up.sql",
                    "crates/api-grpc/src/limits.rs", "crates/api-grpc/src/service.rs",
                    "crates/api-http/src/routes/execution.rs", "crates/api-http/src/routes/scripts.rs",
                    "crates/application/src/scripts.rs", "crates/infra-postgres/src/maintenance.rs",
                    "crates/scheduler/src/passes.rs"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertEqual(self.chosen(), STEP_LOG_SERVER)
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_the_report_path_that_stores_a_carry_selects_the_step_log_suite_and_the_budget(self):
        for rel in ("crates/application/src/worker.rs", "crates/application/src/worker/claim_counters.rs",
                    "crates/infra-postgres/src/execution.rs", "crates/infra-postgres/src/execution/path.rs"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertEqual(self.chosen(), BUDGET | {"integration_step_logs"})
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_the_worker_protocol_and_each_sdks_capture_code_select_the_step_log_suite_and_the_conformance(self):
        for rel in ("specs/001-platform-foundation/contracts/worker.proto",
                    "sdks/typescript/src/worker/step-logs.ts", "sdks/typescript/src/worker/activity.ts",
                    "sdks/typescript/src/worker/host.ts", "sdks/typescript/src/worker/grpc.ts",
                    "sdks/go/worker/steplogs.go", "sdks/go/worker/activity.go", "sdks/go/worker/task.go",
                    "sdks/go/worker/host.go", "sdks/go/worker/rpc.go", "sdks/go/internal/safelog/safelog.go",
                    "sdks/python/sagat_octopus_orchestrator/worker/_step_logs.py",
                    "sdks/python/sagat_octopus_orchestrator/worker/host.py",
                    "sdks/python/sagat_octopus_orchestrator/worker/activity.py",
                    "sdks/python/sagat_octopus_orchestrator/worker/_grpc.py",
                    "sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/StepLogCapture.cs",
                    "sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerHost.cs",
                    "sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/WorkerRpc.cs",
                    "sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workers/ActivityExecutionContext.cs",
                    "sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Internal/Safe.cs",
                    "sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/StepLogCapture.java",
                    "sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/ActivityLogger.java",
                    "sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/WorkerHost.java",
                    "sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/worker/WorkerRpc.java",
                    "sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/Hooks.java",
                    "sdks/rust/src/step_logs.rs", "sdks/rust/src/activity.rs", "sdks/rust/src/worker.rs",
                    "sdks/rust/src/worker_rpc.rs", "sdks/rust/src/capabilities.rs"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertTrue({"integration_step_logs", "sdk_conformance"} <= self.chosen(), rel)
                self.assertFalse(self.chosen() & (BUDGET | {"contract_platform_foundation"}), rel)
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_other_sdk_files_and_other_contracts_select_no_step_log_suite(self):
        for rel in ("sdks/go/worker/other.go", "sdks/typescript/src/client.ts",
                    "specs/001-platform-foundation/contracts/other.proto", "crates/domain/src/step_logging.rs"):
            with self.subTest(rel=rel):
                self.write(rel, f"// {rel}\n")
                self.commit()
                self.assertNotIn("integration_step_logs", self.chosen(), rel)
                self.base = git(self.root, "rev-parse", "HEAD")

    def test_the_rest_of_the_application_selects_no_suite(self):
        self.write("crates/application/src/schedules.rs", "// x\n")
        self.commit()
        self.assertEqual(self.chosen(), set())

    def test_markdown_of_the_hot_path_selects_nothing(self):
        self.write("crates/infra-postgres/src/README.md", "# x\n")
        self.commit()
        self.assertEqual(self.chosen(), set())

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

    def test_a_change_of_the_canonical_contract_flags_the_studio(self):
        # the Studio's API types are generated from it (`npm run check:api`)
        self.write("specs/001-platform-foundation/contracts/openapi.yaml", "openapi: 3.1.0\n")
        self.commit()
        self.assertEqual(self.decide()["frontend"], "true")

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
        self.assertIn('shards=[{"index": 1, "count": 1, "targets": "contract_rest_a", "seconds": ', text)

    # -- the shard matrix of the integration suites ----------------------------------------------
    def shards(self, **kwargs):
        return json.loads(self.decide(**kwargs)["shards"])

    def test_no_selected_suite_gives_no_shard(self):
        self.write("README.md", "changed\n")
        self.commit()
        decision = self.decide()
        self.assertEqual((decision["any"], decision["shards"]), ("false", "[]"))

    def test_a_selected_suite_runs_in_one_shard(self):
        self.write("tests/fault/b/steps.rs", "// new\n")
        self.commit()
        self.assertEqual([s["targets"] for s in self.shards()], ["fault_b"])

    def test_a_release_spreads_every_suite_over_the_shards_exactly_once(self):
        shards = self.shards(event="workflow_dispatch", base="")
        names = [name for shard in shards for name in shard["targets"].split()]
        self.assertEqual(sorted(names), sorted(TARGETS))
        self.assertEqual([s["index"] for s in shards], list(range(1, len(shards) + 1)))
        self.assertTrue(all(s["count"] == len(shards) for s in shards))

    def test_the_shards_are_balanced_by_the_duration_table_of_the_head(self):
        figures = {**{name: 1 for name in TARGETS}, "integration_a": 1800, "fault_b": 1700}
        self.write("scripts/integration-suite-durations.toml",
                   "[seconds]\n" + "".join(f"{name} = {seconds}\n" for name, seconds in figures.items()))
        self.write("tests/integration/a/x.rs", "// x\n")
        self.write("tests/fault/b/x.rs", "// x\n")
        self.write("tests/contract/a/x.rs", "// x\n")
        self.commit()
        self.assertEqual([s["targets"] for s in self.shards()], ["integration_a", "contract_a fault_b"])

    def test_an_unreadable_duration_table_is_refused(self):
        self.write("scripts/integration-suite-durations.toml", "[seconds\n")
        self.commit()
        result = self.run_select()
        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("integration-suite-durations.toml", result.stderr)


def load_selector():
    spec = importlib.util.spec_from_file_location("select_test_targets", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class RealRepository(unittest.TestCase):
    """The contract readers derived from this repository's own sources (review finding m7, rule 21 of AGENTS.md)."""

    @classmethod
    def setUpClass(cls):
        cls.root = str(SCRIPT.parents[1])
        cls.selector = load_selector()
        cls.targets = cls.selector.head_targets(cls.root)
        cls.readers = cls.selector.Readers(cls.root, cls.targets, cls.selector.Workspace(cls.root))

    def suites(self, rel):
        return {name for kind, name in self.readers.of(rel) if kind == "target"}

    def test_the_canonical_openapi_selects_the_three_suites_of_finding_m7(self):
        self.assertLessEqual(
            {"contract_developer_experience", "contract_payload_classification", "contract_iceberg_writes"},
            self.suites("specs/001-platform-foundation/contracts/openapi.yaml"))

    def test_the_worker_protocol_selects_the_grpc_adapter_that_compiles_it(self):
        self.assertIn(("package", "sagat-api-grpc"),
                      self.readers.of("specs/001-platform-foundation/contracts/worker.proto"))

    def test_only_contract_suites_are_selected_by_a_contract(self):
        for rel in ("specs/001-platform-foundation/contracts/openapi.yaml",
                    "specs/001-platform-foundation/contracts/events.asyncapi.yaml"):
            with self.subTest(rel=rel):
                for name in self.suites(rel):
                    self.assertTrue(self.targets[name].startswith("contract/"), name)

    def test_every_path_into_a_contracts_directory_is_seen_as_a_read(self):
        # A reader that names a contracts directory, or a file in one, and is not found to read it builds the path
        # in a way the selector does not see: a pull request that changes only that contract would not run it.
        path = re.compile(r"(?:^|[/.])(specs/[^/\s]+/contracts)(/[^\s;:,]*)?$")
        unseen = []
        for reader, literals in sorted(self.readers.literals.items()):
            for literal in sorted(literals):
                match = path.search(literal)
                if not match:
                    continue
                directory, rest = match.group(1), (match.group(2) or "").rstrip("/")
                under = pathlib.Path(self.root, directory + rest)
                if under.is_file():
                    files = [directory + rest]
                elif under.is_dir():
                    files = [str(p.relative_to(self.root)) for p in sorted(under.rglob("*")) if p.is_file()]
                else:
                    continue  # a contract the head no longer has; a test of the reader fails on its own
                if not any(reader in self.readers.of(rel) for rel in files):
                    unseen.append(f"{reader[0]} {reader[1]}: {literal}")
        self.assertEqual(unseen, [], "these readers name a contract the selector does not see them read")


class Shards(unittest.TestCase):
    """How the selected integration suites are spread over parallel CI jobs: greedy, longest suite first, into the
    least-loaded shard, with as many shards as the measured time needs (run 44793165198 timed out with every suite in
    one step)."""

    @classmethod
    def setUpClass(cls):
        cls.selector = load_selector()

    def plan(self, names, durations):
        return self.selector.plan_shards(names, durations)

    def test_no_suite_gives_no_shard(self):
        self.assertEqual(self.plan([], {"a": 10}), [])

    def test_one_suite_gives_one_shard(self):
        self.assertEqual(self.plan(["a"], {"a": 10}),
                         [{"index": 1, "count": 1, "targets": "a", "seconds": 10}])

    def test_suites_that_fit_the_budget_together_share_one_shard(self):
        budget = self.selector.SHARD_BUDGET_SECONDS
        plan = self.plan(["a", "b", "c"], {"a": budget // 2, "b": budget // 4, "c": budget // 8})
        self.assertEqual([s["targets"] for s in plan], ["a b c"])

    def test_the_longest_suite_goes_first_into_the_least_loaded_shard(self):
        budget = self.selector.SHARD_BUDGET_SECONDS
        durations = {"a": budget, "b": 1000, "c": 900, "d": 800, "e": 100}
        plan = self.plan(sorted(durations), durations)
        self.assertEqual([(s["targets"], s["seconds"]) for s in plan],
                         [("a", budget), ("b e", 1100), ("c d", 1700)])
        self.assertTrue(all(s["count"] == 3 for s in plan))

    def test_the_plan_does_not_depend_on_the_order_of_the_selection(self):
        durations = {"a": 700, "b": 700, "c": 700, "d": 500, "e": 500, "f": 0, "g": 0}
        first = self.plan(sorted(durations), durations)
        second = self.plan(sorted(durations, reverse=True), durations)
        self.assertEqual(first, second)

    def test_suites_with_no_measured_time_are_spread_and_not_piled_into_one_shard(self):
        budget = self.selector.SHARD_BUDGET_SECONDS
        durations = {"a": budget - 100, "b": budget - 100, **{f"z{i}": 0 for i in range(6)}}
        plan = self.plan(sorted(durations), durations)
        empty = [len([n for n in s["targets"].split() if n.startswith("z")]) for s in plan]
        self.assertEqual(empty, [3, 3])

    def test_a_suite_missing_from_the_table_counts_as_the_conservative_default(self):
        default = self.selector.DEFAULT_SUITE_SECONDS
        self.assertGreaterEqual(default, 600)
        self.assertEqual(self.plan(["new"], {}), [{"index": 1, "count": 1, "targets": "new", "seconds": default}])

    def test_a_shard_is_added_while_the_greedy_plan_overflows_the_budget(self):
        # Three suites of two thirds of the budget each need three shards, not the two that their sum suggests.
        budget = self.selector.SHARD_BUDGET_SECONDS
        third = budget // 3
        durations = {"a": 2 * third, "b": 2 * third, "c": 2 * third}
        plan = self.plan(sorted(durations), durations)
        self.assertEqual([s["targets"] for s in plan], ["a", "b", "c"])

    def test_the_number_of_shards_is_capped(self):
        budget = self.selector.SHARD_BUDGET_SECONDS
        cap = self.selector.MAX_SHARDS
        durations = {f"s{i:02d}": budget for i in range(cap * 2)}
        plan = self.plan(sorted(durations), durations)
        self.assertEqual(len(plan), cap)
        self.assertEqual(sorted(n for s in plan for n in s["targets"].split()), sorted(durations))


class DurationTable(unittest.TestCase):
    """The checked-in table the shards are balanced by must name exactly the suites of tests/Cargo.toml (AGENTS.md rule
    21): a suite added without a figure would be planned at the default, and a figure left for a removed suite is stale."""

    @classmethod
    def setUpClass(cls):
        cls.root = str(SCRIPT.parents[1])
        cls.selector = load_selector()
        cls.targets = cls.selector.head_targets(cls.root)
        cls.durations = cls.selector.load_durations(cls.root)

    def test_the_table_names_exactly_the_suites_of_the_manifest(self):
        self.assertEqual(sorted(set(self.targets) - set(self.durations)), [], "suites with no measured duration")
        self.assertEqual(sorted(set(self.durations) - set(self.targets)), [], "durations of suites that do not exist")

    def test_every_figure_is_a_whole_number_of_seconds(self):
        for name, seconds in self.durations.items():
            self.assertTrue(isinstance(seconds, int) and not isinstance(seconds, bool) and seconds >= 0, name)

    def test_no_suite_is_longer_than_a_shard_may_be(self):
        # A suite longer than the budget would make its shard the bottleneck whatever the plan: split the suite, or
        # raise the budget knowingly, with the job's timeout in view.
        budget = self.selector.SHARD_BUDGET_SECONDS
        self.assertEqual({n: s for n, s in self.durations.items() if s > budget}, {})

    def test_every_suite_runs_once_and_no_shard_overflows_for_a_release(self):
        plan = self.selector.plan_shards(sorted(self.targets), self.durations)
        self.assertEqual(sorted(n for s in plan for n in s["targets"].split()), sorted(self.targets))
        self.assertLessEqual(len(plan), self.selector.MAX_SHARDS)
        budget = self.selector.SHARD_BUDGET_SECONDS
        self.assertTrue(all(s["seconds"] <= budget for s in plan), [s["seconds"] for s in plan])


if __name__ == "__main__":
    unittest.main()
