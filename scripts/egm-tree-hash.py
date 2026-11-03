#!/usr/bin/env python3
"""Print the content hash of an installed EGM plugin directory.

Usage: python3 scripts/egm-tree-hash.py <plugin-dir>

The hash is the SHA-256 of the lines "<path>\\0<mode>\\0<sha256 of the file>\\n", sorted by
path, over every file of the directory except the exclusions below. <mode> is 755 when the
file has any executable bit and 644 otherwise. A file that Claude Code
could load as a hook, command, MCP server or agent therefore changes the hash. Point it at the
install directory (~/.claude/plugins/cache/egm/egm/<version>), never at the marketplace clone.

Exit codes: 0 hash printed; 2 the directory is not a plugin install (no
.claude-plugin/plugin.json), is unreadable, or holds a symbolic link.
"""
import hashlib
import os
import sys

TOP_LEVEL_EXCLUDED = {".git", ".github", ".in_use", "site", "tests", "distributions"}
EXCLUDED_PYCACHE = "__pycache__"
ASSET_DIR = "assets"
ASSET_EXTENSIONS = {".svg", ".woff2", ".png", ".jpg", ".jpeg", ".gif", ".ico", ".webp"}
EXCLUDED_FILE_NAMES = {"TEST.md", "test_scenarios.yaml"}
MARKER = os.path.join(".claude-plugin", "plugin.json")


class Refusal(Exception):
    pass


def excluded(rel_parts):
    """True when the path (a tuple of components) is outside the hashed set."""
    if rel_parts[0] in TOP_LEVEL_EXCLUDED:
        return True
    if EXCLUDED_PYCACHE in rel_parts[:-1]:
        return True
    if ASSET_DIR in rel_parts[:-1] and os.path.splitext(rel_parts[-1])[1] in ASSET_EXTENSIONS:
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

    def unreadable(error):
        raise Refusal(f"cannot read a directory: {error.strerror}")

    for current, dirs, files in os.walk(root, followlinks=False, onerror=unreadable):
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
            mode = "755" if os.stat(full).st_mode & 0o111 else "644"
            entries.append(("/".join(parts), mode, file_digest(full)))
    entries.sort(key=lambda entry: os.fsencode(entry[0]))
    summary = hashlib.sha256()
    for rel, mode, digest in entries:
        summary.update(os.fsencode(rel) + b"\0" + mode.encode() + b"\0" + digest.encode() + b"\n")
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
