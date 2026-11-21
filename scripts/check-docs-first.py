#!/usr/bin/env python3
"""Documentation first: a pull request that changes behaviour code changes a document too.

Usage: python3 scripts/check-docs-first.py --base REV [--head REV] [--root DIR]

The rule (docs/constitution.md 4.7.0, Documentation first; AGENTS.md rule 23): the document leads and the code follows.
This check sees the part of the rule that a machine can see:

  * It reads the pull request's own commits, `base..head`, merge commits left out.
  * It judges a commit only when the commit's tree contains this script: a commit written before the rule existed (on a
    branch that was open when the rule merged) is never judged. A commit made after such a branch merged the rule is.
  * Behaviour code is a file that is not Markdown under crates/, apps/, sdks/, runners/, frontend/apps/ or infra/, or
    any file in a migrations/ directory. A document is a file under docs/ or specs/.
  * The pull request passes when no judged commit changes behaviour code, or when any of its own commits changes a
    document. Otherwise every judged commit that changes behaviour code must carry the footer line
    `Docs: none, <reason>`; the reasons are printed so that a reviewer can judge them.
  * When a judged commit changes behaviour code before the first commit that changes a document, the check passes and
    prints a warning: the document came after the code. It cannot fail for that, because the only fix would be to
    rewrite the branch, and force pushes are refused.

What it cannot see: whether the document that changed is the one that describes the code
(docs/governance/constitution-4.7.0-sync-impact.md, follow-ups).

Exit codes: 0 pass; 1 the rule is broken; 2 a revision cannot be read.
"""
import argparse
import os
import re
import subprocess
import sys

RULE_PATH = "scripts/check-docs-first.py"
BEHAVIOUR = re.compile(r"(crates|apps|sdks|runners|frontend/apps|infra)/.*")
MIGRATIONS = re.compile(r"(.*/)?migrations/.*")
DOCUMENT = re.compile(r"(docs|specs)/.*")
MARKDOWN = re.compile(r".*\.md")
# The footer: a whole line, a reason with at least one letter or digit.
EXEMPTION = re.compile(r"(?m)^Docs: none, (?P<reason>[^\n]*[A-Za-z0-9][^\n]*?)\s*$")
ADVICE = """\
The document leads and the code follows (docs/constitution.md, Documentation first; AGENTS.md rule 23).
Fix it by changing the document that describes this change (docs/ or specs/) in a new commit of this pull request.
If no document should change (a refactor, formatting, a dependency bump, a test-only fix), each commit listed above
must carry the footer line `Docs: none, <reason>`; a pushed commit cannot gain one without a force push, so recut the
branch with the footer."""


class Untrusted(Exception):
    pass


def git(root, *args):
    result = subprocess.run(["git", "-C", root, *args], capture_output=True, text=True)
    if result.returncode != 0:
        raise Untrusted(f"git {' '.join(args)}: {result.stderr.strip()}")
    return result.stdout


def is_behaviour(path):
    if MARKDOWN.fullmatch(path):
        return False
    return bool(BEHAVIOUR.fullmatch(path) or MIGRATIONS.fullmatch(path))


def is_document(path):
    return bool(DOCUMENT.fullmatch(path))


class Commit:
    def __init__(self, root, sha):
        self.sha = sha
        self.short = sha[:12]
        message = git(root, "log", "-1", "--format=%B", sha)
        self.subject = message.split("\n", 1)[0]
        found = EXEMPTION.search(message)
        self.reason = found.group("reason").strip() if found else None
        files = git(root, "diff-tree", "--no-commit-id", "--root", "-r", "--name-only", sha).split("\n")
        self.files = [f for f in files if f]
        self.code = [f for f in self.files if is_behaviour(f)]
        self.documents = [f for f in self.files if is_document(f)]
        self.judged = subprocess.run(["git", "-C", root, "cat-file", "-e", f"{sha}:{RULE_PATH}"],
                                     capture_output=True).returncode == 0


def judge(root, base, head):
    """(exit code, lines to print)."""
    for rev in (base, head):
        git(root, "rev-parse", "--verify", "--quiet", f"{rev}^{{commit}}")
    shas = [s for s in git(root, "rev-list", "--no-merges", "--reverse", f"{base}..{head}").split("\n") if s]
    commits = [Commit(root, sha) for sha in shas]
    judged = [c for c in commits if c.judged]
    lines = [f"docs-first: {len(commits)} commit(s) in the pull request, {len(judged)} judged, "
             f"{len(commits) - len(judged)} written before the check existed"]
    coding = [c for c in judged if c.code]
    for commit in coding:
        if commit.reason:
            lines.append(f"exempt {commit.short} {commit.subject}: Docs: none, {commit.reason}")
    if not coding:
        lines.append("docs-first: ok, no behaviour code in the judged commits")
        return 0, lines
    documenting = [i for i, c in enumerate(commits) if c.documents]
    if documenting:
        first = documenting[0]
        for index, commit in enumerate(commits):
            if index < first and commit in coding and not commit.reason:
                lines.append(f"docs-first: warning: {commit.short} {commit.subject} changes behaviour code before "
                             f"the first commit that changes a document ({commits[first].short})")
        lines.append("docs-first: ok, the pull request changes a document")
        return 0, lines
    missing = [c for c in coding if not c.reason]
    if not missing:
        lines.append("docs-first: ok, every commit that changes behaviour code says why no document changes")
        return 0, lines
    lines.append("docs-first: this pull request changes behaviour code and no document under docs/ or specs/; "
                 "these commits carry no `Docs: none, <reason>` footer:")
    for commit in missing:
        lines.append(f"  {commit.short} {commit.subject}")
        lines.extend(f"      {path}" for path in commit.code[:10])
        if len(commit.code) > 10:
            lines.append(f"      ... and {len(commit.code) - 10} more")
    lines.append(ADVICE)
    return 1, lines


def main(argv):
    parser = argparse.ArgumentParser(description="A pull request that changes behaviour code changes a document.")
    parser.add_argument("--base", required=True, help="the base of the pull request (a revision)")
    parser.add_argument("--head", default="HEAD", help="the head of the pull request (default: HEAD)")
    parser.add_argument("--root", default=os.getcwd(), help="repository root (default: the current directory)")
    args = parser.parse_args(argv[1:])
    try:
        code, lines = judge(args.root, args.base, args.head)
    except Untrusted as error:
        print(f"docs-first: {error}", file=sys.stderr)
        return 2
    print("\n".join(lines))
    return code


if __name__ == "__main__":
    sys.exit(main(sys.argv))
