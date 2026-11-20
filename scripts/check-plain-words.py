#!/usr/bin/env python3
"""Words in full: documents and code comments do not use abbreviations such as "rule R8" or "architecture B1".

Usage: python3 scripts/check-plain-words.py --base REV [--head REV] [--root DIR]
       python3 scripts/check-plain-words.py --all [--root DIR]

The rule (docs/constitution.md 4.7.0, Words in full; AGENTS.md rule 24; docs/governance/identifier-scheme.md):

  * It reads the lines that the pull request's own commits added and that are still there at the head: `git blame` of
    each file the commits touched names the commit of each line, and a line counts when that commit is one of the pull
    request's own commits (`base..head`, merges left out) and its tree contains this script. A line written before the
    rule existed is never judged. With --all it reads every line of every file in scope instead (to measure the
    patterns, and for the sweep).
  * In scope: Markdown under docs/ and specs/ (prose), except review reports (`*.reviews/`, `specs/<feature>/reviews/`,
    the reviewers' own words); and the comments of code files (the text after `//`, `#` or `--`, and the lines of a
    block comment), outside vendor/ and node_modules/.
  * Not judged: text in backticks and fenced code blocks (code, literal output), text in double quotes (a quotation),
    link targets, URLs, and any word that contains a slash (a path).
  * The patterns are the abbreviations found in this repository, each with the words that make it one, so that a
    proper name (Amazon S3, Cloudflare R2, Apple M3) or a label (milestone M4, namespace N1, priority P1) is not refused.
  * While identifier-scheme.md says `Sweep: pending`, the old identifier forms (T022-064, FR-022-035, ADR-0097, ...) are
    not refused, because the tools still parse them; once it says `Sweep: done`, they are.

Exit codes: 0 nothing found; 1 abbreviations found (each printed with what to write instead); 2 a revision cannot be read.
"""
import argparse
import os
import re
import subprocess
import sys

RULE_PATH = "scripts/check-plain-words.py"
SCHEME_PATH = "docs/governance/identifier-scheme.md"
SWEEP_DONE = re.compile(r"(?m)^Sweep: done\s*$")
PROSE = re.compile(r"(docs|specs)/.*\.md")
REVIEWS = re.compile(r".*\.reviews/.*|specs/[^/]+/reviews/.*")
SKIPPED_DIRS = re.compile(r"(.*/)?(vendor|node_modules|target|dist|\.venv)/.*")
SLASH_COMMENT = (".rs", ".ts", ".tsx", ".js", ".jsx", ".mjs", ".cjs", ".go", ".cs", ".java", ".kt", ".proto")
HASH_COMMENT = (".py", ".sh", ".bash", ".yaml", ".yml", ".toml")
DASH_COMMENT = (".sql",)
FENCE = re.compile(r"^ {0,3}(```|~~~)")

# What is never judged inside a line.
CODE_SPAN = re.compile(r"`[^`]*`")
QUOTED = re.compile(r'"[^"\n]*"|“[^”\n]*”')
LINK_TARGET = re.compile(r"\]\([^)]*\)")
URL = re.compile(r"https?://\S+")
PATH_WORD = re.compile(r"\S*/\S*")

SEVERITY = {"B": "Blocker", "M": "Major", "m": "minor", "N": "Nit"}
# A review finding: a severity letter and a number, after a reviewer or the word "finding". A list continues it.
REVIEW_CONTEXT = (r"architecture|security|correctness|comprehension|structural|style|zoom-out|design critic|review|"
                  r"reviews|reviewer|re-review|re-audit|consolidated|finding|findings|Blocker|Major|minor|Nit")
