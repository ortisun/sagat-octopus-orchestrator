#!/usr/bin/env python3
"""Keeps the agent configuration honest: bounded, consistent and wired (docs/engineering/ai-engineering.md).

Checks (all must pass; exit 1 otherwise):
  * AGENTS.md exists, stays within its line budget and keeps rules 1..N contiguous (the constitution cites
    them by number).
  * CLAUDE.md imports AGENTS.md; every nested CLAUDE.md is only that import and has a sibling AGENTS.md;
    nested AGENTS.md files stay small.
  * Skills have front matter (name equals the directory, a description within the listing limit) and stay
    under 500 lines; agents have name and description and the reviewers are read-only.
  * .claude/settings.json parses and every hook command it names exists and is executable.
  * ADRs start with `# ADR-NNNN` (colon or dash), carry a Status line near the top and have unique numbers.
  * Relative links in the instruction files and docs/engineering resolve.

Run from anywhere: python3 scripts/check-agent-config.py [repo-root]
"""
import json
import pathlib
import re
import sys

ROOT_AGENTS_BUDGET = 160
NESTED_AGENTS_BUDGET = 60
CLAUDE_BUDGET = 60
SKILL_BUDGET = 500
DESCRIPTION_LIMIT = 1536
WRITE_TOOLS = {"Write", "Edit", "NotebookEdit", "Bash"}
SKIP_DIRS = {"node_modules", "target", ".git", ".venv", "dist", "bin", "obj", ".claude/worktrees"}


def frontmatter(text: str):
    """(dict, body) for a file that starts with a --- block of simple `key: value` lines."""
    if not text.startswith("---\n"):
        return None, text
    end = text.find("\n---", 4)
    if end < 0:
        return None, text
    fields = {}
    for line in text[4:end].splitlines():
        raw = re.match(r"^[A-Za-z_-]+:\s*(\".*)$", line)
        if raw and not re.fullmatch(r'"(?:[^"\\]|\\.)*"', raw.group(1).strip()):
            fields["__yaml_error__"] = line

        match = re.match(r"^([A-Za-z_-]+):\s*(.*)$", line)
        if match:
            fields[match.group(1)] = match.group(2).strip().strip('"\'')
    return fields, text[end + 4:]


def walk(root: pathlib.Path, name: str):
    for path in root.rglob(name):
        rel = path.relative_to(root)
        if any(part in SKIP_DIRS for part in rel.parts):
            continue
        if ".claude/worktrees" in rel.as_posix():
            continue
        yield path


