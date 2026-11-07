#!/usr/bin/env python3
"""Decide what the CI runs: the tests of what a change touches, or the whole stack for a release.

Usage: python3 scripts/select-test-targets.py [--event E] [--ref-name N] [--ref-type T] [--base-ref B]
                                              --base SHA [--head REV] [--root DIR]

The rule (docs/constitution.md, CI gates; docs/test-strategy.md):

  * On a pull request the CI runs only the unit tests of the changed packages and of the packages that depend on them,
    the unit tests of the SDK, the Studio and the scripts whose files changed, and the integration suites (the
    `sagat-tests` targets) that were themselves changed. Lints and the other verification jobs do not run.
  * For a release the CI runs everything: every test, every lint, every verification job.

A release is a push to main, release/** or hotfix/**, a push of a tag v*, a pull request into one of those
branches, a manual run (workflow_dispatch), a change of .github/workflows (the workflow is then checked by what
it runs), and any run with no usable base commit (the safe default).

Scoped selection:
  unit tests    the workspace packages that contain a changed file and every package that depends on one of them,
                directly or not (a break in a core package shows in the tests of what is built on it). A change of
                Cargo.toml, Cargo.lock, rust-toolchain.toml, .cargo/, clippy.toml, rustfmt.toml or vendor/ selects every
                package.
  integration   a `sagat-tests` target runs only when it was itself changed: (a) a file of its directory, or its
                [[test]] entry, changed; or (b) a harness it drives changed (frontend/e2e, sdks/*/conformance, examples/,
                runners/lake/ for the lake suites). Nothing else selects a suite: not a changed package that the suite
                uses or that depends on it, and not a shared input (tests/support, tests/fixtures, runners/ other than
                runners/lake/, a contract file under specs/, a dependency of tests/Cargo.toml, Cargo.lock): the
                integration suites are expensive, and the release runs all of them.
  SDKs, Studio  an SDK's unit tests run when a file under sdks/<language>/ changed; the Studio's when a file under
                frontend/ changed; the scripts' when a file under scripts/ changed; the lake runtime's (Python,
                its lock and its image) when a file other than Markdown under runners/lake/ changed.
  nothing       Markdown, docs/, tasks/, .claude/, infra/ and the rest select no test.

It prints the decision and appends it to $GITHUB_OUTPUT:

  full=true|false   crates=<package names|ALL>   sdks=<languages>   frontend=true|false   scripts=true|false
  lake=true|false
  targets=<target names>   any=true|false   all=true|false

Exit codes: 0 decision printed; 2 a manifest or a name cannot be trusted.
"""
import argparse
import os
import re
import subprocess
import sys
import tomllib

TARGET_NAME = re.compile(r"[a-z][a-z0-9_]*")
RELEASE_BRANCH = re.compile(r"main|release/.+|hotfix/.+")
RELEASE_TAG = re.compile(r"v[0-9].*")
SHA = re.compile(r"[0-9a-f]{40}")
MANIFEST = "tests/Cargo.toml"
TESTS_PACKAGE = "sagat-tests"
SDK_LANGUAGES = ("typescript", "python", "go", "dotnet", "java", "rust")
BROWSER_SUITES = ("e2e_workflow_studio", "e2e_developer_experience", "e2e_database_connections")
SDK_SUITES = ("sdk_conformance", "e2e_sdk_multilanguage", "fault_sdk_multilanguage")
LAKE_SUITES = ("integration_iceberg_writes", "fault_iceberg_writes", "e2e_iceberg_writes", "perf_iceberg_writes")
HARNESSES = (
    (re.compile(r"frontend/e2e/"), BROWSER_SUITES),
    (re.compile(r"sdks/[a-z]+/conformance/"), SDK_SUITES),
    (re.compile(r"examples/"), ("e2e_developer_experience",)),
    # The lake runtime is driven by these suites; the contract suite reads the contract fragments, not the runtime.
    (re.compile(r"runners/lake/"), LAKE_SUITES),
)
# Inputs of every Rust package: a change selects the unit tests of every package, and no suite.
RUST_ROOT_INPUTS = re.compile(r"Cargo\.toml|Cargo\.lock|rust-toolchain\.toml|clippy\.toml|rustfmt\.toml|\.cargo/.*|vendor/.*")
# Inputs of the integration suites that cannot be tied to one of them. They select no suite on their own: a pull
# request runs the suites that were changed, and the release runs the rest (OD-2026-11-06-2). The lake runtime is not
# one of them: it is tied to the lake suites (HARNESSES) and to its own job.
SHARED_INPUTS = re.compile(r"runners/(?!lake/).*|specs/.*/contracts/.*")
LAKE_RUNTIME = re.compile(r"runners/lake/.*")
MARKDOWN = re.compile(r".*\.md")


