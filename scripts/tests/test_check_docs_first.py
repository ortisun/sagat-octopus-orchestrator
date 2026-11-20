"""Tests for scripts/check-docs-first.py (run: python3 -m unittest discover scripts/tests).

The rule (docs/constitution.md 4.7.0, Documentation first): a pull request whose own commits change behaviour code and
no document fails, unless each such commit carries the footer `Docs: none, <reason>`. Commits whose tree does not
contain the check were written before the rule and are not judged.

Every case builds a throwaway git repository: a base commit on `develop` that holds the check, then a branch with the
commits of a pull request, and runs the check over `develop..branch`.
"""
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check-docs-first.py"
RULE_PATH = "scripts/check-docs-first.py"
GIT_ENV = {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]


class Repo:
    def __init__(self, root, with_rule=True):
        self.root = root
        self.git("init", "-q", "-b", "develop")
        self.write("README.md", "# repo\n")
        self.write("crates/domain/src/lib.rs", "pub fn one() {}\n")
        self.write("docs/guide.md", "# guide\n")
        if with_rule:
            self.write(RULE_PATH, "# the check\n")
        self.commit("base")
        self.git("checkout", "-q", "-b", "feature/x")

    def git(self, *args):
        return subprocess.run([*GIT, *args], cwd=self.root, env={**os.environ, **GIT_ENV},
                              capture_output=True, text=True, check=True).stdout.strip()

    def write(self, rel, text):
        path = pathlib.Path(self.root, rel)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def commit(self, message, files=None):
        for rel, text in (files or {}).items():
            self.write(rel, text)
        self.git("add", "-A")
        self.git("commit", "-q", "--allow-empty", "-m", message)
        return self.git("rev-parse", "HEAD")

    def run(self, base="develop", head="HEAD"):
        result = subprocess.run([sys.executable, str(SCRIPT), "--root", self.root, "--base", base, "--head", head],
                                capture_output=True, text=True)
        return result.returncode, result.stdout + result.stderr


