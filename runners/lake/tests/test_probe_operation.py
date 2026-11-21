"""The `probe` operation of the host, through the protocol (feature 035: D035-150, T035-040, T035-045, slice S5).

The runner writes one request line and reads progress lines and one result line, so that is what these tests do: a relay
double serves the control conversation, the host reads the line, parses it, runs the three checks in order and answers
with a typed result. The catalog is a real REST catalog over HTTP (`counting_catalog.py`); the storage listing is a
double that records what it was asked and can fail the way the real one does (the storage's own mapping is held against
the real S3 client in `test_probe_storage.py`).

**A check is the answer.** A connection that does not work is a result (`FAILED` with a code of the closed set, the
checks after it `SKIPPED`), written as the `receipt` of the line, and the attempt succeeds. Only what the connection
cannot tell, a fault of the platform or of the request, is a `failure`. Every result a scenario produces is held to the
rules of the server's `ProbeResult::new`, and each one the corpus of the server accepts is reproduced exactly.
"""

from __future__ import annotations

import asyncio
import copy
import json
import re
import tempfile
import threading
import time
from collections.abc import Generator, Mapping
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

import pytest
import requests
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.schema import Schema
from pyiceberg.types import LongType, NestedField
from test_host_protocol import Conversation

import counting_catalog
import network_doubles
from counting_catalog import Catalog
from relay_double import FakeRelay
from sagat_lake import host
from sagat_lake.failures import Failure
from sagat_lake.probe import probe_operation
from sagat_lake.probe_codes import CHECK_CODES
from sagat_lake.request import PROBE_CHECKS
from sagat_lake.rest_catalog import LakeRestCatalog
from sagat_lake.write import operations_for

ROOT = Path(__file__).resolve().parents[3]
CORPUS = ROOT / "tests/contract/iceberg-writes/fixtures"
TOKEN = "relay-token-canary-0123456789abcdef"
STORAGE_CANARY = "credential-canary-s3-secret-access-key"
CATALOG_CANARY = "credential-canary-catalog-bearer-token"
WAREHOUSE = "s3://lake/warehouse/"

AUTHENTICATION = "DATABASE_AUTHENTICATION_FAILED"
TLS = "DATABASE_TLS_FAILED"
DENIED = "DATABASE_DESTINATION_DENIED"
TIMEOUT = "DATABASE_TIMEOUT"
CATALOG_UNAVAILABLE = "CATALOG_UNAVAILABLE"
STORAGE_UNAVAILABLE = "STORAGE_UNAVAILABLE"
LOCATION = "ICEBERG_LOCATION_NOT_ALLOWED"

# What the real S3 client says (read, never written): the messages of PyArrow's S3 file system, as
# `test_probe_storage.py` provokes them.
S3_DENIED = "When getting information for key 'w' in bucket 'b': AWS Error ACCESS_DENIED during HeadObject operation"
S3_REFUSED = "AWS Error NETWORK_CONNECTION during HeadObject operation: curlCode: 7, Could not connect to server"


