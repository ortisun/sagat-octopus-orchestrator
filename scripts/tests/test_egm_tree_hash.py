"""Tests for scripts/egm-tree-hash.py (run: python3 -m unittest discover scripts/tests)."""
import hashlib
import os
import pathlib
import subprocess
import sys
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "egm-tree-hash.py"


def write(root, rel, data: "bytes | str" = b"x"):
    path = pathlib.Path(root) / rel
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data if isinstance(data, bytes) else data.encode())
    return path


def make_plugin(root):
    write(root, ".claude-plugin/plugin.json", '{"name": "egm"}')
    write(root, ".claude-plugin/agents/synthesizer.md", "agent")
    write(root, "skills/egm-review/SKILL.md", "review")
    write(root, "README.md", "readme")
    return root


def run(plugin_dir):
    result = subprocess.run(
        [sys.executable, str(SCRIPT), str(plugin_dir)], capture_output=True, text=True
    )
    return result.returncode, result.stdout.strip(), result.stderr


class TreeHash(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        self.root = make_plugin(pathlib.Path(self._tmp.name) / "plugin")

    def hash(self):
        code, out, err = run(self.root)
        self.assertEqual(code, 0, err)
        self.assertRegex(out, r"^[0-9a-f]{64}$")
        return out

    def test_same_tree_gives_same_hash(self):
        self.assertEqual(self.hash(), self.hash())

    def test_hash_matches_the_documented_definition(self):
        lines = []
        for rel in sorted(
            [".claude-plugin/plugin.json", ".claude-plugin/agents/synthesizer.md",
             "skills/egm-review/SKILL.md", "README.md"]
        ):
            digest = hashlib.sha256((self.root / rel).read_bytes()).hexdigest()
            lines.append(f"{rel}\0{digest}\n")
        expected = hashlib.sha256("".join(lines).encode()).hexdigest()
        self.assertEqual(self.hash(), expected)

    def test_a_changed_byte_changes_the_hash(self):
        before = self.hash()
        write(self.root, "skills/egm-review/SKILL.md", "reviex")
        self.assertNotEqual(self.hash(), before)

    def test_a_changed_claude_code_adapter_changes_the_hash(self):
        before = self.hash()
        write(self.root, ".claude-plugin/agents/synthesizer.md", "agent with Bash")
        self.assertNotEqual(self.hash(), before)

    def test_a_new_hooks_file_changes_the_hash(self):
        before = self.hash()
        write(self.root, "hooks/hooks.json", "{}")
        self.assertNotEqual(self.hash(), before)

    def test_a_renamed_file_changes_the_hash(self):
        before = self.hash()
        (self.root / "README.md").rename(self.root / "README2.md")
        self.assertNotEqual(self.hash(), before)

    def test_excluded_paths_do_not_change_the_hash(self):
        before = self.hash()
        for rel in (
            ".git/HEAD", ".github/workflows/x.yml", ".in_use", "site/index.html",
            "tests/test_x.py", "distributions/a.zip", "skills/x/assets/logo.png",
            "assets/logo.png", "skills/x/TEST.md", "skills/x/test_scenarios.yaml",
            "scripts/__pycache__/x.pyc",
        ):
            write(self.root, rel, "changed")
        self.assertEqual(self.hash(), before)

    def test_a_path_that_only_resembles_an_exclusion_is_hashed(self):
        before = self.hash()
        write(self.root, "skills/x/TEST.md.bak", "not excluded")
        self.assertNotEqual(self.hash(), before)
        before = self.hash()
        write(self.root, "contest/a.md", "not tests/")
        self.assertNotEqual(self.hash(), before)

    def test_missing_plugin_json_exits_2(self):
        (self.root / ".claude-plugin" / "plugin.json").unlink()
        code, out, err = run(self.root)
        self.assertEqual(code, 2)
        self.assertEqual(out, "")
        self.assertIn("plugin.json", err)

    def test_missing_directory_exits_2(self):
        code, _, _ = run(self.root / "nope")
        self.assertEqual(code, 2)

    def test_symlinks_are_refused(self):
        os.symlink("/etc/hosts", self.root / "link.md")
        code, out, err = run(self.root)
        self.assertEqual(code, 2)
        self.assertEqual(out, "")
        self.assertIn("link.md", err)

    def test_the_marketplace_clone_layout_is_not_special(self):
        # The hash only depends on the files under the directory it is given.
        other = make_plugin(pathlib.Path(self._tmp.name) / "elsewhere")
        self.assertEqual(run(other)[1], self.hash())


if __name__ == "__main__":
    unittest.main()
