"""Tests for scripts/next-feature-id.py (run: python3 -m unittest discover scripts/tests)."""
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "next-feature-id.py"
GIT_ENV = {
    **os.environ,
    "GIT_CONFIG_GLOBAL": os.devnull,
    "GIT_CONFIG_NOSYSTEM": "1",
}
GIT = ["git", "-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"]


def git(repo, *args):
    result = subprocess.run(
        [*GIT, *args], cwd=repo, env=GIT_ENV, capture_output=True, text=True, check=True
    )
    return result.stdout.strip()


def touch(repo, rel):
    path = pathlib.Path(repo) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("x")


class Repo:
    """A throwaway repository with helpers to commit paths on branches and remote refs."""

    def __init__(self, root):
        self.root = pathlib.Path(root)
        git(self.root, "init", "-q", "-b", "develop")
        touch(self.root, "README.md")
        git(self.root, "add", "-A")
        git(self.root, "commit", "-q", "-m", "init")

    def commit_on(self, ref, *paths):
        """Create refs/<ref> holding the given extra paths, without touching the working tree."""
        clone = pathlib.Path(tempfile.mkdtemp(prefix="nfi-"))
        try:
            git(self.root, "worktree", "add", "-q", "--detach", str(clone), "develop")
            for rel in paths:
                touch(clone, rel)
            git(clone, "add", "-A")
            git(clone, "commit", "-q", "-m", "paths")
            sha = git(clone, "rev-parse", "HEAD")
        finally:
            git(self.root, "worktree", "remove", "--force", str(clone))
        git(self.root, "update-ref", f"refs/{ref}", sha)


class _Dir:
    def __init__(self, root):
        self.root = pathlib.Path(root)


def run(repo, *args):
    result = subprocess.run(
        [sys.executable, str(SCRIPT), "--root", str(repo.root), *args],
        capture_output=True,
        text=True,
        env=GIT_ENV,
    )
    return result.returncode, result.stdout, result.stderr


def fresh_fetch_head(repo):
    path = repo.root / ".git" / "FETCH_HEAD"
    path.write_text("")
    return path


class NextFeatureId(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.repo = Repo(self._tmp.name)
        fresh_fetch_head(self.repo)

    def test_an_empty_tree_starts_at_001(self):
        code, out, _ = run(self.repo)
        self.assertEqual((code, out), (0, "001\n"))

    def test_working_tree_specs_and_designs_are_counted(self):
        touch(self.repo.root, "specs/003-a/spec.md")
        touch(self.repo.root, "docs/designs/005-b.md")
        code, out, _ = run(self.repo)
        self.assertEqual((code, out), (0, "006\n"))

    def test_a_reviews_directory_of_a_design_is_counted(self):
        touch(self.repo.root, "docs/designs/012-b.reviews/2026-11-03/x.md")
        self.assertEqual(run(self.repo)[1], "013\n")

    def test_a_number_taken_only_on_another_local_branch_is_counted(self):
        self.repo.commit_on("heads/feature/040-other", "specs/040-other/spec.md")
        self.assertEqual(run(self.repo)[1], "041\n")

    def test_a_number_taken_only_on_a_remote_branch_is_counted(self):
        self.repo.commit_on("remotes/origin/feature/052-remote", "docs/designs/052-remote.md")
        self.assertEqual(run(self.repo)[1], "053\n")

    def test_refs_outside_heads_and_origin_are_ignored(self):
        self.repo.commit_on("remotes/upstream/feature/070-x", "specs/070-x/spec.md")
        self.repo.commit_on("tags/v9", "specs/071-x/spec.md")
        self.assertEqual(run(self.repo)[1], "001\n")

    def test_the_result_is_zero_padded_to_three_digits(self):
        touch(self.repo.root, "specs/008-a/spec.md")
        self.assertEqual(run(self.repo)[1], "009\n")

    def test_a_number_of_1000_or_more_is_ignored_with_a_warning(self):
        touch(self.repo.root, "specs/1000-odd/spec.md")
        touch(self.repo.root, "specs/004-a/spec.md")
        code, out, err = run(self.repo)
        self.assertEqual((code, out), (0, "005\n"))
        self.assertIn("1000", err)

    def test_names_without_a_leading_number_are_ignored(self):
        touch(self.repo.root, "specs/README.md")
        touch(self.repo.root, "specs/AGENTS.md")
        touch(self.repo.root, "docs/designs/TEMPLATE.md")
        touch(self.repo.root, "docs/designs/constitution-4.0.0-egm.md")
        self.assertEqual(run(self.repo)[1], "001\n")

    def test_a_ref_name_outside_the_allowed_pattern_is_not_printed(self):
        self.repo.commit_on("heads/feature/050-a;touch$IFS", "specs/050-x/spec.md")
        code, out, err = run(self.repo)
        self.assertEqual((code, out), (0, "051\n"))
        self.assertNotIn(";touch", err)
        self.assertNotIn(";touch", out)

    def test_a_missing_fetch_head_warns_and_exits_0(self):
        (self.repo.root / ".git" / "FETCH_HEAD").unlink()
        code, out, err = run(self.repo)
        self.assertEqual((code, out), (0, "001\n"))
        self.assertIn("git fetch", err)

    def test_a_stale_fetch_head_warns_and_exits_0(self):
        old = time.time() - 2 * 86400
        os.utime(self.repo.root / ".git" / "FETCH_HEAD", (old, old))
        code, out, err = run(self.repo)
        self.assertEqual((code, out), (0, "001\n"))
        self.assertIn("git fetch", err)

    def test_a_fresh_fetch_head_does_not_warn(self):
        code, _, err = run(self.repo)
        self.assertEqual(code, 0)
        self.assertNotIn("git fetch", err)

    def test_a_missing_fetch_head_in_a_worktree_warns_and_exits_0(self):
        wt = pathlib.Path(self._tmp.name + "-wt")
        git(self.repo.root, "worktree", "add", "-q", "-b", "feature/009-x", str(wt))
        self.addCleanup(lambda: git(self.repo.root, "worktree", "remove", "--force", str(wt)))
        code, out, err = run(_Dir(wt))
        self.assertEqual(code, 0)
        self.assertEqual(out, "001\n")
        self.assertIn("git fetch", err)

    def test_a_name_with_a_huge_number_is_ignored_without_a_crash(self):
        touch(self.repo.root, "docs/designs/" + "9" * 200 + "-x.md")
        touch(self.repo.root, "specs/004-a/spec.md")
        code, out, _ = run(self.repo)
        self.assertEqual((code, out), (0, "005\n"))

    def test_non_ascii_digits_are_not_numbers(self):
        touch(self.repo.root, "specs/\u0663\u0663\u0663-x/spec.md")
        self.assertEqual(run(self.repo)[1], "001\n")

    def test_999_taken_exits_1(self):
        touch(self.repo.root, "specs/999-last/spec.md")
        code, out, _ = run(self.repo)
        self.assertEqual((code, out), (1, ""))

    def test_outside_a_git_repository_exits_2(self):
        with tempfile.TemporaryDirectory() as plain:
            code, out, _ = run(_Dir(plain))
            self.assertEqual(code, 2)
            self.assertEqual(out, "")


if __name__ == "__main__":
    unittest.main()
