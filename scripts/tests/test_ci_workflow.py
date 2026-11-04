"""Guards the CI policy: integration suites run only when added or modified (run: python3 -m unittest discover scripts/tests).

The `sagat-tests` crate holds the integration, contract, fault, e2e and performance suites. The workflow
selects the targets to run with scripts/select-test-targets.py (job `changes`); a step that runs one of
them without that selection would bring back the full cost on every pull request.
"""
import pathlib
import re
import unittest

WORKFLOW = pathlib.Path(__file__).resolve().parents[2] / ".github" / "workflows" / "ci.yml"


def steps(text):
    """Each step of the workflow as text: a chunk that starts at a line '      - ' (the steps' indentation)."""
    chunks = re.split(r"(?m)^(?=      - )", text)
    return [chunk for chunk in chunks if chunk.startswith("      - ")]


class IntegrationSuitesRunOnlyWhenChanged(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.text = WORKFLOW.read_text()
        cls.steps = steps(cls.text)

    def test_the_selection_job_exists_and_the_heavy_jobs_wait_for_it(self):
        self.assertRegex(self.text, r"(?m)^  changes:$")
        self.assertIn("scripts/select-test-targets.py", self.text)
        for job in ("rust", "sdks", "studio"):
            block = re.search(rf"(?ms)^  {job}:\n(.*?)^  [a-z0-9-]+:\n", self.text + "\n  end:\n")
            assert block is not None, job
            self.assertIn("needs: changes", block.group(1), f"job {job} must wait for the selection")

    def test_the_workspace_run_leaves_the_integration_crate_out(self):
        for line in self.text.splitlines():
            if "cargo test --workspace" in line:
                self.assertIn("--exclude sagat-tests", line, line.strip())

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

    def test_the_upgrade_smoke_and_scale_jobs_are_skipped_for_documentation_only_changes(self):
        for job in ("released-upgrade", "local-environment", "scale"):
            block = re.search(rf"(?ms)^  {job}:\n(.*?)^  [a-z0-9-]+:\n", self.text + "\n  end:\n")
            assert block is not None, job
            self.assertIn("needs: changes", block.group(1), f"job {job} must wait for the selection")
            self.assertIn("if: needs.changes.outputs.code == 'true'", block.group(1), f"job {job} must be gated by `code`")

    def test_a_manual_run_is_possible(self):
        self.assertRegex(self.text, r"(?m)^  workflow_dispatch:")


if __name__ == "__main__":
    unittest.main()
