"""The request line the runner builds, read by the host (feature 035: T035-035, D035-109).

``tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json`` is the line the Rust builder writes
(``crates/infra-lake/tests/write_body.rs`` asserts that the builder's output *is* this file). Here the host reads the
same file with its own ``parse_write`` and ``open_connection``. A member spelled differently on either side fails a
test on that side, and not in a container where the only symptom would be ``LAKE_HOST_PROTOCOL``.
"""

from __future__ import annotations

import copy
import json
from collections.abc import Callable
from pathlib import Path
from typing import Any

import pytest

from sagat_lake.connection import open_connection
from sagat_lake.failures import Failure
from sagat_lake.request import SNAPSHOT_PROPERTIES, parse_write

FIXTURE = (
    Path(__file__).resolve().parents[3] / "tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json"
)
SECRETS = ("client-secret-canary", "storage-secret-canary", "AKIAEXAMPLE", "relay-token-canary")


def line() -> dict[str, Any]:
    document: dict[str, Any] = json.loads(FIXTURE.read_text(encoding="utf-8"))
    return document


def test_the_host_parses_the_line_the_runner_builds(tmp_path: Path) -> None:
    attempt = tmp_path / "attempt-1"
    request = parse_write(line(), attempt_dir=attempt)

    assert request.namespace == ("sales",)
    assert request.table == "orders"
    append = request.append
    assert append.effect_key == "run-7f3a/load/1"
    assert append.ref == "main"
    assert append.attempt_id == "attempt-1"
    assert [(item.path, item.format) for item in append.inputs] == [
        (attempt / "in" / "inline.ndjson", "ndjson"),
        (attempt / "in" / "blob-0.parquet", "parquet"),
    ]
    assert append.mapping == (("/amount", "amount"),)
    assert append.ignore_unknown_fields is True
    assert append.allow_empty is False
    assert append.prior_attempt_may_have_left_files is False
    assert append.max_commit_retries == 4
    assert append.max_decoded_bytes == 17_179_869_184
    assert append.max_metadata_bytes == 67_108_864
    assert append.warehouse_locations == ("s3://lake/warehouse/",)
    assert request.deadline_at_ms - append.first_attempt_started_at_ms == 10 * 60 * 1000


def test_the_host_opens_the_connection_the_runner_builds(tmp_path: Path) -> None:
    request = parse_write(line(), attempt_dir=tmp_path / "attempt-1")

    opened = open_connection(request.connection)

    assert opened.properties["uri"] == "https://catalog.example/api"
    assert opened.properties["warehouse"] == "s3://lake/warehouse/"
    assert opened.properties["prefix"] == "tenant-a"
    assert opened.properties["credential"] == "client-1:client-secret-canary"
    assert opened.properties["oauth2-server-uri"] == "https://auth.example/token"
    assert opened.properties["s3.region"] == "eu-west-1"
    assert opened.properties["s3.access-key-id"] == "AKIAEXAMPLE"
    assert opened.properties["s3.secret-access-key"] == "storage-secret-canary"


def test_the_line_carries_every_member_the_host_reads_and_no_other() -> None:
    document = line()
    assert set(document) == {
        "protocol",
        "operation",
        "effectKey",
        "firstAttemptStartedAt",
        "relay",
        "table",
        "ref",
        "mode",
        "overwrite",
        "upsert",
        "snapshotProperties",
        "connection",
        "inputs",
        "mapping",
        "ignoreUnknownFields",
        "allowEmpty",
        "priorAttemptMayHaveLeftFiles",
        "limits",
    }
    assert set(document["snapshotProperties"]) == SNAPSHOT_PROPERTIES


def test_the_host_does_not_show_a_secret_of_the_line(tmp_path: Path) -> None:
    request = parse_write(line(), attempt_dir=tmp_path / "attempt-1")
    shown = repr(request) + repr(open_connection(request.connection))
    for secret in SECRETS:
        assert secret not in shown


@pytest.mark.parametrize("member", ["table", "ref", "mode", "inputs", "limits", "connection", "snapshotProperties"])
def test_a_line_without_a_required_member_is_a_break(member: str, tmp_path: Path) -> None:
    document = line()
    del document[member]
    with pytest.raises(Failure) as raised:
        parse_write(document, attempt_dir=tmp_path / "attempt-1")
    assert raised.value.code == "LAKE_HOST_PROTOCOL"


def test_the_fixture_is_not_changed_by_reading_it(tmp_path: Path) -> None:
    before = copy.deepcopy(line())
    parse_write(before, attempt_dir=tmp_path / "attempt-1")
    assert before == line()