@pytest.fixture(autouse=True)
def loopback(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1")


@pytest.fixture(autouse=True)
def no_real_hardening(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(host, "harden", lambda: True)


@pytest.fixture
def stub(tmp_path: Path) -> Generator[Catalog]:
    sql = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    sql.create_namespace("sales")
    sql.create_table("sales.orders", schema=Schema(NestedField(1, "id", LongType(), required=False)))
    with counting_catalog.serve(sql) as catalog:
        yield catalog


@dataclass
class Storage:
    """The storage listing: what it was asked (the location and the time it has), and how it answers."""

    error: BaseException | None = None
    delay: float = 0.0
    # A call that is let go only when the test says so: a listing that is still running when the check runs out.
    hold: threading.Event | None = None
    calls: list[tuple[str, float]] = field(default_factory=list)
    properties: list[Mapping[str, str]] = field(default_factory=list)

    def __call__(self, properties: Mapping[str, str], location: str, seconds: float) -> None:
        self.calls.append((location, seconds))
        self.properties.append(dict(properties))
        if self.hold is not None:
            self.hold.wait(timeout=30)
        time.sleep(self.delay)
        if self.error is not None:
            raise self.error


def connection(
    stub: Catalog | None = None,
    *,
    vended: bool = False,
    uri: str | None = None,
    locations: list[str] | None = None,
    token_uri: str | None = None,
) -> dict[str, Any]:
    """A connection with a bearer token, or with OAuth2 when there is a token endpoint to ask."""
    rest: dict[str, Any] = {
        "uri": uri or (stub.uri if stub else "https://catalog.example.test"),
        "warehouse": WAREHOUSE,
        "accessDelegation": "vended-credentials" if vended else "none",
        "auth": {"type": "oauth2", "tokenUri": token_uri} if token_uri else {"type": "bearer"},
    }
    catalog_secret = (
        {"clientId": "client-1", "clientSecret": CATALOG_CANARY} if token_uri else {"token": CATALOG_CANARY}
    )
    return {
        "catalog": {"type": "rest", "rest": rest},
        "storage": {
            "type": "s3",
            "s3": {"region": "eu-west-1", "pathStyle": False},
            "credentials": "vended" if vended else "static",
        },
        "warehouseLocations": locations or [WAREHOUSE],
        "credentials": {
            "catalog": catalog_secret,
            **({} if vended else {"storage": {"accessKeyId": "AKIA", "secretAccessKey": STORAGE_CANARY}}),
        },
    }


def line(stub: Catalog | None = None, *, check_timeout_ms: int = 7000, **connection_options: Any) -> bytes:
    """The request of the leased probe of the corpus, with the connection a runner would have resolved."""
    lease = json.loads((CORPUS / "probe-input/probe.json").read_text(encoding="utf-8"))
    document = {
        "protocol": "sagat.lake.v1",
        "operation": "probe",
        "connection": connection(stub, **connection_options),
        "probe": {**{key: lease[key] for key in ("connection", "checks")}, "checkTimeoutMs": check_timeout_ms},
    }
    return json.dumps(document, separators=(",", ":")).encode()


def converse(request_line: bytes, *, storage: Storage | None = None, interval: float = 1.0) -> Conversation:
    conversation = Conversation(request_line)
    relay = FakeRelay(TOKEN, control=conversation)
    lister = storage if storage is not None else Storage()

    async def main() -> None:
        with tempfile.TemporaryDirectory(dir="/tmp", prefix="sl-") as directory:
            attempt = Path(directory) / "attempt-1"
            attempt.mkdir()
            await relay.start(attempt / "relay.sock")
            (attempt / "relay.token").write_text(TOKEN + "\n")
            try:
                operations = {"probe": probe_operation(list_storage=lister)}
                assert await asyncio.wait_for(host.run(attempt, operations, heartbeat_interval=interval), 30) == 0
            finally:
                await relay.stop()

    asyncio.run(main())
    return conversation


def outcome(conversation: Conversation) -> dict[str, Any]:
    (only,) = conversation.results
    assert only.pop("relayDenials") == 0
    return only


def failure_code(conversation: Conversation) -> str:
    answer = outcome(conversation)
    assert list(answer) == ["failure"], answer
    return str(answer["failure"]["code"])


def well_formed(result: Mapping[str, Any], *, vended: bool = False) -> None:
    """The rules of the server's `ProbeResult::new`, so that no scenario can pass with a result the server refuses."""
    assert list(result) == ["kind", "connection", "checks"]
    assert result["kind"] == "probe"
    assert result["connection"] == {"name": "lake-main", "revision": 3}
    checks = result["checks"]
    assert [check["check"] for check in checks] == list(PROBE_CHECKS)
    failed = False
    for check in checks:
        name, state = check["check"], check["outcome"]
        assert set(check) <= {"check", "outcome", "code"}
        if state == "FAILED":
            assert not failed, "a check after a failed one did not run"
            assert check["code"] in CHECK_CODES[name], check
            failed = True
            continue
        assert state in ("PASSED", "SKIPPED"), check
        assert "code" not in check, check
        if failed:
            assert state == "SKIPPED", "a check that ran after one that failed"
        elif state == "SKIPPED":
            # a skip has a cause: an earlier failure, or (the storage check alone) credentials the catalog vends
            assert name == "STORAGE_ACCESS", check
            assert vended, "a skip with no cause"


def result_of(conversation: Conversation, *, vended: bool = False) -> dict[str, Any]:
    """The result of a probe that ran: the receipt of the line, held to the server's rules."""
    answer = outcome(conversation)
    assert list(answer) == ["receipt"], answer
    result: dict[str, Any] = answer["receipt"]
    well_formed(result, vended=vended)
    return result


def fixture(name: str) -> dict[str, Any]:
    document: dict[str, Any] = json.loads((CORPUS / f"probe-results/accepted/{name}.json").read_text(encoding="utf-8"))
    return document


def codes(result: Mapping[str, Any]) -> list[tuple[str, str, str | None]]:
    return [(check["check"], check["outcome"], check.get("code")) for check in result["checks"]]


SKIPPED_AFTER_CATALOG = [("WAREHOUSE", "SKIPPED", None), ("STORAGE_ACCESS", "SKIPPED", None)]


# ------------------------------------------------------------------------------------------------------ the success
def development_line(stub: Catalog, *, development: bool | None) -> bytes:
    """The request of the corpus with a store over plain http on the loopback, and the runner's flag as given."""
    document = json.loads(line(stub))
    document["connection"]["storage"]["s3"] = {
        "region": "eu-west-1",
        "pathStyle": True,
        "endpoint": "http://127.0.0.1:9",
    }
    if development is not None:
        document["development"] = development
    return json.dumps(document, separators=(",", ":")).encode()


def test_a_development_runner_gets_its_store_over_http_on_the_loopback_and_any_other_is_a_protocol_failure(
    stub: Catalog,
) -> None:
    storage = Storage()

    result = result_of(converse(development_line(stub, development=True), storage=storage))

    # the checks ran: the flag reached the connection, and the storage client was given the endpoint
    assert [check["check"] for check in result["checks"]] == ["CATALOG_AUTH", "WAREHOUSE", "STORAGE_ACCESS"]
    assert storage.properties[0]["s3.endpoint"] == "http://127.0.0.1:9"
    for development in (False, None):
        assert failure_code(converse(development_line(stub, development=development))) == "LAKE_HOST_PROTOCOL"


def test_a_connection_that_works_passes_every_check(stub: Catalog) -> None:
    storage = Storage()

    conversation = converse(line(stub), storage=storage)

    assert result_of(conversation) == fixture("all-passed")
    # authenticated with the bearer token of the connection, asked for one page of namespaces (and for no more), and
    # listed the one declared location with the time one check has
    assert stub.headers[0]["Authorization"] == f"Bearer {CATALOG_CANARY}"
    assert len(stub.namespace_requests) == 1
    assert "pageSize=1" in stub.namespace_requests[0]
    assert storage.calls == [(WAREHOUSE, 7.0)]
    assert storage.properties[0]["s3.access-key-id"] == "AKIA"
    assert storage.properties[0]["s3.secret-access-key"] == STORAGE_CANARY
    # and the storage client is given the storage's properties and the file IO the host pins, never the catalog's
    # (its address, its token): a client that has no use for a secret is not handed it
    assert set(storage.properties[0]) == {
        "s3.region",
        "s3.force-virtual-addressing",
        "s3.access-key-id",
        "s3.secret-access-key",
        "py-io-impl",
    }
    assert CATALOG_CANARY not in storage.properties[0].values()
    assert stub.commits == []


def test_the_storage_is_listed_under_every_declared_location_and_not_after_the_first_that_fails(stub: Catalog) -> None:
    locations = ["s3://lake/warehouse/", "s3://lake/other/", "s3://elsewhere/"]
    storage = Storage()
    assert result_of(converse(line(stub, locations=locations), storage=storage)) == fixture("all-passed")
    assert [call[0] for call in storage.calls] == locations

    failing = Storage(error=OSError(S3_DENIED))
    result = result_of(converse(line(stub, locations=locations), storage=failing))
    assert codes(result)[2] == ("STORAGE_ACCESS", "FAILED", AUTHENTICATION)
    assert [call[0] for call in failing.calls] == locations[:1]


def test_a_catalog_that_pages_is_asked_for_the_first_page_only(stub: Catalog) -> None:
    stub.next_page_token = "more"
    assert result_of(converse(line(stub))) == fixture("all-passed")
    assert len(stub.namespace_requests) == 1


def test_a_catalog_that_does_not_list_namespaces_is_not_asked_to(stub: Catalog) -> None:
    # it says what it has, and listing namespaces is not in it: the configuration it gave is what the check had
    stub.config_endpoints = ["GET /v1/{prefix}/namespaces/{namespace}/tables/{table}"]
    assert result_of(converse(line(stub))) == fixture("all-passed")
    assert stub.namespace_requests == []


def test_the_host_answers_while_the_catalog_is_slow_and_says_it_is_alive(stub: Catalog) -> None:
    # PyIceberg blocks: if the checks ran on the event loop, no heartbeat could be written until they ended
    stub.delay = 0.3
    conversation = converse(line(stub), interval=0.1)
    assert len(conversation.heartbeats) >= 3
    assert result_of(conversation) == fixture("all-passed")
    assert conversation.lines[-1] != b'{"heartbeat":true}\n'


# ------------------------------------------------------------------------------------- CATALOG_AUTH: the catalog
@pytest.mark.parametrize("status", [401, 403])
def test_a_catalog_that_refuses_the_credentials_is_an_authentication_failure(stub: Catalog, status: int) -> None:
    stub.get_status = status
    storage = Storage()

    result = result_of(converse(line(stub), storage=storage))

    assert result == fixture("catalog-auth-failed")
    assert storage.calls == []
    assert stub.namespace_requests == []


@pytest.mark.parametrize("status", [404, 429, 500, 503])
def test_a_catalog_that_is_down_or_does_not_know_the_warehouse_is_unavailable(stub: Catalog, status: int) -> None:
    stub.get_status = status
    result = result_of(converse(line(stub)))
    assert codes(result) == [("CATALOG_AUTH", "FAILED", CATALOG_UNAVAILABLE), *SKIPPED_AFTER_CATALOG]


def test_a_catalog_that_cannot_be_reached_is_unavailable() -> None:
    # nothing listens on the discard port of the loopback
    result = result_of(converse(line(uri="http://127.0.0.1:9")))
    assert codes(result) == [("CATALOG_AUTH", "FAILED", CATALOG_UNAVAILABLE), *SKIPPED_AFTER_CATALOG]


def test_a_catalog_whose_configuration_is_not_one_is_unavailable(stub: Catalog) -> None:
    stub.config_endpoints = ["not an endpoint"]
    result = result_of(converse(line(stub)))
    assert codes(result)[0] == ("CATALOG_AUTH", "FAILED", CATALOG_UNAVAILABLE)


def test_a_catalog_that_sends_more_than_any_catalog_has_to_say_is_unavailable(stub: Catalog) -> None:
    stub.config_defaults = {"warehouse": WAREHOUSE, "filler": "x" * (2 * 1024 * 1024)}
    result = result_of(converse(line(stub)))
    assert codes(result) == [("CATALOG_AUTH", "FAILED", CATALOG_UNAVAILABLE), *SKIPPED_AFTER_CATALOG]


@pytest.mark.filterwarnings("ignore::DeprecationWarning")
def test_a_token_endpoint_that_never_answers_is_a_timeout_of_the_first_check_and_does_not_hold_the_host(
    stub: Catalog,
) -> None:
    # OAuth2: the client asks the token endpoint (https, always) for a token before it asks the catalog for anything.
    # Here it takes the connection and says nothing, which for a TLS client is a handshake that never ends.
    with network_doubles.silent_server(hold=4.0) as port:
        started = time.monotonic()
        result = result_of(converse(line(stub, check_timeout_ms=300, token_uri=f"https://127.0.0.1:{port}")))
        elapsed = time.monotonic() - started

    assert codes(result) == [("CATALOG_AUTH", "FAILED", TIMEOUT), *SKIPPED_AFTER_CATALOG]
    assert elapsed < 2.0


def test_a_catalog_that_does_not_answer_in_time_is_a_timeout(stub: Catalog) -> None:
    stub.delay = 3.0
    started = time.monotonic()

    result = result_of(converse(line(stub, check_timeout_ms=300)))

    assert codes(result) == [("CATALOG_AUTH", "FAILED", TIMEOUT), *SKIPPED_AFTER_CATALOG]
    assert time.monotonic() - started < 2.5


def test_a_catalog_with_an_untrusted_certificate_is_a_tls_failure(tmp_path: Path) -> None:
    storage = Storage()
    with network_doubles.untrusted_tls_server(tmp_path) as port:
        result = result_of(converse(line(uri=f"https://127.0.0.1:{port}"), storage=storage))

    assert result == fixture("catalog-tls-failed")
    assert storage.calls == []


def test_a_catalog_the_relay_refuses_is_a_destination_denial(monkeypatch: pytest.MonkeyPatch) -> None:
    storage = Storage()
    with network_doubles.refusing_proxy() as port:
        monkeypatch.setenv("HTTPS_PROXY", f"http://127.0.0.1:{port}")
        monkeypatch.delenv("NO_PROXY")
        result = result_of(converse(line(), storage=storage))

    assert result == fixture("catalog-host-refused")
    assert storage.calls == []


# --------------------------------------------------------------------------------------------- WAREHOUSE: the warehouse
def test_a_warehouse_the_catalog_places_outside_the_declared_locations_is_a_location_failure(stub: Catalog) -> None:
    stub.config_overrides = {"warehouse": "s3://attacker-bucket/stolen"}
    storage = Storage()

    result = result_of(converse(line(stub), storage=storage))

    # the catalog answered and let the client in: what it said is what failed, and nothing is listed under it
    assert result == fixture("warehouse-location-not-allowed")
    assert storage.calls == []
    assert stub.namespace_requests == []


@pytest.mark.parametrize(
    ("status", "code"),
    [(401, AUTHENTICATION), (403, AUTHENTICATION), (404, CATALOG_UNAVAILABLE), (500, CATALOG_UNAVAILABLE)],
)
def test_a_warehouse_that_cannot_be_listed_fails_the_warehouse_check_and_not_the_one_before(
    stub: Catalog, status: int, code: str
) -> None:
    stub.namespaces_status = status
    storage = Storage()

    result = result_of(converse(line(stub), storage=storage))

    assert codes(result) == [
        ("CATALOG_AUTH", "PASSED", None),
        ("WAREHOUSE", "FAILED", code),
        ("STORAGE_ACCESS", "SKIPPED", None),
    ]
    assert storage.calls == []


def test_a_catalog_that_answers_the_listing_with_something_else_is_unavailable(stub: Catalog) -> None:
    stub.namespaces_body = {"unexpected": True}
    result = result_of(converse(line(stub)))
    assert codes(result)[1] == ("WAREHOUSE", "FAILED", CATALOG_UNAVAILABLE)


def test_a_catalog_that_answers_the_listing_with_more_than_any_catalog_sends_is_unavailable(stub: Catalog) -> None:
    stub.namespaces_raw = ({}, json.dumps({"namespaces": [["n" * 100]] * 20_000}).encode())
    result = result_of(converse(line(stub)))
    assert codes(result)[1] == ("WAREHOUSE", "FAILED", CATALOG_UNAVAILABLE)


@pytest.mark.parametrize(
    ("raised", "code"),
    [
        (requests.exceptions.ReadTimeout("slow"), TIMEOUT),
        (requests.exceptions.ConnectTimeout("slow"), TIMEOUT),
        (requests.exceptions.ConnectionError("down"), CATALOG_UNAVAILABLE),
        # the matrix of the server: the warehouse check is never a TLS failure or a denial of the destination, which
        # are what the check before it says about the same host
        (requests.exceptions.SSLError("certificate"), CATALOG_UNAVAILABLE),
        (
            requests.exceptions.ProxyError("Unable to connect to proxy: Tunnel connection failed: 403"),
            CATALOG_UNAVAILABLE,
        ),
    ],
)
def test_what_the_warehouse_check_says_is_always_a_code_it_may_fail_with(
    stub: Catalog, monkeypatch: pytest.MonkeyPatch, raised: Exception, code: str
) -> None:
    def explode(self: LakeRestCatalog) -> None:
        raise raised

    monkeypatch.setattr(LakeRestCatalog, "probe_namespaces", explode)

    result = result_of(converse(line(stub)))

    assert codes(result)[:2] == [("CATALOG_AUTH", "PASSED", None), ("WAREHOUSE", "FAILED", code)]


# ----------------------------------------------------------------------------------- STORAGE_ACCESS: the object store
def test_storage_that_vends_its_credentials_is_skipped_because_nothing_is_vended_without_a_table(stub: Catalog) -> None:
    storage = Storage()

    result = result_of(converse(line(stub, vended=True), storage=storage), vended=True)

    assert result == fixture("storage-skipped-on-vended-credentials")
    assert storage.calls == []


def test_a_storage_listing_that_takes_too_long_is_a_timeout_of_the_storage_check(stub: Catalog) -> None:
    assert result_of(converse(line(stub), storage=Storage(error=TimeoutError()))) == fixture("storage-timed-out")

    started = time.monotonic()
    slow = Storage(delay=2.0)
    result = result_of(converse(line(stub, check_timeout_ms=200), storage=slow))
    assert codes(result)[2] == ("STORAGE_ACCESS", "FAILED", TIMEOUT)
    assert time.monotonic() - started < 3.5


def test_the_storage_check_has_the_time_of_a_check_as_a_whole_and_not_of_each_location(stub: Catalog) -> None:
    # four locations that each answer in half the time the check has: no one of them is too slow, and together they are
    locations = [WAREHOUSE, "s3://lake/b/", "s3://lake/c/", "s3://lake/d/"]
    started = time.monotonic()

    result = result_of(converse(line(stub, check_timeout_ms=400, locations=locations), storage=Storage(delay=0.2)))

    assert codes(result)[2] == ("STORAGE_ACCESS", "FAILED", TIMEOUT)
    assert time.monotonic() - started < 1.5


def test_a_check_that_ran_out_leaves_no_work_behind_it(stub: Catalog) -> None:
    locations = [WAREHOUSE, "s3://lake/b/", "s3://lake/c/"]
    hold = threading.Event()
    storage = Storage(hold=hold)
    # threads of the checks of other tests may still be asleep: this one is about the threads of this probe
    before = set(threading.enumerate())

    result = result_of(converse(line(stub, check_timeout_ms=200, locations=locations), storage=storage))
    assert codes(result)[2] == ("STORAGE_ACCESS", "FAILED", TIMEOUT)

    # the thread of the check is still in its first call, and nobody is waiting for it. It is a daemon: a stuck thread
    # must not keep the host alive (with the secrets of the request in its memory) after the answer is written
    left = [thread for thread in threading.enumerate() if thread.name == "probe-check" and thread not in before]
    assert [thread.daemon for thread in left] == [True]
    # when it is let go it must not go on to ask the customer's storage about the locations the answer no longer needs
    hold.set()
    for thread in left:
        thread.join(timeout=10)
    assert [call[0] for call in storage.calls] == locations[:1]


def test_a_check_that_ran_out_does_not_keep_the_host_alive(stub: Catalog) -> None:
    started = time.monotonic()

    result = result_of(converse(line(stub, check_timeout_ms=200), storage=Storage(delay=4.0)))

    assert codes(result)[2] == ("STORAGE_ACCESS", "FAILED", TIMEOUT)
    # the thread is still asleep for seconds: the host has written its answer and ends, and does not wait to join it
    assert time.monotonic() - started < 2.0


def test_a_storage_that_is_down_is_unavailable(stub: Catalog) -> None:
    result = result_of(converse(line(stub), storage=Storage(error=OSError(S3_REFUSED))))
    assert result == fixture("storage-unavailable")


def test_storage_that_refuses_the_credentials_is_an_authentication_failure(stub: Catalog) -> None:
    result = result_of(converse(line(stub), storage=Storage(error=OSError(S3_DENIED))))
    assert codes(result)[2] == ("STORAGE_ACCESS", "FAILED", AUTHENTICATION)


def test_every_result_of_the_corpus_that_a_host_can_write_is_reproduced_above() -> None:
    # the server's corpus is the contract: a result added to it is one of these scenarios or a decision about one
    written_by_the_server_only = {"reordered", "reordered.canonical"}
    reproduced = {
        "all-passed",
        "catalog-auth-failed",
        "catalog-host-refused",
        "catalog-tls-failed",
        "storage-skipped-on-vended-credentials",
        "storage-timed-out",
        "storage-unavailable",
        "warehouse-location-not-allowed",
    }
    names = {path.stem for path in (CORPUS / "probe-results/accepted").glob("*.json")}
    assert names - written_by_the_server_only == reproduced


# ------------------------------------------------------------------------ a fault is a failure, and a check is a result
def test_a_static_credential_that_is_missing_is_a_failure_of_the_attempt_and_nothing_is_asked(stub: Catalog) -> None:
    document = json.loads(line(stub))
    document["connection"]["credentials"] = {"catalog": {"token": CATALOG_CANARY}}
    conversation = converse(json.dumps(document).encode())
    assert failure_code(conversation) == "DATABASE_CREDENTIAL_UNAVAILABLE"
    assert stub.headers == []


def test_a_request_that_is_not_a_probe_is_a_protocol_failure(stub: Catalog) -> None:
    document = json.loads(line(stub))
    for broken in (
        {**document, "extra": 1},
        {**document, "operation": "icebergWrite"},
        {**document, "probe": {**document["probe"], "checks": ["CATALOG_AUTH"]}},
    ):
        assert failure_code(converse(json.dumps(broken).encode())) == "LAKE_HOST_PROTOCOL"
    assert stub.headers == []


def test_a_failure_that_no_one_expected_is_a_worker_error_with_only_its_class_on_stderr(
    stub: Catalog, capfd: pytest.CaptureFixture[str]
) -> None:
    conversation = converse(line(stub), storage=Storage(error=RuntimeError(f"secret {STORAGE_CANARY}")))

    assert outcome(conversation) == {"failure": {"code": "WORKER_ERROR"}}
    captured = capfd.readouterr()
    assert STORAGE_CANARY not in captured.out + captured.err
    assert captured.err.strip() == "RuntimeError"


def test_a_failure_of_the_closed_set_that_a_check_raises_is_passed_on(stub: Catalog) -> None:
    conversation = converse(line(stub), storage=Storage(error=Failure("DATABASE_CREDENTIAL_UNAVAILABLE")))
    assert failure_code(conversation) == "DATABASE_CREDENTIAL_UNAVAILABLE"


# -------------------------------------------------------------------------------------------------- what never leaks
def test_no_credential_reaches_the_result_or_the_standard_streams(
    stub: Catalog, capfd: pytest.CaptureFixture[str]
) -> None:
    passed = converse(line(stub))
    stub.get_status = 401
    refused = converse(line(stub))
    stub.get_status = None
    broken_storage = converse(line(stub), storage=Storage(error=OSError(f"{S3_DENIED} {STORAGE_CANARY}")))
    protocol = converse(json.dumps({**json.loads(line(stub)), "extra": 1}).encode())

    for conversation in (passed, refused, broken_storage, protocol):
        wire = b"".join(conversation.lines)
        assert STORAGE_CANARY.encode() not in wire
        assert CATALOG_CANARY.encode() not in wire
    captured = capfd.readouterr()
    for canary in (STORAGE_CANARY, CATALOG_CANARY):
        assert canary not in captured.out + captured.err


def test_a_result_has_nothing_in_it_but_the_closed_vocabulary(stub: Catalog) -> None:
    stub.get_status = 503
    result = result_of(converse(line(stub)))

    def words(value: Any) -> set[str]:
        """Every key and every text of a document: what it could say."""
        if isinstance(value, dict):
            return {key for key in value} | set().union(*(words(item) for item in value.values()))
        if isinstance(value, list):
            return set().union(*(words(item) for item in value))
        return {value} if isinstance(value, str) else set()

    assert words(result) == {
        "kind",
        "probe",
        "connection",
        "name",
        "lake-main",
        "revision",
        "checks",
        "check",
        "outcome",
        "code",
        *PROBE_CHECKS,
        "FAILED",
        "SKIPPED",
        CATALOG_UNAVAILABLE,
    }
    # (the revision is a number, and a number is not text)
    assert result["connection"]["revision"] == 3


# --------------------------------------------------------------------------------- one table with the server's
def _failure_codes() -> dict[str, str]:
    table = (ROOT / "crates/domain/src/database/failure.rs").read_text(encoding="utf-8")
    return dict(re.findall(r'^\s*(\w+) => "([A-Z0-9_]+)", (?:true|false);', table, re.MULTILINE))


def test_the_checks_and_the_codes_each_may_fail_with_are_the_servers() -> None:
    source = (ROOT / "crates/domain/src/database/lake_probe.rs").read_text(encoding="utf-8")
    by_variant = _failure_codes()

    names = re.findall(r'Self::\w+ => "([A-Z_]+)",', source)
    assert names[: len(PROBE_CHECKS)] == list(PROBE_CHECKS)

    allows = source[source.index("pub const fn allows") :]
    allows = allows[: allows.index("\n    }\n")]
    rows = re.findall(r"Self::(\w+) => matches!\(\s*code,\s*([A-Za-z| \n]+?)\s*\)", allows)
    assert len(rows) == len(PROBE_CHECKS)
    rust = {
        name: {by_variant[variant.strip()] for variant in variants.split("|")}
        for name, (_, variants) in zip(PROBE_CHECKS, rows, strict=True)
    }
    assert {name: set(allowed) for name, allowed in CHECK_CODES.items()} == rust


def test_the_codes_of_a_check_are_written_in_the_table_module_and_nowhere_else_in_the_probe() -> None:
    sources = Path(__file__).resolve().parent.parent / "sagat_lake"

    def written(name: str) -> set[str]:
        text = (sources / name).read_text(encoding="utf-8")
        return {code for code in _failure_codes().values() if f'"{code}"' in text}

    in_the_table = {code for allowed in CHECK_CODES.values() for code in allowed}
    # the table module is the one file the scans of the codes a failure line may carry skip, so it holds the table and
    # nothing else (the location code is the hygiene's own, imported)...
    assert written("probe_codes.py") == in_the_table - {"ICEBERG_LOCATION_NOT_ALLOWED"}
    # ... and the logic of the probe writes no code: one written there would be one the scans skip and nobody holds
    assert written("probe.py") == set()


# ------------------------------------------------------------------------------------------------------- the entry
def test_the_host_serves_the_probe_beside_the_write() -> None:
    operations = operations_for(Path("/tmp/nowhere"))
    assert set(operations) == {"icebergWrite", "probe"}


def test_the_request_is_not_changed_by_the_operation(stub: Catalog) -> None:
    document = json.loads(line(stub))
    before = copy.deepcopy(document)
    assert result_of(converse(json.dumps(document).encode())) == fixture("all-passed")
    assert document == before


def test_each_check_has_the_time_the_lease_gives_it(stub: Catalog) -> None:
    storage = Storage()
    assert result_of(converse(line(stub, check_timeout_ms=1500), storage=storage)) == fixture("all-passed")
    assert storage.calls == [(WAREHOUSE, 1.5)]
