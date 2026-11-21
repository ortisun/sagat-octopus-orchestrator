"""Tests for scripts/check-docs-index.py (run: python3 -m unittest discover scripts/tests).

The rule (docs/README.md, "How this folder is kept in order"): every folder of docs/ has a README.md index, every index
links every file and every folder directly inside its folder, and docs/README.md links every folder of docs/. Review
reports (a folder whose name ends in `.reviews`, and everything under it) are evidence of a design doc, not documents to
browse, and need no index.

Every case builds a throwaway git repository with a small docs/ tree and runs the check on it.
"""
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check-docs-index.py"
GIT_ENV = {"GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]
TREE = {
    "docs/README.md": "# Docs\n\n- [ADRs](adr/README.md)\n- [Runbooks](runbooks/)\n- [Constitution](constitution.md)\n"
                      "- ![hero](hero.png)\n",
    "docs/constitution.md": "# Constitution\n",
    "docs/hero.png": "png",
    "docs/adr/README.md": "# ADRs\n\n| Decision |\n|---|\n| [ADR-0001](ADR-0001-x.md#status) |\n| [Template](./TEMPLATE.md) |\n",
    "docs/adr/ADR-0001-x.md": "# ADR-0001\n",
    "docs/adr/TEMPLATE.md": "# Template\n",
    "docs/runbooks/README.md": "# Runbooks\n\n[Alerts](alerts.md) and [the constitution](../constitution.md).\n",
    "docs/runbooks/alerts.md": "# Alerts\n",
}


class Repo:
    def __init__(self, root, tree):
        self.root = root
        self.git("init", "-q", "-b", "develop")
        for rel, text in tree.items():
            self.write(rel, text)
        self.git("add", "-A")
        self.git("commit", "-q", "-m", "base")

    def git(self, *args):
        return subprocess.run([*GIT, *args], cwd=self.root, env={**os.environ, **GIT_ENV},
                              capture_output=True, text=True, check=True).stdout.strip()

    def write(self, rel, text):
        path = pathlib.Path(self.root, rel)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def remove(self, rel):
        pathlib.Path(self.root, rel).unlink()

    def run(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "--root", self.root], capture_output=True, text=True)
        return result.returncode, result.stdout + result.stderr


class DocsIndex(unittest.TestCase):
    def repo(self, tree=TREE):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        return Repo(tmp.name, tree)

    def assertPasses(self, repo):
        code, out = repo.run()
        self.assertEqual(code, 0, out)
        return out

    def assertFails(self, repo, *needles):
        code, out = repo.run()
        self.assertEqual(code, 1, out)
        for needle in needles:
            self.assertIn(needle, out)
        return out

    def test_a_complete_tree_passes(self):
        self.assertPasses(self.repo())

    def test_a_folder_without_an_index_fails(self):
        repo = self.repo()
        repo.write("docs/guides/connect.md", "# Connect\n")
        self.assertFails(repo, "docs/guides", "has no README.md")

    def test_a_file_the_index_does_not_link_fails(self):
        repo = self.repo()
        repo.write("docs/runbooks/scaling.md", "# Scaling\n")
        self.assertFails(repo, "docs/runbooks/README.md", "scaling.md")

    def test_a_file_named_only_in_text_or_code_is_not_linked(self):
        repo = self.repo()
        repo.write("docs/runbooks/scaling.md", "# Scaling\n")
        repo.write("docs/runbooks/README.md", "# Runbooks\n\n[Alerts](alerts.md). See scaling.md and `scaling.md`.\n")
        self.assertFails(repo, "scaling.md")

    def test_a_link_inside_a_code_span_does_not_count(self):
        repo = self.repo()
        repo.write("docs/runbooks/README.md", "# Runbooks\n\nWrite `[Alerts](alerts.md)` to link it.\n")
        self.assertFails(repo, "alerts.md")

    def test_a_link_inside_a_code_fence_does_not_count(self):
        repo = self.repo()
        repo.write("docs/runbooks/README.md", "# Runbooks\n\n```md\n[Alerts](alerts.md)\n```\n")
        self.assertFails(repo, "alerts.md")

    def test_a_top_level_file_docs_readme_does_not_link_fails(self):
        repo = self.repo()
        repo.write("docs/glossary.md", "# Glossary\n")
        self.assertFails(repo, "docs/README.md", "glossary.md")

    def test_a_folder_docs_readme_does_not_link_fails_even_with_its_own_index(self):
        repo = self.repo()
        repo.write("docs/guides/README.md", "# Guides\n\n[Connect](connect.md)\n")
        repo.write("docs/guides/connect.md", "# Connect\n")
        self.assertFails(repo, "docs/README.md", "guides")

    def test_a_folder_may_be_linked_by_its_path_with_or_without_a_slash_or_by_its_index(self):
        tree = dict(TREE)
        tree["docs/README.md"] = ("# Docs\n\n[ADRs](adr) [Runbooks](./runbooks/README.md#top) "
                                  "[Constitution](constitution.md) <img src=\"hero.png\">\n")
        self.assertPasses(self.repo(tree))

    def test_reference_style_links_count(self):
        repo = self.repo()
        repo.write("docs/runbooks/scaling.md", "# Scaling\n")
        repo.write("docs/runbooks/README.md", "# Runbooks\n\n[Alerts](alerts.md), [scaling][s].\n\n[s]: scaling.md\n")
        self.assertPasses(repo)

    def test_a_link_to_a_file_elsewhere_does_not_cover_a_file_here(self):
        repo = self.repo()
        repo.write("docs/runbooks/README.md", "# Runbooks\n\n[Alerts](../adr/alerts.md)\n")
        self.assertFails(repo, "alerts.md")

    def test_review_reports_need_no_index_and_their_folder_need_not_be_linked(self):
        repo = self.repo()
        repo.write("docs/adr/ADR-0001-x.reviews/2026-11-20/architecture.md", "# Review\n")
        self.assertPasses(repo)

    def test_an_untracked_file_is_checked_too(self):
        repo = self.repo()
        repo.write("docs/runbooks/new.md", "# New\n")
        self.assertFails(repo, "new.md")

    def test_a_link_that_resolves_nowhere_is_reported(self):
        repo = self.repo()
        repo.write("docs/runbooks/README.md", "# Runbooks\n\n[Alerts](alerts.md) [Gone](gone.md)\n")
        self.assertFails(repo, "gone.md", "does not exist")

    def test_a_link_out_of_docs_is_checked_on_disk(self):
        repo = self.repo()
        repo.write("examples/hello/README.md", "# Hello\n")
        repo.write("docs/runbooks/README.md", "# Runbooks\n\n[Alerts](alerts.md) [Hello](../../examples/hello)\n")
        self.assertPasses(repo)
        repo.write("docs/runbooks/README.md", "# Runbooks\n\n[Alerts](alerts.md) [Gone](../../examples/gone)\n")
        self.assertFails(repo, "../../examples/gone", "does not exist")

    def test_external_links_and_anchors_are_not_files(self):
        repo = self.repo()
        repo.write("docs/runbooks/README.md",
                   "# Runbooks\n\n[Alerts](alerts.md) [web](https://example.org/x.md) [top](#top) [mail](mailto:a@b)\n")
        self.assertPasses(repo)

    def test_no_docs_folder_is_an_error(self):
        repo = self.repo({"README.md": "# x\n"})
        code, out = repo.run()
        self.assertEqual(code, 2, out)


if __name__ == "__main__":
    unittest.main()
