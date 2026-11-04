"""Reads `requirements.lock` the way the image build does, and refuses what the build would refuse.

The lock is what `pip install --require-hashes --no-deps` installs in the runtime image (contracts/lake-runtime.md,
"Image"). A line that is not a pinned requirement with hashes is a way for something unreviewed to get in, so the
parser raises on it instead of skipping it.
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from pathlib import Path

_REQUIREMENT = re.compile(
    r"^(?P<name>[A-Za-z0-9][A-Za-z0-9._-]*)(?:\[[^\]]*\])?==(?P<version>[A-Za-z0-9._!+-]+)"
    r"\s*(?:;\s*(?P<marker>[^\\]*?))?\s*\\?$"
)
_HASH = re.compile(r"^\s+--hash=sha256:(?P<digest>[0-9a-f]{64})\s*\\?$")


@dataclass(frozen=True)
class Requirement:
    name: str
    version: str
    marker: str | None
    hashes: tuple[str, ...]


def parse_lock(text: str, source: str = "requirements.lock") -> list[Requirement]:
    """Every requirement of `text`; `ValueError` for a line that is not a comment, a pinned requirement or a hash."""
    found: list[tuple[str, str, str | None, list[str]]] = []
    for number, line in enumerate(text.splitlines(), start=1):
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        requirement = _REQUIREMENT.match(line)
        if requirement:
            marker = requirement.group("marker")
            found.append((requirement.group("name"), requirement.group("version"), marker or None, []))
            continue
        digest = _HASH.match(line)
        if digest and found:
            found[-1][3].append(digest.group("digest"))
            continue
        raise ValueError(f"{source}:{number}: not a pinned requirement with sha256 hashes: {line.strip()!r}")
    unhashed = [name for name, _, _, hashes in found if not hashes]
    if unhashed:
        raise ValueError(f"{source}: requirements without a hash: {', '.join(sorted(unhashed))}")
    return [Requirement(name, version, marker, tuple(hashes)) for name, version, marker, hashes in found]


def normalise(name: str) -> str:
    """PEP 503 name, so `psycopg2_binary` and `psycopg2-binary` are one package."""
    return re.sub(r"[-_.]+", "-", name).lower()


def lock_path(root: Path) -> Path:
    return root / "requirements.lock"
