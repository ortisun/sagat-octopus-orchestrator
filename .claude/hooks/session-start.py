#!/usr/bin/env python3
"""SessionStart: tell the agent where it is, so it does not rebuild what a sibling session already built.

It also warns, without blocking anything, when the EGM plugin is missing or is not the accepted copy
(docs/engineering/egm-accepted-hashes.json). Branch names are untrusted: only plain names are printed.
"""
import json
import os
import re
import shutil
import subprocess
import sys

PLAIN_NAME = re.compile(r"[A-Za-z0-9._/-]{1,80}")
FEATURE_NAME = re.compile(r"(?:origin/)?feature/[0-9]{3}-[a-z0-9-]{1,40}")
NOT_INSTALLED = 'EGM plugin not installed: run "claude plugin install egm@egm --scope project"'
UNLISTED = ("EGM plugin hash is not in docs/engineering/egm-accepted-hashes.json: "
            "a review made with it will fail scripts/check-design-docs.py")
SKIPPED = "EGM check skipped"


def git(*args: str) -> str:
    try:
        return subprocess.run(["git", *args], capture_output=True, text=True, timeout=10).stdout.strip()
    except Exception:
        return ""


def other_branches(branch: str, number: str) -> list:
    names = []
    for name in git("branch", "-a", "--format=%(refname:short)").splitlines():
        if number not in name or not FEATURE_NAME.fullmatch(name):
            continue
        name = name.removeprefix("origin/")
        if name != branch and name not in names:
            names.append(name)
    return names


def egm_warning(project: str):
    """None when the installed EGM copy is accepted, otherwise the single line to print."""
    try:
        try:
            with open(os.path.join(os.path.expanduser("~"), ".claude", "plugins", "installed_plugins.json")) as handle:
                entries = json.load(handle).get("plugins", {}).get("egm@egm", [])
        except FileNotFoundError:
            return NOT_INSTALLED
        paths = [e["installPath"] for e in entries if os.path.isdir(e.get("installPath", ""))]
        if not paths:
            return NOT_INSTALLED
        with open(os.path.join(project, "docs", "engineering", "egm-accepted-hashes.json")) as handle:
            accepted = {entry["hash"] for entry in json.load(handle)["entries"]}
        script = os.path.join(project, "scripts", "egm-tree-hash.py")
        for path in paths:
            result = subprocess.run([sys.executable, script, path], capture_output=True, text=True, timeout=8)
            if result.returncode != 0:
                return SKIPPED
            if result.stdout.strip() not in accepted:
                return UNLISTED
        return None
    except Exception:
        return SKIPPED


lines = []
branch = git("rev-parse", "--abbrev-ref", "HEAD")
if branch:
    shown = branch if PLAIN_NAME.fullmatch(branch) else "(unusual name)"
    lines.append(f"Branch: {shown}" + ("  (protected: cut a branch before committing)" if branch in ("main", "develop") else ""))
    match = re.match(r"feature/([0-9]{3})-", branch)
    if match:
        others = other_branches(branch, match.group(1))
        if others:
            lines.append(f"Other branches mention {match.group(1)}: {', '.join(others[:6])}"
                         + (" ..." if len(others) > 6 else "") + " — check they are not the same work (git worktree list).")
warning = egm_warning(os.environ.get("CLAUDE_PROJECT_DIR", "."))
if warning:
    lines.append(warning)
free = shutil.disk_usage(os.path.expanduser("~")).free / 2**30
if free < 20:
    lines.append(f"Disk: only {free:.0f} GiB free. Use a private CARGO_TARGET_DIR and do not start parallel builds.")
if lines:
    print("\n".join(lines))
