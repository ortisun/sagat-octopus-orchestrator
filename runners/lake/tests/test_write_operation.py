"""The `icebergWrite` operation of the host, through the protocol (feature 035: T035-057, G035-047, D035-106).

The runner writes one request line and reads progress lines and one result line, so that is what these tests do: a relay
double serves the control conversation, the host reads the line, parses it, opens the catalog of the connection, appends
the records of `in/` and answers with a receipt or with a failure of the closed set. The catalog is a real REST catalog
over HTTP (`counting_catalog.py`) with a real SQL catalog behind it, which can lie, refuse, go down or be slow.

Two kinds of test. Those of the success path hand the operation a catalog opener that reaches the stub with plain files
as its warehouse, because the real connection speaks S3 and a write would leave the machine. Those of the failures that
come before any write use the real opener, so the request, the connection, the hygiene and the catalog are the ones the
host runs.
"""

from __future__ import annotations

import asyncio
import json
import pathlib
import tempfile
from collections.abc import Callable, Generator
from typing import Any

import pytest
from pyiceberg.catalog import Catalog as PyIcebergCatalog
from pyiceberg.catalog.rest import RestCatalog
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.schema import Schema
from pyiceberg.types import LongType, NestedField
from test_host_protocol import Conversation

import counting_catalog
from counting_catalog import Catalog
from relay_double import FakeRelay
from sagat_lake import host
from sagat_lake.failures import Failure
from sagat_lake.write import iceberg_write, operations_for

TOKEN = "relay-token-canary-0123456789abcdef"
CANARY = "credential-canary-s3-secret-access-key"
KEY = "run-1:save-orders"
WAREHOUSE = "s3://lake/warehouse/"


@pytest.fixture(autouse=True)
def loopback(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1")


@pytest.fixture(autouse=True)
def no_real_hardening(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(host, "harden", lambda: True)


@pytest.fixture
def stub(tmp_path: pathlib.Path) -> Generator[Catalog]:
    sql = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    sql.create_namespace("sales")
    sql.create_table("sales.orders", schema=Schema(NestedField(1, "id", LongType(), required=False)))
    with counting_catalog.serve(sql) as catalog:
        catalog.properties = {"commit.retry.min-wait-ms": "1", "commit.retry.max-wait-ms": "2"}
        yield catalog


def warehouse_of(stub: Catalog) -> str:
    return stub.sql.load_table("sales.orders").metadata.location.rsplit("/sales/", 1)[0] + "/"


def connection(stub: Catalog, *, file_warehouse: bool, uri: str | None = None) -> dict[str, Any]:
    return {
        "catalog": {
            "type": "rest",
            "rest": {"uri": uri or stub.uri, "warehouse": WAREHOUSE, "accessDelegation": "none"},
        },
        "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": False}, "credentials": "static"},
        "warehouseLocations": [warehouse_of(stub) if file_warehouse else WAREHOUSE],
        "credentials": {"storage": {"accessKeyId": "AKIA", "secretAccessKey": CANARY}},
    }


def line(stub: Catalog, *, file_warehouse: bool = False, uri: str | None = None, **changes: Any) -> bytes:
    document: dict[str, Any] = {
        "protocol": "sagat.lake.v1",
        "operation": "icebergWrite",
        "effectKey": KEY,
        "firstAttemptStartedAt": "2026-10-27T18:02:11.204Z",
        "snapshotProperties": {"sagat.effect-key": KEY, "sagat.run-id": "run-1"},
        "connection": connection(stub, file_warehouse=file_warehouse, uri=uri),
        "relay": {"socket": "/work/attempt-1/relay.sock", "token": TOKEN},
        "table": {"namespace": ["sales"], "name": "orders"},
        "ref": "main",
        "mode": "append",
        "overwrite": None,
        "upsert": None,
        "inputs": [{"file": "in/inline.ndjson", "format": "ndjson"}],
        "limits": {
            "maxCommitRetries": 4,
            "maxDecodedBytes": "17179869184",
            "maxMetadataBytes": "67108864",
            "deadlineAt": "2026-10-27T18:12:11Z",
        },
    }
    return json.dumps({**document, **changes}, separators=(",", ":")).encode()


def plain_opener(connection_document: Any, request_timeout: float) -> PyIcebergCatalog:
    """The stub as a plain REST catalog: the success path writes to files, never to S3."""
    del request_timeout
    return RestCatalog("plain", uri=connection_document["catalog"]["rest"]["uri"])


