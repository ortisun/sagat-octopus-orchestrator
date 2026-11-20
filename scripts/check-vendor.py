#!/usr/bin/env python3
"""Allow only the reviewed rskafka manifest override over its verified registry source."""
import hashlib
import json
from pathlib import Path
import sys

ARCHIVE_SHA256 = '849b87417a191e37e16b8893eba8928bbd10e1989e6b57c4f8531549eaa29bdc'
FILES_SHA256 = '63361668cea34024c6159ba78dd079c137f644523527ced7a67661be1a06097d'
FILE_COUNT = 84
OLD = b'[dependencies.zstd]\nversion = "0.13"'
NEW = b'[dependencies.zstd]\nversion = "0.14.3"'


def verify(root: Path, provenance: Path) -> int:
    metadata = json.loads(provenance.read_text())
    if (metadata['package'], metadata['version'], metadata['registryArchiveSHA256']) != ('rskafka', '0.6.0', ARCHIVE_SHA256):
        raise ValueError('unexpected registry source identity')
    files_digest = hashlib.sha256(json.dumps(metadata['files'], sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    if files_digest != FILES_SHA256 or len(metadata['files']) != FILE_COUNT:
        raise ValueError('upstream file manifest is not the reviewed archive manifest')
    if root.is_symlink() or not root.is_dir():
        raise ValueError('snapshot root must be a real directory')
    actual = set()
    for path in root.rglob('*'):
        if path.is_symlink():
            raise ValueError('snapshot contains a symlink')
        if path.is_dir():
            continue
        if not path.is_file():
            raise ValueError('snapshot contains a special file')
        name = path.relative_to(root).as_posix()
        actual.add(name)
        data = path.read_bytes()
        if name == 'Cargo.toml':
            if data.count(NEW) != 1:
                raise ValueError('missing or ambiguous reviewed zstd override')
            data = data.replace(NEW, OLD, 1)
        if hashlib.sha256(data).hexdigest() != metadata['files'].get(name):
            raise ValueError(f'upstream snapshot changed: {name}')
    if actual != set(metadata['files']) or 'Cargo.toml' not in actual:
        raise ValueError('snapshot has missing or unaccounted files')
    return len(actual)


def main() -> int:
    repo = Path(__file__).resolve().parents[1]
    try:
        count = verify(repo / 'vendor/rskafka', repo / 'vendor/rskafka.provenance.json')
    except (ValueError, KeyError, OSError) as error:
        print(f'vendor check FAILED: {error}', file=sys.stderr)
        return 1
    print(f'vendor check passed ({count} upstream files, one manifest-only override)')
    return 0


if __name__ == '__main__':
    sys.exit(main())
