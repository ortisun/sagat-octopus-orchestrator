"""Negative controls for unnoticed edits to the temporary third-party snapshot."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location('check_vendor', Path(__file__).parents[1] / 'check-vendor.py')
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


class VendorIntegrity(unittest.TestCase):
    def setUp(self):
        temp = tempfile.TemporaryDirectory(prefix='sagat-vendor-integrity-')
        self.addCleanup(temp.cleanup)
        self.root = Path(temp.name) / 'snapshot'
        (self.root / 'src').mkdir(parents=True)
        original = {'Cargo.toml': CHECK.OLD, 'src/lib.rs': b'upstream source'}
        for name, data in original.items():
            (self.root / name).write_bytes(CHECK.NEW if name == 'Cargo.toml' else data)
        self.provenance = self.root.parent / 'provenance.json'
        metadata = {
            'package': 'rskafka', 'version': '0.6.0', 'registryArchiveSHA256': CHECK.ARCHIVE_SHA256,
            'files': {name: hashlib.sha256(data).hexdigest() for name, data in original.items()},
        }
        self.provenance.write_text(json.dumps(metadata))
        digest = hashlib.sha256(json.dumps(metadata['files'], sort_keys=True, separators=(',', ':')).encode()).hexdigest()
        for name, value in [('FILES_SHA256', digest), ('FILE_COUNT', 2)]:
            patched = patch.object(CHECK, name, value)
            patched.start()
            self.addCleanup(patched.stop)

    def test_only_the_exact_manifest_override_is_accepted(self):
        self.assertEqual(CHECK.verify(self.root, self.provenance), 2)

    def test_source_modification_is_refused(self):
        (self.root / 'src/lib.rs').write_bytes(b'changed source')
        with self.assertRaisesRegex(ValueError, 'snapshot changed'):
            CHECK.verify(self.root, self.provenance)

    def test_source_and_provenance_changed_together_are_refused(self):
        source = self.root / 'src/lib.rs'
        source.write_bytes(b'changed source')
        metadata = json.loads(self.provenance.read_text())
        metadata['files']['src/lib.rs'] = hashlib.sha256(source.read_bytes()).hexdigest()
        self.provenance.write_text(json.dumps(metadata))
        with self.assertRaises(ValueError):
            CHECK.verify(self.root, self.provenance)

    def test_missing_security_override_is_refused(self):
        (self.root / 'Cargo.toml').write_bytes(CHECK.OLD)
        with self.assertRaisesRegex(ValueError, 'reviewed zstd override'):
            CHECK.verify(self.root, self.provenance)

    def test_extra_or_missing_files_are_refused(self):
        (self.root / 'src/extra.rs').write_bytes(b'not upstream')
        with self.assertRaises(ValueError):
            CHECK.verify(self.root, self.provenance)
        (self.root / 'src/extra.rs').unlink()
        (self.root / 'src/lib.rs').unlink()
        with self.assertRaisesRegex(ValueError, 'missing or unaccounted'):
            CHECK.verify(self.root, self.provenance)

    def test_matching_external_file_and_directory_links_are_refused(self):
        source = self.root / 'src/lib.rs'
        external = self.root.parent / 'external.rs'
        source.rename(external)
        source.symlink_to(external)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            CHECK.verify(self.root, self.provenance)
        source.unlink()
        external.rename(source)
        folder = self.root.parent / 'external-src'
        (self.root / 'src').rename(folder)
        (self.root / 'src').symlink_to(folder, target_is_directory=True)
        with self.assertRaisesRegex(ValueError, 'symlink'):
            CHECK.verify(self.root, self.provenance)
