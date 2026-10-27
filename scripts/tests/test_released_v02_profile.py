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


if __name__ == '__main__':
    unittest.main()
