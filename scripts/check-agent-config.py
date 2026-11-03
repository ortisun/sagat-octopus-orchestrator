#!/usr/bin/env python3
"""Keeps the agent configuration honest: bounded, consistent and wired (docs/engineering/ai-engineering.md).

Checks (all must pass; exit 1 otherwise):
  * AGENTS.md exists, stays within its line budget and keeps rules 1..N contiguous (the constitution cites
    them by number).
  * CLAUDE.md imports AGENTS.md; every nested CLAUDE.md is only that import and has a sibling AGENTS.md;
    nested AGENTS.md files stay small.
  * Skills have front matter (name equals the directory, a description within the listing limit) and stay
    under 500 lines; agents have name and description and the reviewers are read-only.
  * .claude/settings.json parses and every hook command it names exists and is executable; the EGM
    marketplace has autoUpdate false and a known source, and no other plugin is enabled.
  * ADRs start with `# ADR-NNNN` (colon or dash), carry a Status line near the top and have unique numbers.
  * Relative links in the instruction files and docs/engineering resolve.
  * Backticked paths in the instruction surfaces exist in the repository (check_paths, needs git), and
    the removed specification tooling (Spec Kit) is named nowhere on a live surface.

Run from anywhere: python3 scripts/check-agent-config.py [repo-root]  (exit 2 outside a git repository)
"""
import fnmatch
import json
import pathlib
import re
import subprocess
import sys

ROOT_AGENTS_BUDGET = 160
NESTED_AGENTS_BUDGET = 60
CLAUDE_BUDGET = 60
SKILL_BUDGET = 500
DESCRIPTION_LIMIT = 1536
WRITE_TOOLS = {"Write", "Edit", "NotebookEdit", "Bash"}
EGM_REPO = "google/elephant-goldfish-model"
EGM_PLUGIN = "egm@egm"
PATH_SUFFIXES = (".md", ".py", ".rs", ".toml", ".json", ".yaml", ".yml", ".sh")
NOT_A_PATH_PARTS = ("<", ">", "*", "{", "NNN", "...", "://")
NOT_A_PATH_STARTS = ("http", "-", "$", "~", "origin/", "/", "@")
SPAN = re.compile(r"(?<!`)`([^`\n]+)`(?!`)")
REMOVED_TOOL = re.compile(r"speckit|spec kit", re.I)
SEARCH_FILES = {"AGENTS.md", "CLAUDE.md", "README.md", "docs/constitution.md"}
SEARCH_PREFIXES = (".claude/", ".github/", "scripts/", "docs/engineering/")
SEARCH_EXCLUDED = {"scripts/check-agent-config.py", "scripts/tests/test_check_agent_config.py",
                   "scripts/paths-allowlist.txt"}
PATH_SCAN_FILES = {"AGENTS.md", "CLAUDE.md", "README.md", "SPECIFICATION-INDEX.md", "specs/README.md",
                   "docs/constitution.md", "docs/adr/README.md"}
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
        # The EGM plugin is pinned by content (ADR-0077): no auto-update, one known source, one plugin.
        marketplaces = data.get("extraKnownMarketplaces") or {}
        for name, entry in marketplaces.items():
            if not isinstance(entry, dict) or entry.get("autoUpdate") is not False:
                err(f".claude/settings.json: marketplace {name} must set autoUpdate to false")
        if "egm" in marketplaces:
            source = (marketplaces["egm"] or {}).get("source") if isinstance(marketplaces["egm"], dict) else None
            if not (isinstance(source, dict) and source.get("source") == "github"
                    and source.get("repo") == EGM_REPO):
                err(f".claude/settings.json: the egm marketplace must come from github {EGM_REPO}")
        enabled = data.get("enabledPlugins") or {}
        if set(enabled) - {EGM_PLUGIN}:
            err(f".claude/settings.json: enabledPlugins may only contain {EGM_PLUGIN}, not {sorted(set(enabled) - {EGM_PLUGIN})}")
        if EGM_PLUGIN in enabled and "egm" not in marketplaces:
            err(f".claude/settings.json: {EGM_PLUGIN} is enabled but the egm marketplace is not declared")

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


