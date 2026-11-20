"""Regression tests for scripts/check-agent-config.py (run: python3 -m unittest discover scripts/tests)."""
import importlib.util
import json
import os
import pathlib
import shutil
import subprocess
import sys
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

    # -- the EGM plugin entry in the project settings (ADR-0077) ---------------------------
    def egm_settings(self, **overrides):
        settings = {
            "permissions": {"deny": ["Read(./.env)"]},
            "hooks": {"PreToolUse": [{"matcher": "Bash", "hooks": [{"type": "command", "command": "${CLAUDE_PROJECT_DIR}/.claude/hooks/h.py"}]}]},
            "extraKnownMarketplaces": {"egm": {"source": {"source": "github", "repo": "google/elephant-goldfish-model"}, "autoUpdate": False}},
            "enabledPlugins": {"egm@egm": True},
        }
        settings.update(overrides)
        self.repo.write(".claude/settings.json", json.dumps(settings))

    def test_the_accepted_egm_entry_passes(self):
        self.egm_settings()
        self.assertEqual(self.repo.errors(), [])

    def test_a_marketplace_with_auto_update_on_fails(self):
        self.egm_settings(extraKnownMarketplaces={"egm": {"source": {"source": "github", "repo": "google/elephant-goldfish-model"}, "autoUpdate": True}})
        self.has("autoUpdate")

    def test_a_marketplace_without_auto_update_false_fails(self):
        self.egm_settings(extraKnownMarketplaces={"egm": {"source": {"source": "github", "repo": "google/elephant-goldfish-model"}}})
        self.has("autoUpdate")

    def test_another_egm_source_fails(self):
        self.egm_settings(extraKnownMarketplaces={"egm": {"source": {"source": "github", "repo": "someone/else"}, "autoUpdate": False}})
        self.has("google/elephant-goldfish-model")

    def test_another_enabled_plugin_fails(self):
        self.egm_settings(enabledPlugins={"egm@egm": True, "other@market": True})
        self.has("enabledPlugins")

    def test_egm_enabled_without_its_marketplace_fails(self):
        self.egm_settings(extraKnownMarketplaces={})
        self.has("marketplace")