class DocsFirst(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.repo = Repo(self._tmp.name)

    def assertPasses(self):
        code, out = self.repo.run()
        self.assertEqual(code, 0, out)
        return out

    def assertFails(self):
        code, out = self.repo.run()
        self.assertEqual(code, 1, out)
        return out

    # -- what passes -------------------------------------------------------------------------------------------------
    def test_a_pull_request_with_no_commit_passes(self):
        self.assertIn("no behaviour code", self.assertPasses())

    def test_a_change_of_documents_only_passes(self):
        self.repo.commit("docs: guide", {"docs/guide.md": "# guide\nmore\n"})
        self.assertPasses()

    def test_code_with_a_document_in_the_same_commit_passes(self):
        self.repo.commit("feat: one", {"crates/domain/src/lib.rs": "pub fn two() {}\n", "docs/guide.md": "# two\n"})
        out = self.assertPasses()
        self.assertNotIn("warning", out)

    def test_code_after_a_commit_that_changed_the_document_passes_without_a_warning(self):
        self.repo.commit("docs: two", {"specs/001-x/spec.md": "# two\n"})
        self.repo.commit("feat: two", {"crates/domain/src/lib.rs": "pub fn two() {}\n"})
        out = self.assertPasses()
        self.assertNotIn("warning", out)

    def test_code_before_the_document_passes_with_a_warning_that_names_the_commit(self):
        sha = self.repo.commit("feat: two", {"crates/domain/src/lib.rs": "pub fn two() {}\n"})
        self.repo.commit("docs: two", {"docs/guide.md": "# two\n"})
        out = self.assertPasses()
        self.assertIn("warning", out)
        self.assertIn(sha[:12], out)

    def test_every_behaviour_area_is_code(self):
        for rel in ("apps/server/src/main.rs", "sdks/go/client.go", "runners/lake/run.py", "frontend/apps/studio/src/a.ts",
                    "infra/docker/compose.yaml", "crates/infra-postgres/migrations/0001_init.sql",
                    "tools/migrations/0002.sql"):
            with self.subTest(rel=rel):
                self.setUp()
                self.repo.commit("feat: x", {rel: "x\n"})
                out = self.assertFails()
                self.assertIn(rel, out)

    def test_markdown_tests_and_scripts_outside_the_behaviour_areas_are_not_code(self):
        self.repo.commit("docs: readme", {"crates/domain/README.md": "# domain\n", "sdks/go/AGENTS.md": "# go\n"})
        self.repo.commit("test: more", {"tests/integration/a.rs": "// test\n", "scripts/x.py": "# x\n", "frontend/e2e/a.ts": "// e2e\n"})
        self.assertPasses()

    # -- what fails, and the exemption -------------------------------------------------------------------------------
    def test_code_without_a_document_fails_and_names_the_commit_and_the_file(self):
        sha = self.repo.commit("feat: two", {"crates/domain/src/lib.rs": "pub fn two() {}\n"})
        out = self.assertFails()
        self.assertIn(sha[:12], out)
        self.assertIn("crates/domain/src/lib.rs", out)
        self.assertIn("Docs: none, <reason>", out, "the failure says how to exempt a commit")

    def test_a_change_outside_docs_and_specs_is_not_a_document(self):
        self.repo.commit("feat: two", {"crates/domain/src/lib.rs": "pub fn two() {}\n", "README.md": "# two\n", "crates/domain/AGENTS.md": "# two\n"})
        self.assertFails()

    def test_a_folder_that_only_starts_like_docs_or_specs_is_not_a_document(self):
        self.repo.commit("feat: two", {"crates/domain/src/lib.rs": "pub fn two() {}\n", "docs-site/a.md": "# a\n",
                                       "specs.md": "# a\n", "frontend/apps/docs/a.md": "# a\n"})
        self.assertFails()

    def test_every_code_commit_carrying_the_footer_passes_and_the_reasons_are_printed(self):
        self.repo.commit("refactor: rename\n\nDocs: none, a rename with no change of behaviour", {"crates/domain/src/lib.rs": "pub fn uno() {}\n"})
        self.repo.commit("chore: bump\n\nRefs: Task 001-001\nDocs: none, dependency bump", {"sdks/go/go.mod": "module x\n"})
        out = self.assertPasses()
        self.assertIn("a rename with no change of behaviour", out)
        self.assertIn("dependency bump", out)

    def test_one_code_commit_without_the_footer_fails_even_when_another_has_it(self):
        self.repo.commit("refactor: a\n\nDocs: none, a rename", {"crates/domain/src/lib.rs": "pub fn a() {}\n"})
        sha = self.repo.commit("feat: b", {"crates/domain/src/lib.rs": "pub fn b() {}\n"})
        out = self.assertFails()
        self.assertIn(sha[:12], out)

    def test_a_footer_without_a_reason_is_not_an_exemption(self):
        for message in ("feat: a\n\nDocs: none,", "feat: a\n\nDocs: none, ", "feat: a\n\nDocs: none",
                        "feat: a\n\nDocs: none, -", "feat: a\n\nsee Docs: none, inline"):
            with self.subTest(message=message):
                self.setUp()
                self.repo.commit(message, {"crates/domain/src/lib.rs": "pub fn a() {}\n"})
                self.assertFails()

    def test_a_commit_that_changes_no_code_needs_no_footer(self):
        self.repo.commit("refactor: a\n\nDocs: none, a rename", {"crates/domain/src/lib.rs": "pub fn a() {}\n"})
        self.repo.commit("test: b", {"tests/b.rs": "// b\n"})
        self.assertPasses()

    # -- which commits are judged ------------------------------------------------------------------------------------
    def test_commits_written_before_the_check_existed_are_not_judged(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        repo = Repo(tmp.name, with_rule=False)
        repo.commit("feat: old", {"crates/domain/src/lib.rs": "pub fn old() {}\n"})
        code, out = repo.run()
        self.assertEqual(code, 0, out)
        self.assertIn("written before the check", out)

    def test_a_commit_after_the_branch_merges_the_rule_is_judged(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        repo = Repo(tmp.name, with_rule=False)
        old = repo.commit("feat: old", {"crates/domain/src/lib.rs": "pub fn old() {}\n"})
        repo.git("checkout", "-q", "develop")
        repo.commit("ci: the check", {RULE_PATH: "# the check\n"})
        repo.git("checkout", "-q", "feature/x")
        repo.git("merge", "-q", "--no-edit", "develop")
        sha = repo.commit("feat: new", {"apps/server/main.rs": "fn main() {}\n"})
        code, out = repo.run()
        self.assertEqual(code, 1, out)
        self.assertIn(sha[:12], out)
        self.assertNotIn(old[:12], out, "a commit written before the check is not judged")

    def test_a_document_in_a_commit_written_before_the_check_counts(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        repo = Repo(tmp.name, with_rule=False)
        repo.commit("docs: old", {"docs/guide.md": "# old\n"})
        repo.git("checkout", "-q", "develop")
        repo.commit("ci: the check", {RULE_PATH: "# the check\n"})
        repo.git("checkout", "-q", "feature/x")
        repo.git("merge", "-q", "--no-edit", "develop")
        repo.commit("feat: new", {"apps/server/main.rs": "fn main() {}\n"})
        code, out = repo.run()
        self.assertEqual(code, 0, out)

    def test_merge_commits_are_not_judged(self):
        self.repo.git("checkout", "-q", "develop")
        self.repo.commit("feat: on develop", {"crates/domain/src/lib.rs": "pub fn d() {}\n", "docs/guide.md": "# d\n"})
        self.repo.git("checkout", "-q", "feature/x")
        self.repo.commit("docs: mine", {"specs/a.md": "# a\n"})
        self.repo.git("merge", "-q", "--no-edit", "develop")
        self.assertPasses()

    def test_only_the_commits_of_the_pull_request_are_read(self):
        self.repo.git("checkout", "-q", "develop")
        self.repo.commit("feat: on develop without docs", {"crates/domain/src/lib.rs": "pub fn d() {}\n"})
        self.repo.git("checkout", "-q", "feature/x")
        self.repo.commit("docs: mine", {"docs/guide.md": "# mine\n"})
        self.assertPasses()

    # -- usage -------------------------------------------------------------------------------------------------------
    def test_an_unknown_base_is_an_error_not_a_pass(self):
        code, out = self.repo.run(base="no-such-ref")
        self.assertEqual(code, 2, out)


if __name__ == "__main__":
    unittest.main()
