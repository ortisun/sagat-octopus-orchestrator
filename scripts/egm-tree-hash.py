#!/usr/bin/env python3
"""Print the content hash of an installed EGM plugin directory.

Usage: python3 scripts/egm-tree-hash.py <plugin-dir>

The hash is the SHA-256 of the lines "<relative path>\\0<sha256 of the file>\\n", sorted by
path, over every file of the directory except the exclusions below. A file that Claude Code
could load as a hook, command, MCP server or agent therefore changes the hash. Point it at the
install directory (~/.claude/plugins/cache/egm/egm/<version>), never at the marketplace clone.

Exit codes: 0 hash printed; 2 the directory is not a plugin install (no
.claude-plugin/plugin.json), is unreadable, or holds a symbolic link.
"""
import hashlib
import os
import sys

TOP_LEVEL_EXCLUDED_DIRS = {".git", ".github", "site", "tests", "distributions"}
EXCLUDED_ROOT_FILES = {".in_use"}
EXCLUDED_DIR_NAMES = {"assets", "__pycache__"}
EXCLUDED_FILE_NAMES = {"TEST.md", "test_scenarios.yaml"}
MARKER = os.path.join(".claude-plugin", "plugin.json")


class Refusal(Exception):
    pass


def excluded(rel_parts):
    """True when the path (a tuple of components) is outside the hashed set."""
    if len(rel_parts) == 1 and rel_parts[0] in EXCLUDED_ROOT_FILES:
        return True
    if len(rel_parts) > 1 and rel_parts[0] in TOP_LEVEL_EXCLUDED_DIRS:
        return True
    if any(part in EXCLUDED_DIR_NAMES for part in rel_parts[:-1]):
        return True
    return rel_parts[-1] in EXCLUDED_FILE_NAMES


def file_digest(path):
    digest = hashlib.sha256()
    try:
        with open(path, "rb") as handle:
            for chunk in iter(lambda: handle.read(1 << 20), b""):
                digest.update(chunk)
    except OSError as error:
        raise Refusal(f"cannot read {path}: {error.strerror}") from error
    return digest.hexdigest()


def tree_hash(root):
    if not os.path.isdir(root):
        raise Refusal(f"{root} is not a directory")
    if not os.path.isfile(os.path.join(root, MARKER)):
        raise Refusal(f"{MARKER} is missing: {root} is not an installed EGM plugin")
    entries = []
    for current, dirs, files in os.walk(root, followlinks=False):
        for name in list(dirs) + files:
            full = os.path.join(current, name)
            if os.path.islink(full):
                raise Refusal(f"symbolic link refused: {os.path.relpath(full, root)}")
        dirs.sort()
        for name in files:
            full = os.path.join(current, name)
            rel = os.path.relpath(full, root)
            parts = tuple(rel.split(os.sep))
            if excluded(parts):
                continue
            entries.append(("/".join(parts), file_digest(full)))
    entries.sort()
    summary = hashlib.sha256()
    for rel, digest in entries:
        summary.update(f"{rel}\0{digest}\n".encode())
    return summary.hexdigest()


def main(argv):
    if len(argv) != 2:
        print("usage: egm-tree-hash.py <plugin-dir>", file=sys.stderr)
        return 2
    try:
        print(tree_hash(argv[1]))
    except Refusal as error:
        print(f"egm-tree-hash: {error}", file=sys.stderr)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