def tracked_files(root: pathlib.Path):
    """Tracked paths (POSIX, relative to root), or None outside a git repository."""
    result = subprocess.run(["git", "-C", str(root), "ls-files", "-z"], capture_output=True)
    if result.returncode != 0:
        return None
    return [name for name in result.stdout.decode("utf-8", errors="replace").split("\0") if name]


def fence_flags(lines):
    flags, marker = [], None
    for line in lines:
        match = re.match(r"^ {0,3}(```|~~~)", line)
        if marker is None:
            marker = match.group(1) if match else None
            flags.append(bool(match))
        else:
            flags.append(True)
            if match and match.group(1) == marker:
                marker = None
    return flags


def is_path_token(token: str) -> bool:
    if " " in token or not ("/" in token or token.endswith(PATH_SUFFIXES)):
        return False
    if any(part in token for part in NOT_A_PATH_PARTS) or token.startswith(NOT_A_PATH_STARTS):
        return False
    return not re.search(r":[0-9]+$", token)


def path_exists(token: str, source: str, tracked: list, basenames: set) -> bool:
    if "/" not in token:
        return token in basenames
    bare = token.rstrip("/")
    here = pathlib.PurePosixPath(source).parent
    candidates = {bare}
    relative = pathlib.PurePosixPath(*here.parts, bare)
    parts = []
    for part in relative.parts:
        if part == "..":
            if parts:
                parts.pop()
        elif part != ".":
            parts.append(part)
    candidates.add("/".join(parts))
    for path in tracked:
        for candidate in candidates:
            if path == candidate or path.startswith(candidate + "/"):
                return True
        if path.endswith("/" + bare) or ("/" + bare + "/") in ("/" + path):
            return True
    return False


def check_paths(root: pathlib.Path) -> list:
    """Backticked paths that do not exist, and mentions of the removed specification tooling."""
    tracked = tracked_files(root)
    if tracked is None:
        return ["not inside a git repository: the path check needs `git ls-files`"]
    errors = []
    basenames = {name.rsplit("/", 1)[-1] for name in tracked}
    allowlist_file = root / "scripts" / "paths-allowlist.txt"
    allowed = []
    if allowlist_file.is_file():
        allowed = [line.strip() for line in allowlist_file.read_text().splitlines()
                   if line.strip() and not line.lstrip().startswith("#")]
    for rel in tracked:
        scanned = (rel in PATH_SCAN_FILES or rel.endswith("/AGENTS.md")
                   or (rel.startswith("docs/engineering/") and rel.endswith(".md"))
                   or (rel.startswith((".claude/agents/", ".claude/skills/")) and rel.endswith(".md")))
        searched = ((rel in SEARCH_FILES or rel.startswith(SEARCH_PREFIXES)) and rel not in SEARCH_EXCLUDED)
        if not (scanned or searched):
            continue
        try:
            lines = (root / rel).read_text().splitlines()
        except (OSError, UnicodeDecodeError):
            continue
        flags = fence_flags(lines)
        for number, line in enumerate(lines, start=1):
            if searched and REMOVED_TOOL.search(line):
                errors.append(f"{rel}:{number}: names the removed tooling (Spec Kit / speckit)")
            if not scanned or flags[number - 1]:
                continue
            for token in (m.group(1) for m in SPAN.finditer(line)):
                if not is_path_token(token) or any(fnmatch.fnmatchcase(token, pat) for pat in allowed):
                    continue
                if not path_exists(token, rel, tracked, basenames):
                    errors.append(f"{rel}:{number}: `{token}` does not exist in the repository")
    return errors


def main() -> int:
    root = pathlib.Path(sys.argv[1] if len(sys.argv) > 1 else pathlib.Path(__file__).resolve().parents[1]).resolve()
    if tracked_files(root) is None:
        print("error: not inside a git repository (the path check needs git ls-files)")
        return 2
    errors = check(root) + check_paths(root)
    for message in errors:
        print(f"error: {message}")
    if errors:
        print(f"{len(errors)} problem(s) in the agent configuration")
        return 1
    print("agent configuration ok")
    return 0


if __name__ == "__main__":
    sys.exit(main())
