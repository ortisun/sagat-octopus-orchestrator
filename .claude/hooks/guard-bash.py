#!/usr/bin/env python3
"""PreToolUse guard for Bash: the rules of AGENTS.md that must not depend on an agent remembering them.

Reads the hook JSON on stdin. Prints a permission decision (deny or ask) with the reason, or nothing
when the command is fine. Never raises: a broken guard must not block the session.
"""
import json
import re
import subprocess
import sys

PROTECTED = {"main", "develop"}


def decide(kind: str, reason: str) -> None:
    print(json.dumps({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": kind,
        "permissionDecisionReason": reason,
    }}))
    sys.exit(0)


def current_branch(cwd: str) -> str:
    try:
        out = subprocess.run(["git", "rev-parse", "--abbrev-ref", "HEAD"], cwd=cwd or None,
                             capture_output=True, text=True, timeout=5)
        return out.stdout.strip()
    except Exception:
        return ""


def check(command: str, cwd: str) -> None:
    lower = command.lower()
    segments = re.split(r"&&|\|\||;|\n|\|", command)

    for seg in segments:
        s = seg.strip()
        if re.match(r"git\s+(-c\s+\S+\s+)?commit\b", s) and "co-authored-by" in lower:
            decide("deny", "AGENTS.md rule 13: no Co-Authored-By trailers in commit messages.")
        if re.match(r"git\s+(commit|push)\b", s) and "--no-verify" in s:
            decide("deny", "Do not skip hooks (--no-verify); fix what the hook reports.")
        if re.match(r"git\s+(commit|merge|cherry-pick|revert|rebase|am)(?![-\w])", s) \
                and not re.search(r"--(abort|quit|continue|skip)\b", s):
            if current_branch(cwd) in PROTECTED:
                decide("deny", "AGENTS.md rule 14: never commit or merge directly on main or develop; "
                               "cut a feature/, bugfix/, release/ or hotfix/ branch and open a pull request.")
        if re.match(r"git\s+push\b", s):
            tokens = s.split()[2:]
            refs = [t for t in tokens if not t.startswith("-")]
            targets = []
            for t in refs:
                targets.append(t.split(":")[-1].replace("refs/heads/", ""))
            if any(t in PROTECTED for t in targets) or (len(refs) <= 1 and current_branch(cwd) in PROTECTED):
                decide("deny", "AGENTS.md rule 14: main and develop change through pull requests, not pushes.")
            if re.search(r"(^|\s)(--force|-f|--force-with-lease)(\s|=|$)", s):
                decide("ask", "A force push rewrites published history. Confirm the branch and that nobody else "
                              "is working on it (other sessions use sibling worktrees).")
        if re.match(r"git\s+clean\b", s) and re.search(r"\s-\w*f", s):
            decide("ask", "git clean -f deletes untracked files, which may belong to another session.")
        if re.match(r"docker\s+(system|container|image|volume|builder|network)\s+prune\b", s) \
                and "sagat-remaining" not in command:
            decide("deny", "AGENTS.md rule 19: do not prune the default Docker daemon (it holds other projects). "
                           "Use DOCKER_HOST=unix://$HOME/.colima/sagat-remaining/docker.sock for this project.")
        if re.match(r"cargo\s+clean\b", s) and "CARGO_TARGET_DIR=" not in command and "--target-dir" not in command:
            decide("deny", "AGENTS.md rule 19: cargo clean on the shared target breaks other sessions' builds. "
                           "Set a private CARGO_TARGET_DIR and clean that.")
        if re.match(r"rm\s+(-\w+\s+)*-\w*[rf]\w*\s+(/|~|\$HOME|\.git)(/?\s|/?$)", s):
            decide("deny", "Refusing to remove the root, the home directory or .git.")
        if re.match(r"gh\s+release\s+(create|delete)\b", s):
            decide("ask", "Releases are the owner's decision (docs/runbooks/release.md).")


def main() -> None:
    try:
        data = json.load(sys.stdin)
        command = (data.get("tool_input") or {}).get("command") or ""
        check(command, data.get("cwd") or "")
    except SystemExit:
        raise
    except Exception:
        pass
    sys.exit(0)


if __name__ == "__main__":
    main()
