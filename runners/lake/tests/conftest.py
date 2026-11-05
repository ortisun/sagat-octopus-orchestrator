"""Shared fixtures of the lake runtime tests."""

from __future__ import annotations

from pathlib import Path

import pytest

from lockfile import Requirement, lock_path, parse_lock

ROOT = Path(__file__).resolve().parent.parent


@pytest.fixture(scope="session")
def lake_root() -> Path:
    """`runners/lake`, wherever the tests are started from."""
    return ROOT


@pytest.fixture(scope="session")
def lock(lake_root: Path) -> list[Requirement]:
    """The parsed `requirements.lock`; the fixture fails when the lock is malformed."""
    path = lock_path(lake_root)
    return parse_lock(path.read_text(encoding="utf-8"), str(path))
