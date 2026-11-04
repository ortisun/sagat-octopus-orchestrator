#!/usr/bin/env python3
"""Select the integration test targets of tests/Cargo.toml that a change adds or modifies.

Usage: python3 scripts/select-test-targets.py --event EVENT --ref-name REF --base SHA [--head REV] [--root DIR]

The CI runs the suites of the `sagat-tests` crate (integration, contract, fault, e2e, performance; they
start PostgreSQL through Testcontainers) only for the targets whose files changed, and all of them when
the change cannot be narrowed. It prints the selection and appends it to $GITHUB_OUTPUT:

  targets=<space-separated target names>   any=true|false   all=true|false

A changed file selects, by the deepest target directory that contains it:
  * a file under tests/<kind>/<feature>/ : that target (a nested target such as .../rest/ wins over its parent);
  * a file under tests/ that no target directory contains (tests/support, tests/fixtures, ...): every target;
  * tests/Cargo.toml: the [[test]] entries that are new or changed; every target if anything else but the
    package description changed;
  * Markdown under tests/: nothing;
  * the harnesses the suites drive from outside tests/ (frontend/e2e, sdks/*/conformance): the suites that run them.
Every target runs for a manual run, for a push to main, release/** or hotfix/**, and whenever no usable base
commit is given (the safe default).

Exit codes: 0 selection printed; 2 the manifest or a target name cannot be trusted.
"""
import argparse
import os
import re
import subprocess
import sys
import tomllib

TARGET_NAME = re.compile(r"[a-z][a-z0-9_]*")
FULL_REFS = re.compile(r"main|release/.+|hotfix/.+")
SHA = re.compile(r"[0-9a-f]{40}")
MANIFEST = "tests/Cargo.toml"
BROWSER_SUITES = ("e2e_workflow_studio", "e2e_developer_experience", "e2e_database_connections")
SDK_SUITES = ("sdk_conformance", "e2e_sdk_multilanguage", "fault_sdk_multilanguage")
HARNESSES = (
    (re.compile(r"frontend/e2e/"), BROWSER_SUITES),
    (re.compile(r"sdks/[a-z]+/conformance/"), SDK_SUITES),
)


class Untrusted(Exception):
    pass


def git(root, *args):
    return subprocess.run(["git", "-C", root, *args], capture_output=True)


def head_targets(root):
    """{target name: path relative to tests/} from the manifest in the working tree."""
    try:
        with open(os.path.join(root, MANIFEST), "rb") as handle:
            data = tomllib.load(handle)
        targets = {entry["name"]: entry["path"] for entry in data["test"]}
    except (OSError, ValueError, KeyError, TypeError):
        raise Untrusted("tests/Cargo.toml cannot be read as a list of [[test]] targets")
    for name, path in targets.items():
        if not (isinstance(name, str) and TARGET_NAME.fullmatch(name) and isinstance(path, str)):
            raise Untrusted("a [[test]] target has a name outside [a-z][a-z0-9_]* or a path that is not text")
    return targets


def parse_manifest(text):
    data = tomllib.loads(text)
    tests = {entry["name"]: entry for entry in data.get("test", [])}
    rest = {key: value for key, value in data.items() if key != "test"}
    rest.get("package", {}).pop("description", None)
    return rest, tests


def manifest_selection(root, base, targets, reasons):
    """Targets whose [[test]] entry is new or changed, or None when every target must run."""
    shown = git(root, "show", f"{base}:{MANIFEST}")
    try:
        if shown.returncode != 0:
            raise ValueError("the base has no manifest")
        old_rest, old_tests = parse_manifest(shown.stdout.decode("utf-8"))
        with open(os.path.join(root, MANIFEST), encoding="utf-8") as handle:
            new_rest, new_tests = parse_manifest(handle.read())
    except (OSError, ValueError, KeyError, TypeError, UnicodeDecodeError):
        reasons.append(("every target", f"{MANIFEST} cannot be compared with the base"))
        return None
    if old_rest != new_rest:
        reasons.append(("every target", f"{MANIFEST} changed beyond its [[test]] entries"))
        return None
    chosen = {name for name, entry in new_tests.items() if old_tests.get(name) != entry and name in targets}
    for name in sorted(chosen):
        reasons.append((name, f"{MANIFEST} adds or changes its [[test]] entry"))
    return chosen


def select(root, base, head, targets):
    """(chosen names, reasons, whether every target runs) for the files that differ between base and head."""
    changed = git(root, "diff", "--name-only", "-z", "--no-renames", base, head)
    if changed.returncode != 0:
        return set(targets), [("every target", "the diff against the base could not be read")], True
    files = [name for name in changed.stdout.decode("utf-8", errors="replace").split("\0") if name]
    directories = {name: "tests/" + path.rsplit("/", 1)[0] + "/" if "/" in path else "tests/" for name, path in targets.items()}
    chosen, reasons = set(), []
    for rel in files:
        if rel == MANIFEST:
            found = manifest_selection(root, base, targets, reasons)
            if found is None:
                return set(targets), reasons, True
            chosen |= found
        elif rel.startswith("tests/"):
            if rel.endswith(".md"):
                continue
            owners = [name for name, directory in directories.items() if rel.startswith(directory)]
            if not owners:
                reasons.append(("every target", f"{rel} is shared by the suites"))
                return set(targets), reasons, True
            depth = max(len(directories[name]) for name in owners)
            for name in sorted(n for n in owners if len(directories[n]) == depth):
                chosen.add(name)
                reasons.append((name, rel))
        else:
            for pattern, names in HARNESSES:
                if pattern.match(rel):
                    for name in names:
                        if name in targets:
                            chosen.add(name)
                            reasons.append((name, rel))
    return chosen, reasons, False


def full_run_reason(event, ref_name, base, root):
    if event == "workflow_dispatch":
        return "a manual run"
    if event == "push" and FULL_REFS.fullmatch(ref_name):
        return f"a push to {ref_name}"
    if not SHA.fullmatch(base) or set(base) == {"0"}:
        return "no usable base commit"
    if git(root, "cat-file", "-e", f"{base}^{{commit}}").returncode != 0:
        return "the base commit is not in this clone"
    return None


def emit(chosen, everything, reasons):
    grouped = {}
    for name, why in reasons:
        grouped.setdefault(name, []).append(why)
    for name, whys in grouped.items():
        more = f" (+{len(whys) - 1} more)" if len(whys) > 1 else ""
        print(f"select {name}: {whys[0]}{more}")
    names = " ".join(sorted(chosen))
    lines = [f"targets={names}", f"any={'true' if chosen else 'false'}", f"all={'true' if everything else 'false'}"]
    for line in lines:
        print(line)
    path = os.environ.get("GITHUB_OUTPUT")
    if path:
        with open(path, "a", encoding="utf-8") as handle:
            handle.write("\n".join(lines) + "\n")


def main(argv):
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    parser.add_argument("--root", default=os.getcwd())
    parser.add_argument("--event", default=os.environ.get("GITHUB_EVENT_NAME", ""))
    parser.add_argument("--ref-name", default=os.environ.get("GITHUB_REF_NAME", ""))
    parser.add_argument("--base", default="")
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args(argv[1:])
    try:
        targets = head_targets(args.root)
    except Untrusted as error:
        print(f"select-test-targets: {error}", file=sys.stderr)
        return 2
    reason = full_run_reason(args.event, args.ref_name, args.base, args.root)
    if reason:
        emit(set(targets), True, [("every target", reason)])
        return 0
    chosen, reasons, everything = select(args.root, args.base, args.head, targets)
    emit(chosen, everything, reasons)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
