#!/usr/bin/env python3
"""Print the next free three-digit feature number.

Usage: python3 scripts/next-feature-id.py [--root DIR]

The number is the highest NNN found plus one, among the directories specs/NNN-* and the
files or directories docs/designs/NNN-* of the working tree and of every ref under
refs/heads/ and refs/remotes/origin/. A number of 1000 or more is ignored with a warning.
A warning goes to standard error when the last `git fetch` is missing or older than a day,
because numbers taken on branches that were never fetched are invisible.

It prints the number only: D, G, O and ADR numbers are not allocated here.
Exit codes: 0 number printed; 1 no number left below 1000; 2 not a git repository.
"""
import argparse
import os
import re
import subprocess
import sys
import time

LEADING_NUMBER = re.compile(r"^(\d+)-")
PRINTABLE_REF = re.compile(r"^[A-Za-z0-9._/-]{1,80}$")
TREE_DIRS = ("specs", "docs/designs")
STALE_AFTER_SECONDS = 86400


def git(root, *args, check=True):
    return subprocess.run(
        ["git", "-C", root, *args], capture_output=True, text=True, check=check
    )


def number_of(name):
    match = LEADING_NUMBER.match(name)
    return int(match.group(1)) if match else None


def working_tree_names(root):
    for rel in TREE_DIRS:
        try:
            yield from os.listdir(os.path.join(root, rel))
        except OSError:
            continue


def refs(root):
    """Yield (refname, commit) once per distinct commit, branches first."""
    out = git(
        root, "for-each-ref", "--format=%(objectname) %(refname)", "refs/heads", "refs/remotes/origin"
    ).stdout
    seen = set()
    for line in out.splitlines():
        commit, _, refname = line.partition(" ")
        if commit in seen or not refname:
            continue
        seen.add(commit)
        yield refname, commit


def tree_names(root, commit):
    out = git(root, "ls-tree", "-z", "--name-only", commit, "--", "specs/", "docs/designs/").stdout
    for entry in out.split("\0"):
        if entry:
            yield entry.rsplit("/", 1)[-1]


def describe(refname):
    return refname if PRINTABLE_REF.match(refname) else "a ref whose name is not printable"


def fetch_warning(root):
    path = git(root, "rev-parse", "--git-path", "FETCH_HEAD").stdout.strip()
    if not os.path.isabs(path):
        path = os.path.join(root, path)
    try:
        age = time.time() - os.stat(path).st_mtime
    except OSError:
        return "FETCH_HEAD is missing: run `git fetch` first, or numbers taken on other branches may be invisible"
    if age > STALE_AFTER_SECONDS:
        return "the last fetch is more than a day old: run `git fetch` first, or numbers taken on other branches may be invisible"
    return None


def find_numbers(root):
    """Return (numbers, warnings)."""
    numbers, warnings = set(), []

    def take(name, source):
        value = number_of(name)
        if value is None:
            return
        if value >= 1000:
            warnings.append(f"ignored the number {value} found in {source}")
            return
        numbers.add(value)

    for name in working_tree_names(root):
        take(name, "the working tree")
    for refname, commit in refs(root):
        for name in tree_names(root, commit):
            take(name, describe(refname))
    return numbers, warnings


def main(argv):
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--root", help="repository directory (default: the current repository)")
    args = parser.parse_args(argv[1:])
    start = args.root or os.getcwd()
    probe = git(start, "rev-parse", "--show-toplevel", check=False)
    if probe.returncode != 0:
        print("next-feature-id: not inside a git repository", file=sys.stderr)
        return 2
    root = probe.stdout.strip()
    numbers, warnings = find_numbers(root)
    stale = fetch_warning(root)
    if stale:
        warnings.append(stale)
    for message in warnings:
        print(f"next-feature-id: warning: {message}", file=sys.stderr)
    following = max(numbers, default=0) + 1
    if following > 999:
        print("next-feature-id: no three-digit number is left", file=sys.stderr)
        return 1
    print(f"{following:03d}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