FINDING = r"(?:r[0-9]+-|A-)?[BMmN][0-9]{1,2}[a-z]?"
LIST_SEPARATOR = r"(?:\s*,\s*(?:and\s+)?|\s+and\s+|\s*[–-]\s*)"
REVIEW_LIST = re.compile(rf"\b(?:{REVIEW_CONTEXT})\b:?\s+(?P<list>{FINDING}(?:{LIST_SEPARATOR}{FINDING})*)\b")
RULE_LIST = re.compile(rf"\b[Rr]ules?\s+(?P<list>R[0-9]{{1,2}}(?:{LIST_SEPARATOR}R[0-9]{{1,2}})*)\b")
RULE_BARE_LIST = re.compile(r"(?<![\w-])(?P<list>R[0-9]{1,2}(?:\s*,\s*R[0-9]{1,2})+)\b")
SLICE_BEFORE = re.compile(rf"\bslices?\s+(?P<list>S[0-9]{{1,2}}[a-z]?(?:{LIST_SEPARATOR}S[0-9]{{1,2}}[a-z]?)*)\b")
SLICE_AFTER = re.compile(r"(?<![\w-])(?P<list>S[0-9]{1,2}[a-z]?)\s+slices?\b")
MUTATION_LIST = re.compile(rf"\b(?:mutations?|mutants?)\s+(?P<list>W[0-9]{{1,3}}(?:{LIST_SEPARATOR}W[0-9]{{1,3}})*)\b")
EGM_FINDING = re.compile(r"(?<![\w-])(?P<list>[ZCKST][0-9]{1,2}-[0-9]{1,3})(?![\w-])")
UNIT_TEST = re.compile(r"(?<![\w-])(?P<list>UT)(?![\w-])")
TOKEN = re.compile(r"(?:r[0-9]+-|A-)?[A-Za-z][0-9]{1,3}(?:-[0-9]{1,3})?[a-z]?|UT")

# The old identifier forms, refused after the sweep, with the new form.
IDENTIFIERS = (
    (re.compile(r"(?<![\w/-])OD-([0-9]{4}-[0-9]{2}-[0-9]{2}-[0-9]+)(?![\w-])"), "Owner decision {0}"),
    (re.compile(r"(?<![\w/-])ADR-([0-9]{4})(?![\w-])"), "Architecture decision {0}"),
    (re.compile(r"(?<![\w/-])NFR-([0-9]{3}-[0-9]+)(?![\w-])"), "Non-functional requirement {0}"),
    (re.compile(r"(?<![\w/-])FR-([0-9]{3}-[0-9]+)(?![\w-])"), "Requirement {0}"),
    (re.compile(r"(?<![\w/-])SC-([0-9]{3}-[0-9]+)(?![\w-])"), "Success criterion {0}"),
    (re.compile(r"(?<![\w/-])US([0-9]{2})(?![\w-])"), "User story NNN-{0} (with its feature)"),
    (re.compile(r"(?<![\w/-])T([0-9]{3}-[0-9]+|[A-Z]{2,8}-[0-9]+)(?![\w-])"), "Task {0}"),
    (re.compile(r"(?<![\w/-])D([0-9]{3}-[0-9]+|[0-9]{3,4})(?![\w-])"), "Decision {0}"),
    (re.compile(r"(?<![\w/-])G([0-9]{3}-[0-9]+|[0-9]{3,4})(?![\w-])"), "Gap {0}"),
    (re.compile(r"(?<![\w/-])O([0-9]{3}-[0-9]+|[0-9]{3,4})(?![\w-])"), "Owner question {0}"),
)


class Untrusted(Exception):
    pass


class Finding:
    def __init__(self, start, token, instead):
        self.start, self.token, self.instead = start, token, instead


def instead_of(token):
    bare = re.sub(r"^(r[0-9]+-|A-)", "", token)
    if bare == "UT":
        return "unit test"
    if re.fullmatch(r"[ZCKST][0-9]{1,2}-[0-9]{1,3}", bare):
        run, number = bare[1:].split("-")
        return f"finding {number} of run {run}, with the reviewer named"
    letter, number = bare[0], re.sub(r"^[A-Za-z]", "", bare)
    if letter == "R":
        return f"Rule {number}"
    if letter == "S":
        return f"slice {number}"
    if letter == "W":
        return f"mutation {number}"
    word = SEVERITY.get(letter, "finding")
    if token.startswith("r"):
        return f"{word} {number} of review run {token[1:token.index('-')]}"
    if token.startswith("A-"):
        return f"{word} {number} of the consolidated review"
    return f"{word} {number}"


