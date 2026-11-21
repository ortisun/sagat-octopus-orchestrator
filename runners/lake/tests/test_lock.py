"""The runtime lock: what the image installs, and the rules that keep it reviewable (G035-023, D035-026)."""

from __future__ import annotations

from pathlib import Path

import pytest

from lockfile import Requirement, normalise, parse_lock

DECIDED_REQUIREMENT = "pyiceberg[pyarrow,pyiceberg-core,s3fs,gcsfs,adlfs,hive,glue,sql-postgres]==0.12.0"
GOOD_HASH = "sha256:" + "ab" * 32


def test_the_lock_pins_and_hashes_every_requirement(lock: list[Requirement]) -> None:
    assert len(lock) > 50, "the closure of PyIceberg with its storage and catalog extras is dozens of packages"
    assert all(requirement.hashes for requirement in lock)


def test_the_lock_pins_pyiceberg_to_the_decided_version(lock: list[Requirement]) -> None:
    versions = {requirement.version for requirement in lock if normalise(requirement.name) == "pyiceberg"}
    assert versions == {"0.12.0"}


@pytest.mark.parametrize(
    "package",
    [
        "pyarrow",  # extra pyarrow
        "pyiceberg-core",  # extra pyiceberg-core: the partition transforms but identity and void (G035-040)
        "s3fs",  # extra s3fs
        "gcsfs",  # extra gcsfs
        "adlfs",  # extra adlfs
        "thrift",  # extra hive
        "boto3",  # extra glue
        "psycopg2-binary",  # extra sql-postgres
        "sqlalchemy",  # extra sql-postgres
    ],
)
def test_the_lock_holds_the_closure_of_every_extra(lock: list[Requirement], package: str) -> None:
    assert package in {normalise(requirement.name) for requirement in lock}


def test_a_package_is_pinned_once_for_each_environment_marker(lock: list[Requirement]) -> None:
    seen: set[tuple[str, str | None]] = set()
    for requirement in lock:
        key = (normalise(requirement.name), requirement.marker)
        assert key not in seen, f"{requirement.name} is pinned twice for the same marker"
        seen.add(key)


def test_requirements_in_is_the_one_decided_requirement(lake_root: Path) -> None:
    lines = [
        line.strip()
        for line in (lake_root / "requirements.in").read_text(encoding="utf-8").splitlines()
        if line.strip() and not line.strip().startswith("#")
    ]
    assert lines == [DECIDED_REQUIREMENT]


# The parser is the policy: these prove it refuses what the build must never install.


def test_the_parser_refuses_a_requirement_without_a_hash() -> None:
    with pytest.raises(ValueError, match="without a hash: requests"):
        parse_lock("requests==2.34.2\n")


def test_the_parser_refuses_a_range() -> None:
    with pytest.raises(ValueError, match="not a pinned requirement"):
        parse_lock(f"requests>=2.0 \\\n    --hash={GOOD_HASH}\n")


@pytest.mark.parametrize(
    "line",
    [
        "--extra-index-url https://example.invalid/simple",
        "--index-url https://example.invalid/simple",
        "--find-links https://example.invalid/wheels",
        "--trusted-host example.invalid",
        "git+https://example.invalid/project.git#egg=project",
        "project @ https://example.invalid/project-1.0.tar.gz",
        "-e .",
    ],
)
def test_the_parser_refuses_a_source_other_than_the_pinned_index(line: str) -> None:
    with pytest.raises(ValueError, match="not a pinned requirement"):
        parse_lock(f"requests==2.34.2 \\\n    --hash={GOOD_HASH}\n{line}\n")


def test_the_parser_refuses_a_hash_that_is_not_sha256_of_full_length() -> None:
    with pytest.raises(ValueError, match="not a pinned requirement"):
        parse_lock("requests==2.34.2 \\\n    --hash=sha256:abc123\n")


def test_the_parser_reads_markers_and_several_hashes() -> None:
    text = (
        "# a comment\n"
        f'colorama==0.4.6 ; sys_platform == "win32" \\\n    --hash={GOOD_HASH} \\\n    --hash=sha256:{"cd" * 32}\n'
        "    # via click\n"
    )
    [requirement] = parse_lock(text)
    assert (requirement.name, requirement.version) == ("colorama", "0.4.6")
    assert requirement.marker == 'sys_platform == "win32"'
    assert len(requirement.hashes) == 2
