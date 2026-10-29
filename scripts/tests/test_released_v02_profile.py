"""Adversarial source-boundary checks; no compiler, service or existing tree is modified."""
import importlib.util
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('released_v02_profile', Path(__file__).parents[1] / 'test-released-v02-profile.py')
RUNNER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(RUNNER)


class ReleaseSourceBoundary(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='sagat-v02-source-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / 'source'
        (self.source / 'tests').mkdir(parents=True)
        self.original = {'Cargo.toml': b'original', 'tests/main.rs': b'entry'}
        self.overlays = {'tests/main.rs': b'entry with registration', 'tests/phase.rs': b'phase'}
        for name, data in (self.original | self.overlays).items():
            (self.source / name).write_bytes(data)

    def test_valid_archive_and_declared_registration_are_accounted_for(self):
        manifest = RUNNER.verify_source(self.source, self.original, self.overlays)
        self.assertEqual([item['path'] for item in manifest], ['Cargo.toml', 'tests/main.rs', 'tests/phase.rs'])

    def test_changed_production_source_is_refused(self):
        (self.source / 'Cargo.toml').write_bytes(b'changed')
        with self.assertRaisesRegex(RuntimeError, 'differs'):
            RUNNER.verify_source(self.source, self.original, self.overlays)

    def test_changed_test_registration_is_refused(self):
        (self.source / 'tests/main.rs').write_bytes(b'unaccounted registration')
        with self.assertRaisesRegex(RuntimeError, 'differs'):
            RUNNER.verify_source(self.source, self.original, self.overlays)

    def test_undeclared_source_file_is_refused(self):
        (self.source / 'tests/extra.rs').write_bytes(b'extra')
        with self.assertRaisesRegex(RuntimeError, 'undeclared'):
            RUNNER.verify_source(self.source, self.original, self.overlays)

    def test_matching_external_file_link_is_refused(self):
        outside = self.root / 'external'
        outside.write_bytes(b'original')
        (self.source / 'Cargo.toml').unlink()
        (self.source / 'Cargo.toml').symlink_to(outside)
        with self.assertRaisesRegex(RuntimeError, 'symlink'):
            RUNNER.verify_source(self.source, self.original, self.overlays)

    def test_matching_external_directory_link_is_refused(self):
        outside = self.root / 'external-directory'
        (self.source / 'tests').rename(outside)
        (self.source / 'tests').symlink_to(outside, target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, 'symlink'):
            RUNNER.verify_source(self.source, self.original, self.overlays)

    def test_source_root_link_is_refused(self):
        actual = self.root / 'actual'
        self.source.rename(actual)
        self.source.symlink_to(actual, target_is_directory=True)
        with self.assertRaisesRegex(RuntimeError, 'real directory'):
            RUNNER.verify_source(self.source, self.original, self.overlays)

    def test_current_snapshot_captures_untracked_inputs_and_edits(self):
        repo = self.root / 'current'
        repo.mkdir()
        subprocess.run(['git', 'init', '-q'], cwd=repo, check=True)
        (repo / 'tracked.rs').write_text('tracked')
        (repo / 'untracked.sql').write_text('original')
        subprocess.run(['git', 'add', 'tracked.rs'], cwd=repo, check=True)
        before = RUNNER.current_manifest(repo)
        self.assertEqual({item['path'] for item in before}, {'tracked.rs', 'untracked.sql'})
        (repo / 'untracked.sql').write_text('changed')
        self.assertNotEqual(RUNNER.current_manifest(repo), before)


class SelectedPhaseOutcome(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='sagat-phase-outcome-')
        self.addCleanup(self.temp.cleanup)
        self.log = Path(self.temp.name) / 'phase.txt'
        self.case = 'current::selected'

    def check(self, output):
        self.log.write_text(output)
        RUNNER.verify_selected_phase(self.log, self.case)

    def test_nested_old_phases_do_not_inflate_the_one_outer_case(self):
        self.check('test current::selected ... \nrunning 1 test\ntest old::phase ... ok\n'
                   'test result: ok. 1 passed; 0 failed; 0 ignored; 4 filtered out\n'
                   'ok\ntest result: ok. 1 passed; 0 failed; 0 ignored; 9 filtered out\n')

    def test_zero_matching_cases_are_refused(self):
        with self.assertRaisesRegex(RuntimeError, 'selected'):
            self.check('running 0 tests\ntest result: ok. 0 passed; 0 failed; 0 ignored; 9 filtered out\n')

    def test_successful_old_child_cannot_mask_zero_outer_cases(self):
        with self.assertRaisesRegex(RuntimeError, 'selected'):
            self.check('test current::selected ... \ntest result: ok. 1 passed; 0 failed; 0 ignored;\n'
                       'test result: ok. 0 passed; 0 failed; 0 ignored;\n')

    def test_wrong_name_is_refused_even_when_one_test_passed(self):
        with self.assertRaisesRegex(RuntimeError, 'selected'):
            self.check('test another::case ... ok\ntest result: ok. 1 passed; 0 failed; 0 ignored;\n')

    def test_failed_or_multiple_outer_cases_are_refused(self):
        for result in ('FAILED. 0 passed; 1 failed; 0 ignored;', 'ok. 2 passed; 0 failed; 0 ignored;'):
            with self.subTest(result=result), self.assertRaisesRegex(RuntimeError, 'selected'):
                self.check(f'test current::selected ... ok\ntest result: {result}\n')


class PhaseResultIdentity(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='sagat-phase-result-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.result = self.root / 'outcome' / 'result.json'
        self.result.parent.mkdir()

    def test_stale_generic_cutover_result_cannot_replace_missing_selected_result(self):
        import json
        (self.root / 'result.json').write_text(json.dumps({
            'scenario': 'scoped-event-cutover', 'releaseCommit': RUNNER.RELEASE}))
        with self.assertRaisesRegex(RuntimeError, 'missing'):
            RUNNER.verify_phase_result(self.result, 'applied-lookup')

    def test_wrong_scenario_or_release_is_refused(self):
        import json
        for scenario, release in [('scoped-event-cutover', RUNNER.RELEASE), ('applied-lookup', 'wrong')]:
            self.result.write_text(json.dumps({'scenario': scenario, 'releaseCommit': release}))
            with self.subTest(scenario=scenario, release=release), self.assertRaisesRegex(RuntimeError, 'identity'):
                RUNNER.verify_phase_result(self.result, 'applied-lookup')

    def test_selected_result_is_returned_with_its_content_hash(self):
        import json
        self.result.write_text(json.dumps({'scenario': 'applied-lookup', 'releaseCommit': RUNNER.RELEASE}))
        self.assertEqual(RUNNER.verify_phase_result(self.result, 'applied-lookup'), RUNNER.sha(self.result))


if __name__ == '__main__':
    unittest.main()
