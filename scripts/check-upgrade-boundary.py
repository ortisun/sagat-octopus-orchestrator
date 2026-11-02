#!/usr/bin/env python3
"""Check current-schema upgrade oracles before compiling the genuine released producer.

Only explicit current `after` boundaries are inspected here. Genuine released-source runtime
checks own the immutable old schema/commit evidence and remain required. This is a lexical
source check, not a Rust parser or proof of macro expansion/control-flow reachability.
"""
from pathlib import Path
import re

CURRENT_ORACLE = re.compile(r"\bassert_eq!\s*\(\s*after\s*,\s*([0-9][0-9_]*)\s*,?\s*\)")
RAW_STRING = re.compile(r'(?:br|cr|r)(#*)"')
CHAR = re.compile(r"(?:b)?'(?:\\(?:x[0-9a-fA-F]{2}|u\{[0-9a-fA-F_]+\}|.)|[^'\\\r\n])'")

PROFILES = (
    "release_upgrade.rs",
    "release_upgrade_inflight.rs",
    "release_upgrade_callback_cutover.rs",
)


def code_only(source):
    """Remove comments and literals; preserve lifetimes and executable numeric oracles."""
    code = []
    i = 0
    while i < len(source):
        if source.startswith("//", i):
            end = source.find("\n", i)
            i = len(source) if end < 0 else end
        elif source.startswith("/*", i):
            depth = 1
            i += 2
            while i < len(source) and depth:
                if source.startswith("/*", i):
                    depth += 1
                    i += 2
                elif source.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
        elif (i == 0 or not (source[i - 1].isalnum() or source[i - 1] == "_")) and (raw := RAW_STRING.match(source, i)):
            closing = '"' + raw.group(1)
            end = source.find(closing, raw.end())
            i = len(source) if end < 0 else end + len(closing)
        elif source[i] == '"':
            i += 1
            while i < len(source):
                if source[i] == "\\":
                    i += 2
                elif source[i] == '"':
                    i += 1
                    break
                else:
                    i += 1
        elif char := CHAR.match(source, i):
            i = char.end()
        else:
            code.append(source[i])
            i += 1
            continue
        code.append(" ")
    return "".join(code)


def check_boundaries(sources, latest):
    errors = []
    for name, source in sorted(sources.items()):
        matches = CURRENT_ORACLE.findall(code_only(source))
        if len(matches) != 1:
            errors.append(f"{name}: expected one explicit current-schema `after` oracle")
        elif int(matches[0].replace("_", "")) != latest:
            errors.append(f"{name}: current-schema oracle {matches[0]} must match migration {latest}")
    return errors


def main():
    root = Path(__file__).resolve().parents[1]
    versions = [int(p.name[:14]) for p in
                (root / "crates/infra-postgres/migrations").glob("[0-9]" * 14 + "_*.up.sql")]
    if not versions:
        print("upgrade boundary: no current UP migrations")
        return 1
    sources = {name: (root / "tests/integration/workflow-primitives" / name).read_text()
               for name in PROFILES}
    errors = check_boundaries(sources, max(versions))
    for error in errors:
        print(error)
    if not errors:
        print(f"upgrade current-schema boundaries match migration {max(versions)}")
    return bool(errors)


if __name__ == "__main__":
    raise SystemExit(main())
