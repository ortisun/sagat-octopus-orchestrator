"""The request line the runner builds, read by the host (feature 035: T035-035, D035-109).

``tests/contract/iceberg-writes/fixtures/requests/iceberg-write-append.json`` is the line the Rust builder writes
(``crates/infra-lake/tests/write_body.rs`` asserts that the builder's output *is* this file). Here the host reads the
same file with its own ``parse_write`` and ``open_connection``. A member spelled differently on either side fails a
test on that side, and not in a container where the only symptom would be ``LAKE_HOST_PROTOCOL``.
"""

from __future__ import annotations

import copy
import json
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