def blank(text, pattern):
    return pattern.sub(lambda m: " " * len(m.group(0)), text)


def judged_text(line):
    """The line with what is never judged replaced by spaces (positions are kept)."""
    for pattern in (CODE_SPAN, LINK_TARGET, URL, QUOTED, PATH_WORD):
        line = blank(line, pattern)
    return line


def line_findings(line, identifiers):
    text = judged_text(line)
    found = {}
    for pattern in (REVIEW_LIST, RULE_LIST, RULE_BARE_LIST, SLICE_BEFORE, SLICE_AFTER, MUTATION_LIST, EGM_FINDING,
                    UNIT_TEST):
        for match in pattern.finditer(text):
            base = match.start("list")
            for token in TOKEN.finditer(match.group("list")):
                word = token.group(0)
                # A bare list of R-numbers labels rules, research items or mutants: the words around it say which.
                instead = (f"the word R stands for and {word[1:]} (\"Rule {word[1:]}\" for a rule)"
                           if pattern is RULE_BARE_LIST else instead_of(word))
                found.setdefault(base + token.start(), Finding(base + token.start(), word, instead))
    if identifiers:
        for pattern, form in IDENTIFIERS:
            for match in pattern.finditer(text):
                found.setdefault(match.start(), Finding(match.start(), match.group(0), form.format(match.group(1))))
    return [found[k] for k in sorted(found)]


def in_scope(path):
    """'prose', 'code' or None."""
    if SKIPPED_DIRS.fullmatch(path) or REVIEWS.fullmatch(path):
        return None
    if PROSE.fullmatch(path):
        return "prose"
    if path.endswith(SLASH_COMMENT + HASH_COMMENT + DASH_COMMENT):
        return "code"
    return None


def _after_marker(line, marker, quotes):
    """The text after the first comment marker that is outside a string, or None."""
    quote, index = None, 0
    while index < len(line):
        char = line[index]
        if quote:
            if char == "\\":
                index += 2
                continue
            if char == quote:
                quote = None
        elif char in quotes:
            quote = char
        elif line.startswith(marker, index):
            return line[index + len(marker):]
        index += 1
    return None


def comment_text(path, line, in_block=False):
    """The comment part of one line of code, or None. A line inside a block comment is all comment."""
    stripped = line.strip()
    if path.endswith(SLASH_COMMENT):
        if in_block or stripped.startswith(("*", "/*")):
            return re.sub(r"^\s*/?\*+", "", line).split("*/")[0]
        return _after_marker(line, "//", "\"`")
    if path.endswith(HASH_COMMENT):
        return _after_marker(line, "#", "\"'")
    if path.endswith(DASH_COMMENT):
        return _after_marker(line, "--", "'")
    return None


def judged_lines(path, text):
    """(line number, text to judge) for every line of a file in scope."""
    kind = in_scope(path)
    lines = text.split("\n")
    if kind == "prose":
        fenced = False
        for number, line in enumerate(lines, start=1):
            if FENCE.match(line):
                fenced = not fenced
                continue
            if not fenced:
                yield number, line
    elif kind == "code":
        in_block = False
        slash = path.endswith(SLASH_COMMENT)
        for number, line in enumerate(lines, start=1):
            comment = comment_text(path, line, in_block)
            if slash:
                outside = _after_marker(line, "//", "\"`") is None
                if in_block:
                    in_block = "*/" not in line
                elif outside and "/*" in line and "*/" not in line.split("/*", 1)[1]:
                    in_block = True
            if comment:
                yield number, comment


def git(root, *args, check=True):
    result = subprocess.run(["git", "-C", root, *args], capture_output=True, text=True, errors="replace")
    if check and result.returncode != 0:
        raise Untrusted(f"git {' '.join(args)}: {result.stderr.strip()}")
    return result.stdout


