#!/usr/bin/env python3
"""SessionStart: tell the agent where it is, so it does not rebuild what a sibling session already built."""
import json
import os
import re
import shutil
import subprocess


def git(*args: str) -> str:
    try:
        return subprocess.run(["git", *args], capture_output=True, text=True, timeout=10).stdout.strip()
    except Exception:
        return ""


lines = []
branch = git("rev-parse", "--abbrev-ref", "HEAD")
if branch:
    lines.append(f"Branch: {branch}" + ("  (protected: cut a branch before committing)" if branch in ("main", "develop") else ""))
try:
    with open(os.path.join(os.environ.get("CLAUDE_PROJECT_DIR", "."), ".specify", "feature.json")) as f:
        feature = json.load(f).get("feature_directory", "")
except Exception:
    feature = ""
if feature:
    lines.append(f"Active feature (.specify/feature.json): {feature}")
    match = re.search(r"specs/(\d{3})-", feature)
    if match:
        fid = match.group(1)
        others = [b for b in git("branch", "-a", "--format=%(refname:short)").splitlines()
                  if fid in b and b != branch]
        if others:
            lines.append(f"Other branches mention {fid}: {', '.join(others[:6])}"
                         + (" ..." if len(others) > 6 else "") + " — check they are not the same work (git worktree list).")
free = shutil.disk_usage(os.path.expanduser("~")).free / 2**30
if free < 20:
    lines.append(f"Disk: only {free:.0f} GiB free. Use a private CARGO_TARGET_DIR and do not start parallel builds.")
if lines:
    print("\n".join(lines))