def converse(
    request_line: bytes,
    *,
    records: bytes = b'{"id": 1}\n{"id": 2}\n',
    opener: Callable[[Any, float], PyIcebergCatalog] | None = None,
    interval: float = 1.0,
    request_timeout: float = 5.0,
) -> Conversation:
    conversation = Conversation(request_line)
    relay = FakeRelay(TOKEN, control=conversation)

    async def main() -> None:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            attempt = pathlib.Path(directory) / "attempt-1"
            (attempt / "in").mkdir(parents=True)
            (attempt / "in" / "inline.ndjson").write_bytes(records)
            await relay.start(attempt / "relay.sock")
            (attempt / "relay.token").write_text(TOKEN + "\n")
            try:
                operations = (
                    {"icebergWrite": iceberg_write(attempt, request_timeout=request_timeout, open_catalog=opener)}
                    if opener is not None
                    else {"icebergWrite": iceberg_write(attempt, request_timeout=request_timeout)}
                )
                assert await asyncio.wait_for(host.run(attempt, operations, heartbeat_interval=interval), 30) == 0
            finally:
                await relay.stop()

    asyncio.run(main())
    return conversation


def result(conversation: Conversation) -> dict[str, Any]:
    (only,) = conversation.results
    assert only.pop("relayDenials") == 0
    return only


def failure_code(conversation: Conversation) -> str:
    outcome = result(conversation)
    assert list(outcome) == ["failure"], outcome
    return str(outcome["failure"]["code"])


def rows(stub: Catalog) -> list[int]:
    return sorted(row["id"] for row in stub.sql.load_table("sales.orders").scan().to_arrow().to_pylist())


# ------------------------------------------------------------------------------------------------------ the success
def test_the_records_of_the_attempt_directory_are_appended_and_the_result_is_the_receipt(stub: Catalog) -> None:
    conversation = converse(line(stub, file_warehouse=True), opener=plain_opener)

    outcome = result(conversation)

    assert list(outcome) == ["receipt"]
    receipt = outcome["receipt"]
    assert (receipt["kind"], receipt["outcome"], receipt["mode"], receipt["ref"]) == (
        "iceberg",
        "COMMITTED",
        "append",
        "main",
    )
    assert (receipt["foundByLookup"], receipt["rowsAdded"], receipt["dataFilesAdded"]) == (False, "2", "1")
    assert receipt["effectKey"] == KEY
    assert rows(stub) == [1, 2]
    assert len(stub.commits) == 1


def test_the_same_request_again_finds_its_commit_and_writes_nothing(stub: Catalog) -> None:
    first = result(converse(line(stub, file_warehouse=True), opener=plain_opener))["receipt"]
    commits = len(stub.commits)

    again = result(converse(line(stub, file_warehouse=True), opener=plain_opener))["receipt"]

    assert again["foundByLookup"] is True
    assert again["snapshotId"] == first["snapshotId"]
    assert len(stub.commits) == commits
    assert rows(stub) == [1, 2]


def test_the_host_answers_while_the_catalog_is_slow_and_says_it_is_alive(stub: Catalog) -> None:
    # PyIceberg blocks: if the operation ran on the event loop, no heartbeat could be written until it ended
    stub.delay = 0.4
    conversation = converse(line(stub, file_warehouse=True), opener=plain_opener, interval=0.1)

    assert len(conversation.heartbeats) >= 3
    assert list(result(conversation)) == ["receipt"]
    # and the heartbeats come before the result
    assert conversation.lines[-1] != b'{"heartbeat":true}\n'


def test_the_mapping_and_the_flags_of_the_request_reach_the_append(stub: Catalog) -> None:
    conversation = converse(
        line(stub, file_warehouse=True, mapping=[{"field": "/n", "column": "id"}], allowEmpty=True),
        records=b'{"n": 7, "extra": 1}\n',
        opener=plain_opener,
    )
    assert result(conversation)["receipt"]["rowsAdded"] == "1"
    assert rows(stub) == [7]


def test_a_record_that_does_not_map_is_a_schema_mismatch_with_its_column_and_nothing_is_written(stub: Catalog) -> None:
    conversation = converse(line(stub, file_warehouse=True), records=b'{"id": "x"}\n', opener=plain_opener)
    assert result(conversation) == {"failure": {"code": "ICEBERG_SCHEMA_MISMATCH", "column": "id", "reason": "TYPE"}}
    assert stub.commits == []
    assert rows(stub) == []


# --------------------------------------------------------------------------------------- failures before any write
def test_a_request_the_host_cannot_serve_is_a_protocol_failure(stub: Catalog) -> None:
    assert failure_code(converse(line(stub, mode="overwrite"))) == "LAKE_HOST_PROTOCOL"
    assert failure_code(converse(line(stub, extra="x"))) == "LAKE_HOST_PROTOCOL"
    assert stub.headers == []