def check(root: pathlib.Path) -> list[str]:
    errors: list[str] = []

    def err(message: str) -> None:
        errors.append(message)

    # AGENTS.md and CLAUDE.md
    agents = root / "AGENTS.md"
    if not agents.is_file():
        err("AGENTS.md is missing")
    else:
        text = agents.read_text()
        lines = text.splitlines()
        if len(lines) > ROOT_AGENTS_BUDGET:
            err(f"AGENTS.md has {len(lines)} lines; the budget is {ROOT_AGENTS_BUDGET} (move detail to docs/engineering or a nested AGENTS.md)")
        numbers = [int(m.group(1)) for m in (re.match(r"^(\d+)\. ", line) for line in lines) if m]
        if numbers != list(range(1, len(numbers) + 1)):
            err(f"AGENTS.md rules must be numbered 1..N without gaps (the constitution cites them); found {numbers}")
    claude = root / "CLAUDE.md"
    if not claude.is_file():
        err("CLAUDE.md is missing (it imports AGENTS.md for Claude Code)")
    else:
        text = claude.read_text()
        first = next((line for line in text.splitlines() if line.strip()), "")
        if first.strip() != "@AGENTS.md":
            err("CLAUDE.md must start with the import `@AGENTS.md`")
        if len(text.splitlines()) > CLAUDE_BUDGET:
            err(f"CLAUDE.md has more than {CLAUDE_BUDGET} lines")

    for path in walk(root, "CLAUDE.md"):
        if path == claude:
            continue
        rel = path.relative_to(root)
        if path.read_text().strip() != "@AGENTS.md":
            err(f"{rel}: a nested CLAUDE.md contains only `@AGENTS.md`")
        if not (path.parent / "AGENTS.md").is_file():
            err(f"{rel}: no AGENTS.md next to it")
    for path in walk(root, "AGENTS.md"):
        if path == agents:
            continue
        rel = path.relative_to(root)
        count = len(path.read_text().splitlines())
        if count > NESTED_AGENTS_BUDGET:
            err(f"{rel} has {count} lines; the nested budget is {NESTED_AGENTS_BUDGET}")
        if not (path.parent / "CLAUDE.md").is_file():
            err(f"{rel}: add a CLAUDE.md containing `@AGENTS.md` so Claude Code loads it")

    # Skills
    skills = root / ".claude" / "skills"
    if skills.is_dir():
        for skill in sorted(skills.glob("*/SKILL.md")):
            rel = skill.relative_to(root)
            text = skill.read_text()
            fields, _ = frontmatter(text)
            if fields is None:
                err(f"{rel}: missing front matter")
                continue
            if "__yaml_error__" in fields:
                err(f"{rel}: front matter is not valid YAML at `{fields['__yaml_error__'][:60]}` (a quoted value must be one string)")
            if fields.get("name", skill.parent.name) != skill.parent.name:
                err(f"{rel}: name must equal the directory name")
            description = fields.get("description", "")
            if not description:
                err(f"{rel}: description is required")
            if len(description) + len(fields.get("when_to_use", "")) > DESCRIPTION_LIMIT:
                err(f"{rel}: description plus when_to_use exceed {DESCRIPTION_LIMIT} characters")
            if len(text.splitlines()) > SKILL_BUDGET:
                err(f"{rel}: more than {SKILL_BUDGET} lines")

    # Subagents
    agents_dir = root / ".claude" / "agents"
    if agents_dir.is_dir():
        for agent in sorted(agents_dir.glob("*.md")):
            rel = agent.relative_to(root)
            fields, _ = frontmatter(agent.read_text())
            if fields and "__yaml_error__" in fields:
                err(f"{rel}: front matter is not valid YAML at `{fields['__yaml_error__'][:60]}`")
            if not fields or not fields.get("name") or not fields.get("description"):
                err(f"{rel}: name and description are required in the front matter")
                continue
            if fields["name"].endswith("-reviewer"):
                tools = {t.strip() for t in fields.get("tools", "").split(",") if t.strip()}
                if not tools:
                    err(f"{rel}: a reviewer lists its tools (an allowlist) so it stays read-only")
                if tools & WRITE_TOOLS:
                    err(f"{rel}: a reviewer must not have {sorted(tools & WRITE_TOOLS)}")

    # Settings and hooks
    settings = root / ".claude" / "settings.json"
    if settings.is_file():
        try:
            data = json.loads(settings.read_text())
        except ValueError as exc:
            err(f".claude/settings.json does not parse: {exc}")
            data = {}
        for event, groups in (data.get("hooks") or {}).items():
            for group in groups:
                for hook in group.get("hooks", []):
                    command = hook.get("command", "")
                    path = pathlib.Path(command.replace("${CLAUDE_PROJECT_DIR}", str(root)))
                    if not path.is_file():
                        err(f".claude/settings.json: {event} hook {command} does not exist")
                    elif not path.stat().st_mode & 0o111:
                        err(f".claude/settings.json: {event} hook {command} is not executable")
        deny = " ".join((data.get("permissions") or {}).get("deny", []))
        if ".env" not in deny:
            err(".claude/settings.json must deny reading .env files")

    # ADRs
    seen: dict[str, str] = {}
    adr_dir = root / "docs" / "adr"
    if adr_dir.is_dir():
        for adr in sorted(adr_dir.glob("ADR-*.md")):
            rel = adr.relative_to(root)
            text = adr.read_text()
            head = re.match(r"^# (ADR-(\d{4}))\b", text)
            if not head:
                err(f"{rel}: must start with `# ADR-NNNN` (a colon or a dash may follow)")
                continue
            if not adr.name.startswith(head.group(1) + "-"):
                err(f"{rel}: the file name and the title number disagree")
            if not re.search(r"^\W*status\W", "\n".join(text.splitlines()[:15]), re.I | re.M):
                err(f"{rel}: missing a Status line in the first lines")
            if head.group(2) in seen:
                err(f"{rel}: number {head.group(2)} is also used by {seen[head.group(2)]}")
            seen[head.group(2)] = str(rel)

    # Links
    docs = [agents, claude, *sorted((root / "docs" / "engineering").glob("*.md")), root / "docs" / "adr" / "README.md"]
    docs += [p for p in walk(root, "AGENTS.md")]
    for doc in docs:
        if not doc.is_file():
            continue
        for match in re.finditer(r"\]\(([^)#\s]+)(?:#[^)]*)?\)", doc.read_text()):
            target = match.group(1)
            if re.match(r"^[a-z]+:", target):
                continue
            if not (doc.parent / target).exists():
                err(f"{doc.relative_to(root)}: broken link {target}")
    return errors


def main() -> int:
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[1]).resolve()
    errors = check(root)
    for message in errors:
        print(f"error: {message}")
    if errors:
        print(f"{len(errors)} problem(s) in the agent configuration")
        return 1
    print("agent configuration ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