class Untrusted(Exception):
    pass


def git(root, *args):
    return subprocess.run(["git", "-C", root, *args], capture_output=True)


def read_toml(path):
    with open(path, "rb") as handle:
        return tomllib.load(handle)


class Workspace:
    """The packages of the root workspace: where they live, what they are called in code and who depends on whom."""

    def __init__(self, root):
        data = read_toml(os.path.join(root, "Cargo.toml"))
        self.directory, self.ident, self.depends_on = {}, {}, {}
        for member in data["workspace"]["members"]:
            manifest = read_toml(os.path.join(root, member, "Cargo.toml"))
            name = manifest["package"]["name"]
            self.directory[name] = member + "/"
            self.ident[manifest.get("lib", {}).get("name", name.replace("-", "_"))] = name
            self.depends_on[name] = self._dependencies(manifest)
        known = set(self.directory)
        for name in self.depends_on:
            self.depends_on[name] &= known

    @staticmethod
    def _dependencies(manifest):
        found = set()
        keys = ("dependencies", "dev-dependencies", "build-dependencies")
        tables = [manifest.get(key, {}) for key in keys]
        tables += [target.get(key, {}) for target in manifest.get("target", {}).values() for key in keys]
        for table in tables:
            for key, value in table.items():
                found.add(value["package"] if isinstance(value, dict) and "package" in value else key)
        return found

    def owner(self, rel):
        owners = [name for name, directory in self.directory.items() if rel.startswith(directory)]
        return max(owners, key=lambda name: len(self.directory[name])) if owners else None

    def dependents(self, names):
        """These packages and every package that depends on any of them, directly or not."""
        reached = set(names)
        grew = True
        while grew:
            grew = False
            for name, deps in self.depends_on.items():
                if name not in reached and deps & reached:
                    reached.add(name)
                    grew = True
        return reached


def head_targets(root):
    """{target name: path relative to tests/} from the manifest in the working tree."""
    try:
        targets = {entry["name"]: entry["path"] for entry in read_toml(os.path.join(root, MANIFEST))["test"]}
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
    """Targets whose [[test]] entry is new or changed, or None when the manifests cannot be compared and every
    target must run (the safe default of a run that cannot know what changed)."""
    shown = git(root, "show", f"{base}:{MANIFEST}")
    try:
        if shown.returncode != 0:
            raise ValueError("the base has no manifest")
        _, old_tests = parse_manifest(shown.stdout.decode("utf-8"))
        with open(os.path.join(root, MANIFEST), encoding="utf-8") as handle:
            _, new_tests = parse_manifest(handle.read())
    except (OSError, ValueError, KeyError, TypeError, UnicodeDecodeError):
        reasons.append(("every target", f"{MANIFEST} cannot be compared with the base"))
        return None
    # A dependency or a setting of the crate (all of the manifest that is not a [[test]] entry) is shared by the
    # suites: it selects none of them.
    chosen = {name for name, entry in new_tests.items() if old_tests.get(name) != entry and name in targets}
    for name in sorted(chosen):
        reasons.append((name, f"{MANIFEST} adds or changes its [[test]] entry"))
    return chosen


class Decision:
    def __init__(self, targets):
        self.targets = targets
        self.full = False
        self.crates, self.sdks, self.frontend, self.scripts, self.lake = set(), set(), False, False, False
        self.chosen, self.everything, self.reasons = set(), False, []

    def run_everything(self, why):
        self.full = True
        self.everything = True
        self.chosen = set(self.targets)
        self.reasons.append(("every target", why))


def changed_files(root, base, head):
    diff = git(root, "diff", "--name-only", "-z", "--no-renames", base, head)
    if diff.returncode != 0:
        return None
    return [name for name in diff.stdout.decode("utf-8", errors="replace").split("\0") if name]