class PathCheck(unittest.TestCase):
    """The backticked-path check and the scoped search (repository fixtures: they need `ls-files`)."""

    def setUp(self):
        self.repo = Repo().good()
        self.addCleanup(self.repo.close)
        self._env = {**os.environ, "GIT_CONFIG_GLOBAL": os.devnull, "GIT_CONFIG_NOSYSTEM": "1"}
        self.vcs("init", "-q", "-b", "develop")
        self.repo.write("docs/a.md", "a\n")
        self.repo.write("tests/x/rest/conformance.rs", "// c\n")
        self.repo.write("crates/x/src/lib.rs", "// l\n")

    def vcs(self, *args):
        subprocess.run(["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
                       cwd=self.repo.root, env=self._env, capture_output=True, check=True)

    def problems(self, agents_text="", allowlist=None):
        self.repo.write("AGENTS.md", AGENTS + agents_text)
        if allowlist is not None:
            self.repo.write("scripts/paths-allowlist.txt", allowlist)
        self.vcs("add", "-A")
        return module.check_paths(self.repo.root)

    def test_existing_paths_pass(self):
        self.assertEqual(self.problems("See `docs/a.md`, `crates/x/` and `AGENTS.md`.\n"), [])

    def test_a_missing_path_fails(self):
        out = self.problems("See `docs/gone.md`.\n")
        self.assertTrue(any("docs/gone.md" in e for e in out), out)

    def test_a_missing_bare_file_name_fails_and_an_existing_one_passes(self):
        self.assertTrue(self.problems("Use `nothing.py`.\n"))
        self.assertEqual(self.problems("Use `lib.rs`.\n"), [])

    def test_placeholders_urls_and_options_are_not_paths(self):
        text = "`docs/<id>.md` `specs/NNN-slug/` `a/*.md` `{a,b}/c.md` `https://x.y/z.md` `-x/y` `$HOME/a.md` `~/a.md` `origin/main` `a/...` `crates/x/src/lib.rs:42` `/srv/studio` `/v1/ready` `@AGENTS.md`\n"
        self.assertEqual(self.problems(text), [])

    def test_slash_commands_are_not_paths(self):
        self.assertEqual(self.problems("Run `/verify`, `/adr` and `/egm:egm-draft`.\n"), [])

    def test_a_path_suffix_counts(self):
        self.assertEqual(self.problems("See `rest/conformance.rs`.\n"), [])

    def test_a_path_relative_to_the_containing_file_counts(self):
        self.repo.write("docs/engineering/guide.md", "See `../a.md`.\n")
        self.assertEqual(self.problems(), [])

    def test_fenced_blocks_are_skipped(self):
        self.assertEqual(self.problems("```\n`docs/gone.md`\n```\n"), [])

    def test_an_allowlisted_path_is_skipped(self):
        self.assertEqual(self.problems("See `target/debug/x`.\n", allowlist="# built outside the repository\ntarget/*\n"), [])

    def test_a_stale_pointer_to_the_old_constitution_is_found(self):
        out = self.problems("See `.specify/memory/constitution.md`.\n")
        self.assertTrue(any(".specify/memory/constitution.md" in e for e in out), out)

    def test_speckit_on_a_live_surface_fails(self):
        out = self.problems("Run the old planning skill, speckit.\n")
        self.assertTrue(any("speckit" in e.lower() for e in out), out)
        self.assertTrue(any("spec kit" in e.lower() for e in self.problems("Use Spec Kit.\n")))

    def test_the_search_ignores_its_own_files(self):
        self.repo.write("scripts/check-agent-config.py", "# speckit\n")
        self.repo.write("scripts/tests/test_check_agent_config.py", "# speckit\n")
        self.repo.write("scripts/paths-allowlist.txt", "# speckit is gone\n")
        self.assertEqual(self.problems(), [])

    def test_a_dated_record_outside_the_surfaces_may_name_it(self):
        self.repo.write("docs/governance/old.md", "Spec Kit and speckit.\n")
        self.assertEqual(self.problems(), [])

    def test_main_exits_2_outside_a_repository(self):
        with tempfile.TemporaryDirectory() as plain:
            result = subprocess.run([sys.executable, str(SCRIPT), plain], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)


    def test_a_relative_path_cannot_escape_the_repository(self):
        self.repo.write("etc/x.md", "x\n")
        self.repo.write("docs/engineering/guide.md", "See `../../../etc/x.md`.\n")
        out = self.problems()
        self.assertTrue(any("../../../etc/x.md" in e for e in out), out)

    def test_a_directory_token_matches_a_directory_by_path_or_by_suffix(self):
        self.repo.write("specs/001-x/contracts/a.json", "{}\n")
        self.assertEqual(self.problems("See `contracts/` and `specs/001-x/contracts/`.\n"), [])
        self.assertTrue(self.problems("See `contracts/nothing/`.\n"))


class TransitionalWording(unittest.TestCase):
    """Every file that tells an agent which document to read carries the alternative for a feature
    not yet converted to a design doc (the constitution's transitional clause)."""

    FILES = (
        "AGENTS.md",
        ".claude/agents/architecture-reviewer.md",
        ".claude/agents/security-reviewer.md",
        ".claude/agents/correctness-reviewer.md",
        ".claude/skills/verify/SKILL.md",
        ".claude/skills/parallel-agents/SKILL.md",
        ".github/pull_request_template.md",
    )

    def test_the_seven_dependents_carry_the_wording(self):
        root = pathlib.Path(__file__).resolve().parents[2]
        for rel in self.FILES:
            with self.subTest(file=rel):
                text = (root / rel).read_text()
                self.assertIn("not yet converted", " ".join(text.split()), f"{rel} lacks the transitional wording")


if __name__ == "__main__":
    unittest.main()
