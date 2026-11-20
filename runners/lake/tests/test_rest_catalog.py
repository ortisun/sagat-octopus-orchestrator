"""The REST catalog of the host (feature 035: T035-018, D035-050, D035-094, G035-041, G035-043).

PyIceberg's own client trusts what the catalog says: it merges the `defaults` and `overrides` of `/v1/config` into its
properties (the `uri` it talks to included), and it builds the file IO of a table from the table's own properties
overridden by the config of the response. `LakeRestCatalog` runs every answer through `Hygiene` before PyIceberg uses
it, and gives every request a time limit, which PyIceberg's client does not. These tests use the client of PyIceberg
itself against a catalog that lies, over real HTTP.
"""

from __future__ import annotations

import gzip
import json
import pathlib
import time
from collections.abc import Generator

import pytest
import requests
from pyiceberg.catalog.rest import RestCatalog
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.exceptions import ServerError
from pyiceberg.io.pyarrow import PyArrowFileIO
from pyiceberg.schema import Schema
from pyiceberg.types import LongType, NestedField

import counting_catalog
import network_doubles
import recording_io
from counting_catalog import Catalog
from sagat_lake.config_hygiene import LOCATION_NOT_ALLOWED, Declaration, Denial, Hygiene, HygieneError, StorageType
from sagat_lake.rest_catalog import LakeRestCatalog, ResponseTooLarge

NAME = f"{counting_catalog.NAMESPACE}.{counting_catalog.TABLE}"
PINNED_IO = "pyiceberg.io.pyarrow.PyArrowFileIO"
HOSTILE_IO = "recording_io.RecordingFileIO"
ATTACKER = "http://attacker.invalid:9"
WAREHOUSE = "s3://lake/warehouse/"
VENDED = {"s3.access-key-id": "vended-key", "s3.secret-access-key": "vended-secret", "s3.session-token": "vended-token"}
HOSTILE_KEYS = {"py-io-impl": HOSTILE_IO, "s3.endpoint": ATTACKER, "s3.proxy-uri": ATTACKER, "s3.signer": "evil.Signer"}


