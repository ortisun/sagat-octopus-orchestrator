"""Regression tests for scripts/check-agent-config.py (run: python3 -m unittest discover scripts/tests)."""
import importlib.util
import json
import pathlib
import shutil
import tempfile
import unittest

SCRIPT = pathlib.Path(__file__).resolve().parents[1] / "check-agent-config.py"
spec = importlib.util.spec_from_file_location("check_agent_config", SCRIPT)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

AGENTS = "# Agents\n\n1. First.\n2. Second.\n"
SKILL = '---\nname: demo\ndescription: "Does a thing. Use when asked."\n---\n\nBody.\n'
REVIEWER = "---\nname: x-reviewer\ndescription: Reviews.\ntools: Read, Grep, Glob\n---\n\nBody.\n"
ADR = "# ADR-0001: Title\n\n**Status:** Accepted\n"


class Repo:
    def __init__(self):
        self.root = pathlib.Path(tempfile.mkdtemp())

    def write(self, rel: str, text: str, mode: int = 0o644):
        path = self.root / rel
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        path.chmod(mode)
        return path

    def good(self):
        self.write("AGENTS.md", AGENTS)
        self.write("CLAUDE.md", "@AGENTS.md\n\nMore.\n")
        self.write(".claude/skills/demo/SKILL.md", SKILL)
        self.write(".claude/agents/x-reviewer.md", REVIEWER)
        self.write(".claude/hooks/h.py", "#!/usr/bin/env python3\n", 0o755)
        self.write(".claude/settings.json", json.dumps({
            "permissions": {"deny": ["Read(./.env)"]},
            "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/h.py"}]}]},
        }))
        self.write("docs/adr/ADR-0001-title.md", ADR)
        return self

    def errors(self):
        return module.check(self.root)

    def close(self):
        shutil.rmtree(self.root, ignore_errors=True)


class CheckAgentConfig(unittest.TestCase):
    def setUp(self):
        self.repo = Repo().good()
        self.addCleanup(self.repo.close)

    def has(self, fragment):
        self.assertTrue(any(fragment in e for e in self.repo.errors()), self.repo.errors())

    def test_a_sound_configuration_passes(self):
        self.assertEqual(self.repo.errors(), [])

    def test_root_agents_over_budget_fails(self):
        self.repo.write("AGENTS.md", AGENTS + "x\n" * 200)
        self.has("budget")

    def test_a_gap_in_the_rule_numbers_fails(self):
        self.repo.write("AGENTS.md", "1. a\n3. c\n")
        self.has("without gaps")

    def test_claude_md_must_import_agents(self):
        self.repo.write("CLAUDE.md", "No import here\n")
        self.has("@AGENTS.md")

    def test_nested_claude_must_be_only_the_import_and_have_a_sibling(self):
        self.repo.write("crates/x/CLAUDE.md", "@AGENTS.md\nextra\n")
        self.has("contains only")
        self.has("no AGENTS.md next to it")

    def test_a_nested_agents_without_the_claude_stub_fails(self):
        self.repo.write("crates/x/AGENTS.md", "# x\n")
        self.has("add a CLAUDE.md")

    def test_skill_name_must_match_its_directory(self):
        self.repo.write(".claude/skills/demo/SKILL.md", SKILL.replace("name: demo", "name: other"))
        self.has("name must equal")

    def test_a_quoted_value_that_is_several_strings_is_not_valid_yaml(self):
        self.repo.write(".claude/skills/demo/SKILL.md", SKILL.replace('description: "Does a thing. Use when asked."', 'description: "a", "b"'))
        self.has("not valid YAML")

    def test_skill_without_description_fails(self):
        self.repo.write(".claude/skills/demo/SKILL.md", "---\nname: demo\n---\nBody\n")
        self.has("description is required")

    def test_a_reviewer_with_write_tools_fails(self):
        self.repo.write(".claude/agents/x-reviewer.md", REVIEWER.replace("Read, Grep, Glob", "Read, Edit"))
        self.has("must not have")

    def test_a_reviewer_without_an_allowlist_fails(self):
        self.repo.write(".claude/agents/x-reviewer.md", "---\nname: x-reviewer\ndescription: R.\n---\nB\n")
        self.has("allowlist")

    def test_a_missing_or_non_executable_hook_fails(self):
        self.repo.write(".claude/hooks/h.py", "#!/usr/bin/env python3\n", 0o644)
        self.has("not executable")
        (self.repo.root / ".claude/hooks/h.py").unlink()
        self.has("does not exist")

    def test_settings_must_deny_env_files(self):
        self.repo.write(".claude/settings.json", json.dumps({"permissions": {"deny": []}}))
        self.has("deny reading .env")

    def test_adr_without_title_or_with_duplicate_number_fails(self):
        self.repo.write("docs/adr/ADR-0002-bad.md", "Not a title\n**Status:** x\n")
        self.has("must start with")
        self.repo.write("docs/adr/ADR-0001-again.md", ADR)
        self.has("also used by")

    def test_adr_dash_style_and_plain_status_are_accepted(self):
        self.repo.write("docs/adr/ADR-0001-title.md", "# ADR-0001 — Old style\n\nStatus: **Accepted**\n")
        self.assertEqual(self.repo.errors(), [])

    def test_a_broken_link_fails(self):
        self.repo.write("AGENTS.md", AGENTS + "\nSee [it](docs/missing.md).\n")
        self.has("broken link")


if __name__ == "__main__":
    unittest.main()
