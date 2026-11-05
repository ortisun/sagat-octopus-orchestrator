"""Guards the CI rule (run: python3 -m unittest discover scripts/tests).

The rule (docs/constitution.md, CI gates): a pull request runs only the unit tests of the changed packages and the
integration suites that touch the affected code, a merge into develop starts no run, and a release runs the whole
stack, lints included. The job `changes` runs scripts/select-test-targets.py and every other job is gated by its outputs. A step
that ran a lint, a verification job or an integration suite without that gate would bring the full cost back on
every pull request, or hide the release check.
"""
import pathlib
import re
import unittest

WORKFLOW = pathlib.Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"
FULL = "needs.changes.outputs.full == 'true'"
# Jobs that only a release runs: lints, verification of the product as a whole, advisories.
RELEASE_ONLY_JOBS = ("released-upgrade", "native-runners", "contracts", "local-environment", "observability",
                     "scale", "dependencies")
# Jobs whose steps are scoped by the change (and run whole for a release).
SCOPED_JOBS = ("rust", "sdks", "studio", "architecture", "lake-runtime")
LAKE = "needs.changes.outputs.lake == 'true'"
AUDIT_WORKFLOW = WORKFLOW.parent / "lake-audit.yml"


def steps(text):
    """Each step of the workflow as text: a chunk that starts at a line '      - ' (the steps' indentation)."""
    chunks = re.split(r"(?m)^(?=      - )", text)
    return [chunk for chunk in chunks if chunk.startswith("      - ")]


def job_block(text, job):
    block = re.search(rf"(?ms)^  {job}:\n(.*?)^  [a-z0-9-]+:\n", text + "\n  end:\n")
    assert block is not None, f"job {job} not found"
    return block.group(1)