@pytest.fixture(autouse=True)
def loopback(monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1")
    recording_io.BUILT.clear()


@pytest.fixture
def rest(tmp_path: pathlib.Path) -> Generator[Catalog]:
    sql = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    sql.create_namespace(counting_catalog.NAMESPACE)
    sql.create_table(NAME, schema=Schema(NestedField(1, "id", LongType(), required=False)))
    with counting_catalog.serve(sql) as catalog:
        yield catalog


def said_to_be_in_the_warehouse(rest: Catalog, where: str = "s3://lake/warehouse") -> None:
    """The table the stub returns says it lives under the declared warehouse (the file IO is only built, not used)."""
    here = rest.sql.load_table(NAME).metadata.location.rsplit("/sales/", 1)[0]
    rest.rewrite = (here, where)


def hygiene(*, vended: bool = False) -> Hygiene:
    return Hygiene(Declaration(StorageType.S3, (WAREHOUSE,), vended_credentials=vended))


def lake(rest: Catalog, guard: Hygiene, *, timeout: float = 5.0) -> LakeRestCatalog:
    return LakeRestCatalog("lake", hygiene=guard, request_timeout=timeout, **guard.properties({"uri": rest.uri}, {}))


# ------------------------------------------------------------------------------------------------------ /v1/config
def test_a_catalog_cannot_redirect_the_client_or_choose_its_file_io_with_its_config(rest: Catalog) -> None:
    rest.config_defaults = {**HOSTILE_KEYS, "warehouse": WAREHOUSE}
    rest.config_overrides = {"uri": ATTACKER, "py-io-impl": HOSTILE_IO, "s3.endpoint": ATTACKER}
    guard = hygiene()

    catalog = lake(rest, guard)

    assert catalog.uri == rest.uri
    assert catalog.properties["py-io-impl"] == PINNED_IO
    assert catalog.properties["warehouse"] == WAREHOUSE
    for key in ("s3.endpoint", "s3.proxy-uri", "s3.signer"):
        assert key not in catalog.properties
    assert guard.denied[Denial.KEY_NOT_ALLOWED] >= 1
    # and the client still talks to the catalog it was given
    said_to_be_in_the_warehouse(rest)
    assert catalog.load_table(NAME).name() == ("sales", "orders")


def test_pyiceberg_alone_would_follow_the_catalog_to_the_attacker(rest: Catalog) -> None:
    # the premise of the class: the client merges the overrides of the server over its own properties
    rest.config_overrides = {"uri": ATTACKER, "py-io-impl": HOSTILE_IO}
    catalog = RestCatalog("plain", uri=rest.uri, **{"py-io-impl": PINNED_IO})
    assert catalog.uri == ATTACKER
    assert catalog.properties["py-io-impl"] == HOSTILE_IO


def test_what_the_connection_declares_wins_over_what_the_catalog_proposes(rest: Catalog) -> None:
    rest.config_defaults = {"warehouse": "s3://lake/warehouse/other/"}
    catalog = LakeRestCatalog(
        "lake",
        hygiene=hygiene(),
        request_timeout=5.0,
        **{**hygiene().properties({"uri": rest.uri}, {}), "warehouse": WAREHOUSE},
    )
    assert catalog.properties["warehouse"] == WAREHOUSE


# ------------------------------------------------------------------------------------------------------------ LoadTable
def test_a_table_cannot_choose_its_file_io_through_its_own_properties(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    rest.properties = {**HOSTILE_KEYS, "write.format.default": "parquet", "commit.retry.min-wait-ms": "1"}
    rest.table_config = dict(HOSTILE_KEYS)

    table = lake(rest, hygiene()).load_table(NAME)

    assert recording_io.BUILT == []
    assert isinstance(table.io, PyArrowFileIO)
    assert table.metadata.properties["write.format.default"] == "parquet"
    for key in HOSTILE_KEYS:
        assert key not in table.metadata.properties
        assert table.io.properties.get(key) in (None, PINNED_IO)
    assert table.io.properties["py-io-impl"] == PINNED_IO


def test_pyiceberg_alone_builds_the_file_io_class_a_table_names(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    rest.properties = {"py-io-impl": HOSTILE_IO}
    RestCatalog("plain", uri=rest.uri, **{"py-io-impl": PINNED_IO}).load_table(NAME)
    assert recording_io.BUILT


def test_a_table_the_catalog_places_outside_the_warehouse_is_refused_before_a_file_io_exists(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest, "s3://attacker-bucket/stolen")
    guard = hygiene()
    with pytest.raises(HygieneError) as refused:
        lake(rest, guard).load_table(NAME)
    assert refused.value.code == LOCATION_NOT_ALLOWED
    assert guard.denied[Denial.LOCATION_NOT_ALLOWED] == 1
    assert "attacker-bucket" not in str(refused.value)


def test_vended_credentials_are_kept_only_when_the_connection_declares_them(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    rest.table_config = {**VENDED, "s3.endpoint": ATTACKER}

    declared = lake(rest, hygiene(vended=True)).load_table(NAME)
    assert declared.io.properties["s3.access-key-id"] == "vended-key"
    assert "s3.endpoint" not in declared.io.properties

    refused = lake(rest, hygiene(vended=False)).load_table(NAME)
    assert "s3.access-key-id" not in refused.io.properties


def test_storage_credentials_of_the_warehouse_are_filtered_and_the_others_are_dropped(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    rest.storage_credentials = [
        {"prefix": "s3://lake/warehouse/sales", "config": {"s3.access-key-id": "inside", "s3.endpoint": ATTACKER}},
        {"prefix": "s3://attacker-bucket/", "config": {"s3.access-key-id": "outside"}},
    ]

    table = lake(rest, hygiene(vended=True)).load_table(NAME)

    assert table.io.properties["s3.access-key-id"] == "inside"
    assert "s3.endpoint" not in table.io.properties


def test_storage_credentials_are_dropped_when_the_connection_does_not_declare_them(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    rest.storage_credentials = [{"prefix": "s3://lake/warehouse/sales", "config": {"s3.access-key-id": "inside"}}]
    table = lake(rest, hygiene(vended=False)).load_table(NAME)
    assert "s3.access-key-id" not in table.io.properties


# --------------------------------------------------------------------------------------------------------- the time
def test_a_catalog_that_does_not_answer_in_time_is_an_error_and_not_a_wait_without_end(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    catalog = lake(rest, hygiene(), timeout=0.3)
    rest.delay = 3.0

    started = time.monotonic()
    with pytest.raises(requests.exceptions.ReadTimeout):
        catalog.load_table(NAME)

    assert time.monotonic() - started < 2.0


def test_the_first_request_of_the_client_has_a_limit_too(rest: Catalog) -> None:
    rest.delay = 3.0
    started = time.monotonic()
    with pytest.raises(requests.exceptions.ReadTimeout):
        lake(rest, hygiene(), timeout=0.3)
    assert time.monotonic() - started < 2.0


def test_a_catalog_that_answers_within_the_limit_is_used(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    rest.delay = 0.05
    assert lake(rest, hygiene(), timeout=2.0).load_table(NAME).name() == ("sales", "orders")


def test_every_request_of_the_session_has_a_limit_the_commit_included(rest: Catalog) -> None:
    # PyIceberg posts a commit with `session.post(url, data=...)` and no timeout; the limit is the session's
    catalog = lake(rest, hygiene(), timeout=0.3)
    rest.delay = 3.0
    started = time.monotonic()
    with pytest.raises(requests.exceptions.ReadTimeout):
        catalog._session.post(f"{rest.uri}/v1/namespaces/sales/tables/orders", data=b"{}")
    assert time.monotonic() - started < 2.0


def test_an_explicit_timeout_of_a_caller_is_not_overridden(rest: Catalog) -> None:
    catalog = lake(rest, hygiene(), timeout=0.2)
    rest.delay = 0.6
    response = catalog._session.get(f"{rest.uri}/v1/config", timeout=5)
    assert response.status_code == 200


def test_a_limit_that_is_not_positive_is_refused(rest: Catalog) -> None:
    for bad in (0, -1):
        with pytest.raises(ValueError, match="a request timeout is positive"):
            lake(rest, hygiene(), timeout=bad)


def test_the_file_io_is_pinned_even_when_the_caller_forgot_to_name_it(rest: Catalog) -> None:
    rest.config_overrides = {"py-io-impl": HOSTILE_IO}
    catalog = LakeRestCatalog("lake", hygiene=hygiene(), request_timeout=5.0, uri=rest.uri)
    assert catalog.properties["py-io-impl"] == PINNED_IO


# ------------------------------------------------------------------------------------------- access delegation
def delegation_headers(rest: Catalog) -> list[str | None]:
    return [headers.get("X-Iceberg-Access-Delegation") for headers in rest.headers]


def test_pyiceberg_alone_asks_every_catalog_for_vended_credentials(rest: Catalog) -> None:
    # the premise: the plain client sets the header by default, whatever the connection says
    RestCatalog("plain", uri=rest.uri, **{"py-io-impl": PINNED_IO})
    assert delegation_headers(rest) == ["vended-credentials"]


def test_a_connection_that_declares_no_delegation_does_not_ask_for_any(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    lake(rest, hygiene(vended=False)).load_table(NAME)
    assert delegation_headers(rest)
    assert set(delegation_headers(rest)) == {None}


def test_a_connection_that_declares_vended_credentials_asks_for_them(rest: Catalog) -> None:
    said_to_be_in_the_warehouse(rest)
    lake(rest, hygiene(vended=True)).load_table(NAME)
    assert set(delegation_headers(rest)) == {"vended-credentials"}


def test_remote_signing_is_asked_for_next_to_vended_credentials(rest: Catalog) -> None:
    guard = Hygiene(Declaration(StorageType.S3, (WAREHOUSE,), vended_credentials=True, remote_signing=True))
    LakeRestCatalog("lake", hygiene=guard, request_timeout=5.0, **guard.properties({"uri": rest.uri}, {}))
    assert set(delegation_headers(rest)) == {"vended-credentials,remote-signing"}


def test_a_header_property_cannot_ask_for_more_than_the_connection_declares(rest: Catalog) -> None:
    guard = hygiene(vended=False)
    properties = {**guard.properties({"uri": rest.uri}, {}), "header.X-Iceberg-Access-Delegation": "vended-credentials"}
    LakeRestCatalog("lake", hygiene=guard, request_timeout=5.0, **properties)
    assert set(delegation_headers(rest)) == {None}


# ----------------------------------------------------------------------------------- the token of OAuth2 and the bodies
@pytest.mark.filterwarnings("ignore::DeprecationWarning")
def test_the_request_for_an_oauth2_token_has_the_time_limit_of_every_other(rest: Catalog) -> None:
    # PyIceberg's legacy manager asks the token endpoint for a token as it is built, with the session it is given, so
    # the limit has to be on that session before the manager is: an endpoint that never answers must not be waited for
    guard = hygiene()
    with network_doubles.silent_server(hold=2.5) as port:
        properties = guard.properties(
            {"uri": rest.uri, "credential": "id:secret", "oauth2-server-uri": f"http://127.0.0.1:{port}"}, {}
        )
        started = time.monotonic()
        with pytest.raises(requests.exceptions.Timeout):
            LakeRestCatalog("lake", hygiene=guard, request_timeout=0.3, **properties)
        elapsed = time.monotonic() - started
    assert elapsed < 2.0


def test_a_configuration_larger_than_any_catalog_has_is_refused_unread(rest: Catalog) -> None:
    rest.config_defaults = {"warehouse": WAREHOUSE, "filler": "x" * (2 * 1024 * 1024)}
    with pytest.raises(ResponseTooLarge):
        lake(rest, hygiene())


def test_an_error_body_larger_than_any_catalog_has_is_refused_unread(rest: Catalog) -> None:
    # PyIceberg reads the text of a failed answer to say what failed: that is a read of the whole body as well
    rest.config_raw = (500, b"x" * (2 * 1024 * 1024))
    with pytest.raises(ResponseTooLarge):
        lake(rest, hygiene())


def test_a_failed_answer_of_a_sensible_size_is_still_what_it_says(rest: Catalog) -> None:
    rest.config_raw = (500, b'{"error": {"message": "down", "type": "ServerError", "code": 500}}')
    with pytest.raises(ServerError):
        lake(rest, hygiene())


def test_a_page_of_namespaces_larger_than_any_catalog_sends_is_refused_unread(rest: Catalog) -> None:
    catalog = lake(rest, hygiene())
    big = json.dumps({"namespaces": [["n" * 100]] * 20_000}).encode()
    assert len(big) > 1024 * 1024
    rest.namespaces_raw = ({}, big)
    with pytest.raises(ResponseTooLarge):
        catalog.probe_namespaces()


def test_a_compressed_page_that_grows_past_the_limit_is_refused_as_it_is_decoded(rest: Catalog) -> None:
    catalog = lake(rest, hygiene())
    # 64 MiB of spaces is a few tens of kilobytes on the wire: the limit is on what the client would hold
    bomb = gzip.compress(b" " * (64 * 1024 * 1024) + b'{"namespaces": []}')
    assert len(bomb) < 1024 * 1024
    rest.namespaces_raw = ({"Content-Encoding": "gzip"}, bomb)
    with pytest.raises(ResponseTooLarge):
        catalog.probe_namespaces()


def test_a_page_of_namespaces_that_is_one_is_read(rest: Catalog) -> None:
    catalog = lake(rest, hygiene())
    catalog.probe_namespaces()
    assert len(rest.namespace_requests) == 1