def scope(root, base, head, targets, workspace):
    """The Decision for the files that differ between base and head."""
    decision = Decision(targets)
    files = changed_files(root, base, head)
    if files is None:
        decision.run_everything("the diff against the base could not be read")
        return decision
    if any(rel.startswith(".github/workflows/") for rel in files):
        decision.run_everything("the workflow itself changed, so it is checked by running all of it")
        return decision
    directories = {name: "tests/" + path.rsplit("/", 1)[0] + "/" if "/" in path else "tests/"
                   for name, path in targets.items()}
    changed_packages = set()
    for rel in files:
        if MARKDOWN.fullmatch(rel):
            continue
        if RUST_ROOT_INPUTS.fullmatch(rel):
            decision.everything = True
            decision.reasons.append(("every package", f"{rel} is an input of every Rust package"))
            continue
        if SHARED_INPUTS.fullmatch(rel):
            # Said in the log so that nobody wonders why no suite ran; the file still goes through the rest, since a
            # runner may be a workspace package whose own unit tests (and its dependents') are due.
            decision.reasons.append(("no suite", f"{rel} is shared by the suites; the release runs them"))
        for language in SDK_LANGUAGES:
            if rel.startswith(f"sdks/{language}/"):
                decision.sdks.add(language)
        if rel.startswith("frontend/"):
            decision.frontend = True
        if rel.startswith("scripts/"):
            decision.scripts = True
        if LAKE_RUNTIME.fullmatch(rel):
            decision.lake = True
        for pattern, names in HARNESSES:
            if pattern.match(rel):
                for name in names:
                    if name in targets:
                        decision.chosen.add(name)
                        decision.reasons.append((name, rel))
        if rel == MANIFEST:
            found = manifest_selection(root, base, targets, decision.reasons)
            if found is None:
                decision.chosen = set(targets)
            else:
                decision.chosen |= found
        elif rel.startswith("tests/"):
            owners = [name for name, directory in directories.items() if rel.startswith(directory)]
            if not owners:
                decision.reasons.append(("no suite", f"{rel} is shared by the suites; the release runs them"))
                continue
            depth = max(len(directories[name]) for name in owners)
            for name in sorted(n for n in owners if len(directories[n]) == depth):
                decision.chosen.add(name)
                decision.reasons.append((name, rel))
        else:
            owner = workspace.owner(rel)
            if owner and owner != TESTS_PACKAGE:
                changed_packages.add(owner)
    if decision.everything:
        decision.crates = {name for name in workspace.directory if name != TESTS_PACKAGE}
    else:
        # The changed packages and everything built on them. The integration crate is never one of them: its suites
        # are chosen by their own files.
        decision.crates = workspace.dependents(changed_packages) - {TESTS_PACKAGE}
    return decision


def full_run_reason(event, ref_name, ref_type, base_ref, base, root):
    if event == "workflow_dispatch":
        return "a manual run"
    if event == "push" and ref_type == "tag" and RELEASE_TAG.fullmatch(ref_name):
        return f"the release tag {ref_name}"
    if event == "push" and ref_type != "tag" and RELEASE_BRANCH.fullmatch(ref_name):
        return f"a push to {ref_name}"
    if event == "pull_request" and RELEASE_BRANCH.fullmatch(base_ref):
        return f"a pull request into {base_ref}"
    if not SHA.fullmatch(base) or set(base) == {"0"}:
        return "no usable base commit"
    if git(root, "cat-file", "-e", f"{base}^{{commit}}").returncode != 0:
        return "the base commit is not in this clone"
    return None


def emit(decision):
    if decision.full:
        crates, sdks = "ALL", list(SDK_LANGUAGES)
        frontend = scripts = lake = True
    else:
        crates = "ALL" if decision.everything else " ".join(sorted(decision.crates))
        sdks, frontend, scripts = sorted(decision.sdks), decision.frontend, decision.scripts
        lake = decision.lake
    grouped = {}
    for name, why in decision.reasons:
        grouped.setdefault(name, []).append(why)
    for name, whys in grouped.items():
        more = f" (+{len(whys) - 1} more)" if len(whys) > 1 else ""
        print(f"select {name}: {whys[0]}{more}")

    def flag(value):
        return "true" if value else "false"

    lines = [
        f"full={flag(decision.full)}",
        f"crates={crates}",
        f"sdks={' '.join(sdks)}",
        f"frontend={flag(frontend)}",
        f"scripts={flag(scripts)}",
        f"lake={flag(lake)}",
        f"targets={' '.join(sorted(decision.chosen))}",
        f"any={flag(decision.chosen)}",
        f"all={flag(decision.everything)}",
    ]
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
    parser.add_argument("--ref-type", default=os.environ.get("GITHUB_REF_TYPE", "branch"))
    parser.add_argument("--base-ref", default=os.environ.get("GITHUB_BASE_REF", ""))
    parser.add_argument("--base", default="")
    parser.add_argument("--head", default="HEAD")
    args = parser.parse_args(argv[1:])
    try:
        targets = head_targets(args.root)
        workspace = Workspace(args.root)
    except (Untrusted, OSError, KeyError, ValueError, TypeError) as error:
        message = error if isinstance(error, Untrusted) else "the workspace cannot be read"
        print(f"select-test-targets: {message}", file=sys.stderr)
        return 2
    reason = full_run_reason(args.event, args.ref_name, args.ref_type, args.base_ref, args.base, args.root)
    if reason:
        decision = Decision(targets)
        decision.run_everything(reason)
    else:
        decision = scope(args.root, args.base, args.head, targets, workspace)
    emit(decision)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