def test_a_table_the_catalog_does_not_have_is_a_table_not_found(stub: Catalog) -> None:
    conversation = converse(line(stub, table={"namespace": ["sales"], "name": "missing"}))
    assert failure_code(conversation) == "ICEBERG_TABLE_NOT_FOUND"


def test_a_table_the_catalog_places_outside_the_warehouse_is_a_location_failure(stub: Catalog) -> None:
    stub.rewrite = (warehouse_of(stub).rstrip("/"), "s3://attacker-bucket/stolen")
    assert failure_code(converse(line(stub))) == "ICEBERG_LOCATION_NOT_ALLOWED"
    assert stub.commits == []


@pytest.mark.parametrize(
    ("status", "code"),
    [
        (401, "DATABASE_AUTHENTICATION_FAILED"),
        (403, "DATABASE_AUTHENTICATION_FAILED"),
        (500, "CATALOG_UNAVAILABLE"),
        (503, "CATALOG_UNAVAILABLE"),
        (429, "CATALOG_UNAVAILABLE"),
    ],
)
def test_a_catalog_that_refuses_or_is_down_is_told_apart(stub: Catalog, status: int, code: str) -> None:
    stub.get_status = status
    assert failure_code(converse(line(stub))) == code


def test_a_catalog_that_cannot_be_reached_is_unavailable(stub: Catalog) -> None:
    # nothing listens on the discard port of the loopback
    assert failure_code(converse(line(stub, uri="http://127.0.0.1:9"))) == "CATALOG_UNAVAILABLE"


def test_a_catalog_that_does_not_answer_in_time_is_unavailable_and_not_waited_for(stub: Catalog) -> None:
    stub.delay = 3.0
    conversation = converse(line(stub), request_timeout=0.3)
    assert failure_code(conversation) == "CATALOG_UNAVAILABLE"


def test_a_static_credential_that_is_missing_fails_closed(stub: Catalog) -> None:
    document = json.loads(line(stub))
    document["connection"]["credentials"] = {}
    conversation = converse(json.dumps(document).encode())
    assert failure_code(conversation) == "DATABASE_CREDENTIAL_UNAVAILABLE"
    assert stub.headers == []


# -------------------------------------------------------------------------------------------------- what never leaks
def test_no_credential_reaches_the_result_or_the_standard_streams(
    stub: Catalog, capfd: pytest.CaptureFixture[str]
) -> None:
    ok = converse(line(stub, file_warehouse=True), opener=plain_opener)
    bad = converse(line(stub, file_warehouse=True), records=b'{"id": "x"}\n', opener=plain_opener)
    refused = converse(line(stub, mode="overwrite"))
    for conversation in (ok, bad, refused):
        assert CANARY.encode() not in b"".join(conversation.lines)
    captured = capfd.readouterr()
    assert CANARY not in captured.out + captured.err


def test_a_failure_that_no_one_expected_is_a_worker_error_with_only_its_class_on_stderr(
    stub: Catalog, capfd: pytest.CaptureFixture[str]
) -> None:
    def explode(connection_document: Any, request_timeout: float) -> PyIcebergCatalog:
        raise RuntimeError(f"secret {CANARY}")

    conversation = converse(line(stub, file_warehouse=True), opener=explode)

    assert result(conversation) == {"failure": {"code": "WORKER_ERROR"}}
    captured = capfd.readouterr()
    assert CANARY not in captured.out + captured.err
    assert captured.err.strip() == "RuntimeError"


def test_a_failure_of_the_closed_set_that_the_opener_raises_is_passed_on(stub: Catalog) -> None:
    def refuses(connection_document: Any, request_timeout: float) -> PyIcebergCatalog:
        raise Failure("DATABASE_CREDENTIAL_UNAVAILABLE")

    conversation = converse(line(stub, file_warehouse=True), opener=refuses)
    assert failure_code(conversation) == "DATABASE_CREDENTIAL_UNAVAILABLE"


# ------------------------------------------------------------------------------------------------------- the entry
def test_the_host_serves_the_write_and_the_probe_and_no_other_yet() -> None:
    operations = operations_for(pathlib.Path("/tmp/nowhere"))
    assert set(operations) == {"icebergWrite", "probe"}


def test_the_entry_point_builds_its_operations_for_the_attempt_directory(monkeypatch: pytest.MonkeyPatch) -> None:
    seen: dict[str, Any] = {}

    async def fake_run(attempt_dir: pathlib.Path, operations: Any, **_: Any) -> int:
        seen["attempt_dir"] = attempt_dir
        seen["operations"] = sorted(operations)
        return 0

    monkeypatch.setattr(host, "run", fake_run)
    assert host.main(["--attempt-dir", "/tmp/somewhere"]) == 0
    assert seen == {"attempt_dir": pathlib.Path("/tmp/somewhere"), "operations": ["icebergWrite", "probe"]}
