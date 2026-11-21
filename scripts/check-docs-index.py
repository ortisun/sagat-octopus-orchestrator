#!/usr/bin/env python3
"""Keep docs/ in order: every folder has an index, and every index links everything in its folder.

Usage: python3 scripts/check-docs-index.py [--root DIR]

The rule (docs/README.md, "How this folder is kept in order"; constitution, Documentation first: "an index in every
folder"):

  * Every folder under docs/ has a README.md, its index.
  * Every index links every file and every folder directly inside its folder (a Markdown link or image, inline or
    reference style, or an HTML `src`/`href`). A folder counts as linked by its path, with or without a trailing slash,
    or by its README.md. A link inside a code fence or a code span does not count.
  * docs/README.md, the index of docs/ itself, therefore links every folder and every file at the top of docs/.
  * Review reports are evidence of a design doc, not pages to browse: a folder whose name ends in `.reviews`, and
    everything under it, needs no index and need not be linked.
  * A relative link in an index that resolves to nothing is reported too.

Files are those git tracks or would track (untracked files that are not ignored count, so a new page is caught before it
is committed).

Exit codes: 0 in order; 1 something is missing (each printed); 2 no docs/ folder, or not a git repository.
"""
import argparse
import os
import posixpath
import re
import subprocess
import sys
from urllib.parse import unquote

INDEX = "README.md"
FENCE = re.compile(r"^ {0,3}(```|~~~)")
CODE_SPAN = re.compile(r"`[^`\n]*`")
INLINE_LINK = re.compile(r"!?\[[^\]]*\]\(\s*<?([^)\s>]+)>?(?:\s+\"[^\"]*\")?\s*\)")
REFERENCE_DEFINITION = re.compile(r"^ {0,3}\[[^\]]+\]:\s*<?(\S+?)>?(?:\s+.*)?$")
HTML_LINK = re.compile(r"(?:src|href)=\"([^\"]+)\"")
EXTERNAL = re.compile(r"[a-z][a-z0-9+.-]*:", re.I)


def is_review(path):
    return any(part.endswith(".reviews") for part in path.split("/"))


def files_of(root):
    result = subprocess.run(["git", "-C", root, "ls-files", "--cached", "--others", "--exclude-standard", "docs"],
                            capture_output=True, text=True)
    if result.returncode != 0:
        raise SystemExit(2)
    return sorted({f for f in result.stdout.split("\n") if f and os.path.exists(os.path.join(root, f))})


def link_targets(text):
    """Every link target of a Markdown text, outside code fences and code spans."""
    targets, fenced = [], False
    for line in text.split("\n"):
        if FENCE.match(line):
            fenced = not fenced
            continue
        if fenced:
            continue
        line = CODE_SPAN.sub("", line)
        targets.extend(INLINE_LINK.findall(line))
        targets.extend(HTML_LINK.findall(line))
        definition = REFERENCE_DEFINITION.match(line)
        if definition:
            targets.append(definition.group(1))
    return targets


def resolve(folder, target):
    """The repository path a relative link points to, or None for an external link or a bare anchor."""
    if EXTERNAL.match(target) or target.startswith(("#", "/")):
        return None
    target = unquote(target.split("#", 1)[0].split("?", 1)[0])
    if not target:
        return None
    return posixpath.normpath(posixpath.join(folder, target))


def check(root):
    files = files_of(root)
    if not any(f.startswith("docs/") for f in files):
        print("check-docs-index: no docs/ folder", file=sys.stderr)
        raise SystemExit(2)
    existing = set(files)
    folders = {"docs"}
    for path in files:
        parent = posixpath.dirname(path)
        while parent.startswith("docs/"):
            folders.add(parent)
            parent = posixpath.dirname(parent)
    folders = {f for f in folders if not is_review(f)}
    errors = []
    for folder in sorted(folders):
        index = f"{folder}/{INDEX}"
        if index not in existing:
            errors.append(f"{folder}: has no {INDEX} (every folder of docs/ has an index)")
            continue
        with open(os.path.join(root, index), encoding="utf-8") as handle:
            text = handle.read()
        linked = set()
        for target in link_targets(text):
            path = resolve(folder, target)
            if path is None:
                continue
            exists = (path in existing or path in folders or any(f.startswith(path + "/") for f in existing)
                      or (not path.startswith("docs/") and os.path.exists(os.path.join(root, path))))
            if not exists:
                errors.append(f"{index}: the link {target} does not exist")
                continue
            linked.add(path[:-len(INDEX) - 1] if path.endswith("/" + INDEX) else path)
        children = {f for f in existing if posixpath.dirname(f) == folder and f != index}
        children |= {d for d in folders if posixpath.dirname(d) == folder}
        for child in sorted(children):
            if child not in linked:
                kind = "folder" if child in folders else "file"
                errors.append(f"{index}: does not link the {kind} {posixpath.basename(child)}")
    return errors


def main(argv):
    parser = argparse.ArgumentParser(description="Every folder of docs/ has an index that links everything in it.")
    parser.add_argument("--root", default=os.getcwd(), help="repository root (default: the current directory)")
    args = parser.parse_args(argv[1:])
    errors = check(args.root)
    if errors:
        print("\n".join(errors))
        print(f"check-docs-index: {len(errors)} problem(s). Add the page to its folder's README.md, in the right "
              f"section and with one line on what it holds (docs/README.md, How this folder is kept in order).")
        return 1
    print("docs index ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