def sweep_done(root, rev):
    text = git(root, "show", f"{rev}:{SCHEME_PATH}", check=False) if rev else None
    if text is None or not rev:
        try:
            with open(os.path.join(root, SCHEME_PATH), encoding="utf-8") as handle:
                text = handle.read()
        except OSError:
            text = ""
    return bool(SWEEP_DONE.search(text or ""))


def pull_request_lines(root, base, head):
    """{path: set of line numbers} added by the judged commits and still present at the head."""
    for rev in (base, head):
        git(root, "rev-parse", "--verify", "--quiet", f"{rev}^{{commit}}")
    shas = [s for s in git(root, "rev-list", "--no-merges", f"{base}..{head}").split("\n") if s]
    judged = {s for s in shas
              if subprocess.run(["git", "-C", root, "cat-file", "-e", f"{s}:{RULE_PATH}"],
                                capture_output=True).returncode == 0}
    paths = set()
    for sha in judged:
        paths.update(p for p in git(root, "diff-tree", "--no-commit-id", "--root", "-r", "--name-only",
                                    "--diff-filter=AMR", sha).split("\n") if p and in_scope(p))
    wanted = {}
    for path in sorted(paths):
        if subprocess.run(["git", "-C", root, "cat-file", "-e", f"{head}:{path}"], capture_output=True).returncode:
            continue
        blame = git(root, "blame", "--porcelain", head, "--", path)
        for header in re.finditer(r"(?m)^([0-9a-f]{40}) [0-9]+ ([0-9]+)", blame):
            if header.group(1) in judged:
                wanted.setdefault(path, set()).add(int(header.group(2)))
    return wanted


def scan(root, rev, wanted, identifiers):
    """Findings as (path, line, Finding) for the wanted lines ({path: set or None for every line})."""
    results = []
    for path, numbers in sorted(wanted.items()):
        if rev:
            text = git(root, "show", f"{rev}:{path}", check=False)
        else:
            try:
                with open(os.path.join(root, path), encoding="utf-8", errors="replace") as handle:
                    text = handle.read()
            except OSError:
                continue
        for number, line in judged_lines(path, text):
            if numbers is None or number in numbers:
                results.extend((path, number, f) for f in line_findings(line, identifiers))
    return results


def main(argv):
    parser = argparse.ArgumentParser(description="Words in full in documents and code comments.")
    parser.add_argument("--base", help="the base of the pull request (a revision)")
    parser.add_argument("--head", default="HEAD", help="the head of the pull request (default: HEAD)")
    parser.add_argument("--all", action="store_true", help="read every line of every file in scope")
    parser.add_argument("--root", default=os.getcwd(), help="repository root (default: the current directory)")
    args = parser.parse_args(argv[1:])
    if not args.all and not args.base:
        parser.error("--base is required unless --all is given")
    try:
        if args.all:
            files = git(args.root, "ls-files").split("\n")
            wanted = {f: None for f in files if f and in_scope(f)}
            identifiers = sweep_done(args.root, None)
            results = scan(args.root, None, wanted, identifiers)
        else:
            wanted = pull_request_lines(args.root, args.base, args.head)
            identifiers = sweep_done(args.root, args.head)
            results = scan(args.root, args.head, wanted, identifiers)
    except Untrusted as error:
        print(f"plain-words: {error}", file=sys.stderr)
        return 2
    for path, number, finding in results:
        print(f"{path}:{number}: \"{finding.token}\" -> write \"{finding.instead}\"")
    read = sum(len(v) if v is not None else 1 for v in wanted.values())
    scope = "files" if args.all else "added lines"
    if results:
        print(f"plain-words: {len(results)} abbreviation(s) in {read} {scope}. Write words in full "
              f"(docs/constitution.md, Words in full; AGENTS.md rule 24; docs/governance/identifier-scheme.md). "
              f"Code, paths and literal output go in backticks; a quotation goes in double quotes.")
        return 1
    print(f"plain-words: ok, {read} {scope} read, old identifier forms "
          f"{'refused' if identifiers else 'allowed until the sweep'}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