# The builder in Rust refuses what is outside these numbers (``crates/infra-lake/tests/write_body.rs`` pins the same
# ones). The constants live once on each side, so a test has to say that the host takes the last value the builder lets
# through and refuses the first it does not: if either side moves a ceiling alone, one of the two tests fails.
def put(*path: str | int) -> Callable[[dict[str, Any], Any], None]:
    def apply(document: dict[str, Any], value: Any) -> None:
        target: Any = document
        for step in path[:-1]:
            target = target[step]
        target[path[-1]] = value

    return apply


def files(count: int) -> list[dict[str, str]]:
    return [{"file": f"in/f{index}.ndjson", "format": "ndjson"} for index in range(count)]


def pairs(count: int) -> list[dict[str, str]]:
    return [{"field": f"/f{index}", "column": f"c{index}"} for index in range(count)]


def named(length: int) -> list[dict[str, str]]:
    return [{"file": "in/" + "a" * length, "format": "ndjson"}]


INT64 = 2**63 - 1
LOCATION = "s3://lake/warehouse/"
CEILINGS = [
    ("effect key, in bytes", put("effectKey"), "é" * 256, "é" * 256 + "a"),
    ("retries", put("limits", "maxCommitRetries"), 16, 17),
    ("decoded bytes", put("limits", "maxDecodedBytes"), str(INT64), str(INT64 + 1)),
    ("metadata bytes", put("limits", "maxMetadataBytes"), str(INT64), str(INT64 + 1)),
    ("ref, in bytes", put("ref"), "é" * 127 + "a", "é" * 128),
    ("inputs", put("inputs"), files(32), files(33)),
    ("file name", put("inputs"), named(128), named(129)),
    ("mapped columns", put("mapping"), pairs(1024), pairs(1025)),
    ("property value, in bytes", put("snapshotProperties", "sagat.run-id"), "é" * 512, "é" * 512 + "a"),
    ("warehouse locations", put("connection", "warehouseLocations"), [LOCATION] * 16, [LOCATION] * 17),
    # the catalog's own warehouse has to stay inside a declared location, so the long one comes in addition to it
    (
        "a location, in bytes",
        put("connection", "warehouseLocations"),
        [LOCATION, "s3://" + "é" * 1021 + "a"],
        [LOCATION, "s3://" + "é" * 1022],
    ),
    ("a secret", put("connection", "credentials", "storage", "secretAccessKey"), "x" * 16384, "x" * 16385),
]


def taken(document: dict[str, Any], tmp_path: Path) -> bool:
    try:
        request = parse_write(document, attempt_dir=tmp_path / "attempt-1")
        open_connection(request.connection)
    except Failure:
        return False
    return True


@pytest.mark.parametrize(("name", "setter", "last", "first_over"), CEILINGS, ids=[row[0] for row in CEILINGS])
def test_the_host_takes_the_last_value_the_builder_allows_and_refuses_the_next(
    name: str, setter: Callable[[dict[str, Any], Any], None], last: Any, first_over: Any, tmp_path: Path
) -> None:
    inside = line()
    setter(inside, last)
    assert taken(inside, tmp_path), f"{name}: the last allowed value was refused"

    over = line()
    setter(over, first_over)
    assert not taken(over, tmp_path), f"{name}: the first value over the ceiling was taken"


@pytest.mark.parametrize(
    "deadline",
    [
        "2026-02-30T10:10:00Z",
        "2025-02-29T10:10:00Z",
        "2100-02-29T10:10:00Z",
        "2026-04-31T10:10:00Z",
        "2026-13-01T10:10:00Z",
        "2026-00-10T10:10:00Z",
        "2026-11-05T25:00:00Z",
        "2026-11-05T10:60:00Z",
        "2026-11-05T10:10:60Z",
        "0000-01-01T00:00:00Z",
    ],
)
def test_the_host_refuses_the_instants_the_builder_refuses(deadline: str, tmp_path: Path) -> None:
    for setter in (put("limits", "deadlineAt"), put("firstAttemptStartedAt")):
        document = line()
        setter(document, deadline)
        assert not taken(document, tmp_path)


@pytest.mark.parametrize(
    "deadline",
    [
        "2024-02-29T23:59:59Z",
        "2000-02-29T00:00:00Z",
        "2026-12-31T23:59:59.999999Z",
        "9999-12-31T23:59:59Z",
        "0001-01-01T00:00:00Z",
    ],
)
def test_the_host_takes_the_instants_the_builder_takes(deadline: str, tmp_path: Path) -> None:
    for setter in (put("limits", "deadlineAt"), put("firstAttemptStartedAt")):
        document = line()
        setter(document, deadline)
        assert taken(document, tmp_path)