class CiRule(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text()
        cls.steps = steps(cls.text)

    def test_the_selection_job_exists_and_every_other_job_waits_for_it(self):
        self.assertRegex(self.text, r"(?m)^  changes:$")
        self.assertIn("scripts/select-test-targets.py", self.text)
        for job in (*SCOPED_JOBS, *RELEASE_ONLY_JOBS):
            self.assertIn("needs: changes", job_block(self.text, job), f"job {job} must wait for the selection")

    def test_the_lints_and_verification_jobs_run_only_for_a_release(self):
        for job in RELEASE_ONLY_JOBS:
            self.assertRegex(job_block(self.text, job), rf"(?m)^    if: {re.escape(FULL)}\s*$",
                             f"job {job} must run only for a release")

    def test_the_formatting_and_lint_steps_run_only_for_a_release(self):
        # The release-only jobs are gated as a whole (test above); the steps of the scoped jobs one by one.
        found = 0
        for job in SCOPED_JOBS:
            for step in steps(job_block(self.text, job)):
                commands = [l for l in step.splitlines() if not l.lstrip().startswith("#")]
                if any(re.search(r"cargo (fmt|clippy)\b|npm run (lint|typecheck)\b|check-[a-z-]+\.py|\bmypy\b|\bruff\b|go vet|gofmt|dotnet format", l)
                       for l in commands):
                    found += 1
                    self.assertRegex(step, rf"(?m)^        if: {re.escape(FULL)}\s*$",
                                     f"this lint step of job {job} must run only for a release:\n{step}")
        self.assertGreater(found, 5, "the test no longer finds the lint steps it is meant to guard")

    def test_the_workspace_unit_tests_leave_the_integration_crate_out(self):
        for line in self.text.splitlines():
            if "cargo test --workspace" in line:
                self.assertIn("--exclude sagat-tests", line, line.strip())

    def test_the_unit_tests_are_those_of_the_changed_packages(self):
        step = next(s for s in self.steps if s.startswith("      - name: unit tests of the changed packages"))
        self.assertIn("needs.changes.outputs.crates", step)
        self.assertIn('[ "$CRATES" = ALL ]', step)

    def test_every_step_that_runs_an_integration_suite_is_gated_by_the_selection(self):
        found = 0
        for step in self.steps:
            # a comment may mention the crate; only commands count
            commands = [l for l in step.splitlines()
                        if "cargo test" in l and "sagat-tests" in l and "--exclude sagat-tests" not in l
                        and not l.lstrip().startswith("#")]
            if not commands:
                continue
            found += 1
            gated = re.search(r"(?m)^        if: .*needs\.changes\.outputs", step) or "needs.changes.outputs.targets" in step
            self.assertTrue(gated, f"this step runs sagat-tests without the selection:\n{step}")
        self.assertGreater(found, 0, "the test no longer finds the steps it is meant to guard")

    def test_the_sdk_unit_tests_follow_the_changed_languages(self):
        for language in ("TypeScript", "Python", "Go", "C#", "Java", "Rust"):
            step = next(s for s in self.steps if s.startswith(f"      - name: {language} SDK\n"))
            self.assertIn("needs.changes.outputs.sdks", step, f"{language} SDK must follow the changed languages")

    def test_the_studio_unit_tests_follow_the_frontend_and_the_rest_is_for_a_release(self):
        unit = next(s for s in self.steps if s.startswith("      - name: unit tests\n"))
        self.assertIn("needs.changes.outputs.frontend", unit)
        build = next(s for s in self.steps if s.startswith("      - name: lint, types, build"))
        self.assertRegex(build, rf"(?m)^        if: {re.escape(FULL)}\s*$")

    def test_the_selection_hands_out_the_lake_gate(self):
        self.assertIn("lake: ${{ steps.select.outputs.lake }}", self.text)

    def test_the_lake_runtime_job_follows_the_lake_gate(self):
        block = job_block(self.text, "lake-runtime")
        self.assertIn("needs: changes", block)
        self.assertRegex(block, rf"(?m)^    if: {re.escape(LAKE)}\s*$")
        unit = next(s for s in steps(block) if "pytest" in s)
        self.assertNotRegex(unit, r"(?m)^        if:", "the unit tests of the lake runtime run whenever the gate is open")
        audit = next(s for s in steps(block) if "pip-audit" in s)
        self.assertIn("--require-hashes", audit)
        self.assertNotRegex(audit, r"(?m)^        if:", "the audit of the lock runs with every change of the runtime")

    def test_the_lake_runtime_image_and_its_sbom_are_built_only_for_a_release(self):
        found = 0
        for step in steps(job_block(self.text, "lake-runtime")):
            commands = [l for l in step.splitlines() if not l.lstrip().startswith("#")]
            if any(re.search(r"docker (buildx )?(build|create)|smoke-image\.sh|check_sbom", l) for l in commands):
                found += 1
                self.assertRegex(step, rf"(?m)^        if: {re.escape(FULL)}\s*$",
                                 f"this image step must run only for a release:\n{step}")
        self.assertGreaterEqual(found, 3, "the image, its smoke test and its SBOM are all built")

    def test_the_lake_image_is_built_with_pinned_inputs(self):
        block = job_block(self.text, "lake-runtime")
        self.assertRegex(block, r"image=moby/buildkit:v[0-9.]+@sha256:[0-9a-f]{64}", "the builder image is pinned")
        self.assertRegex(block, r"buildkit-syft-scanner:[0-9]+@sha256:[0-9a-f]{64}", "the SBOM scanner is pinned")

    def test_the_lake_lock_is_audited_every_week_by_a_workflow_of_its_own(self):
        audit = AUDIT_WORKFLOW.read_text()
        self.assertRegex(audit, r"(?m)^  schedule:\n    - cron: ")
        self.assertRegex(audit, r"(?m)^  workflow_dispatch:")
        self.assertRegex(audit, r"(?ms)^permissions:\n  contents: read\n")
        self.assertIn("pip-audit", audit)
        self.assertIn("runners/lake/requirements.lock", audit)
        for action in re.findall(r"(?m)^\s*(?:- )?uses: (\S+)", audit):
            self.assertRegex(action, r"@[0-9a-f]{40}$", f"{action} must be pinned to a commit")

    def test_a_release_can_be_triggered_by_a_tag_a_branch_and_by_hand(self):
        self.assertRegex(self.text, r'(?m)^    tags: \["v\*"\]')
        self.assertRegex(self.text, r'(?m)^    branches: \[main, "release/\*\*", "hotfix/\*\*"\]')
        self.assertRegex(self.text, r"(?m)^  workflow_dispatch:")

    def test_a_merge_into_develop_starts_no_run_and_a_pull_request_still_does(self):
        """The owner's rule (OD-2026-11-05-1): the CI runs for a pull request and for a release, and a merge into
        develop is neither: what the pull request proved is not proved again by the push that merges it."""
        triggers = self.text.split("\njobs:", 1)[0]
        pushed = re.search(r"(?ms)^  push:\n(.*?)^  \S", triggers)
        self.assertIsNotNone(pushed, "the workflow has a push trigger")
        self.assertNotIn("develop", pushed.group(1), "a push to develop starts the CI")
        self.assertRegex(triggers, r"(?m)^  pull_request:")


if __name__ == "__main__":
    unittest.main()
