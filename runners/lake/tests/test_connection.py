"""The pinned connection of a request, as what the catalog client is built with (T035-018, D035-105).

The runner sends the connection the way the platform stored it (``data-model.md`` §1) next to the secrets it resolved.
The host turns it into three things and nothing else: the hygiene filter of this attempt (the storage type, the
warehouse locations, what the catalog may delegate), the properties of the REST client, and a catalog. Whatever the
host does not serve yet is a break of the protocol, and so is anything it does not know: the line is the platform's,
but a definition that was published and a secret that was resolved are not a reason to be generous.
"""

from __future__ import annotations

import copy
import pathlib
from collections.abc import Generator
from typing import Any

import pytest
import requests
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.schema import Schema
from pyiceberg.types import LongType, NestedField

import counting_catalog
from counting_catalog import Catalog
from sagat_lake.config_hygiene import CREDENTIAL_UNAVAILABLE, HygieneError
from sagat_lake.connection import open_connection
from sagat_lake.failures import Failure

WAREHOUSE = "s3://lake/warehouse/"


def connection(**changes: Any) -> dict[str, Any]:
    base: dict[str, Any] = {
        "catalog": {
            "type": "rest",
            "rest": {"uri": "https://catalog.example/api", "warehouse": WAREHOUSE, "accessDelegation": "none"},
        },
        "storage": {
            "type": "s3",
            "s3": {"region": "eu-west-1", "pathStyle": False},
            "credentials": "static",
        },
        "warehouseLocations": [WAREHOUSE],
        "credentials": {"storage": {"accessKeyId": "AKIA", "secretAccessKey": "secret"}},
    }
    return {**base, **changes}


def changed(path: tuple[str, ...], value: Any) -> dict[str, Any]:
    """The connection with the part at ``path`` replaced (or removed, for ``...``)."""
    document = copy.deepcopy(connection())
    parent = document
    for key in path[:-1]:
        parent = parent[key]
    if value is ...:
        del parent[path[-1]]
    else:
        parent[path[-1]] = value
    return document


def refused(document: dict[str, Any]) -> Failure:
    with pytest.raises(Failure) as caught:
        open_connection(document)
    failure = caught.value
    assert (failure.code, failure.reason, failure.column) == ("LAKE_HOST_PROTOCOL", None, None)
    return failure


# ----------------------------------------------------------------------------------------------------- what is built
def test_a_rest_catalog_with_static_s3_credentials_becomes_the_properties_of_the_client() -> None:
    opened = open_connection(connection())
    assert opened.properties == {
        "uri": "https://catalog.example/api",
        "warehouse": WAREHOUSE,
        "s3.region": "eu-west-1",
        "s3.force-virtual-addressing": "true",
        "s3.access-key-id": "AKIA",
        "s3.secret-access-key": "secret",
    }
    assert opened.hygiene.access_delegation is None


def test_the_hygiene_is_made_of_the_storage_the_warehouse_and_what_may_be_delegated() -> None:
    hygiene = open_connection(connection()).hygiene
    assert hygiene.check_location("s3://lake/warehouse/sales/orders") == "s3://lake/warehouse/sales/orders"
    with pytest.raises(HygieneError):
        hygiene.check_location("s3://elsewhere/x")


def test_a_session_token_a_prefix_a_path_style_and_an_endpoint_are_carried() -> None:
    document = connection()
    document["catalog"]["rest"]["prefix"] = "tenant-a"
    document["storage"]["s3"] = {"region": "us-east-1", "pathStyle": True, "endpoint": "https://minio.example:9000"}
    document["credentials"]["storage"]["sessionToken"] = "tok"
    properties = open_connection(document).properties
    assert properties["prefix"] == "tenant-a"
    assert properties["s3.force-virtual-addressing"] == "false"
    assert properties["s3.endpoint"] == "https://minio.example:9000"
    assert properties["s3.session-token"] == "tok"
    assert properties["s3.region"] == "us-east-1"


def test_oauth2_becomes_a_credential_a_server_uri_and_a_scope() -> None:
    document = connection()
    document["catalog"]["rest"]["auth"] = {
        "type": "oauth2",
        "tokenUri": "https://auth.example/token",
        "scope": "catalog",
    }
    document["credentials"]["catalog"] = {"clientId": "id", "clientSecret": "s3cr3t"}
    properties = open_connection(document).properties
    assert properties["credential"] == "id:s3cr3t"
    assert properties["oauth2-server-uri"] == "https://auth.example/token"
    assert properties["scope"] == "catalog"


def test_oauth2_without_a_scope_has_none() -> None:
    document = connection()
    document["catalog"]["rest"]["auth"] = {"type": "oauth2", "tokenUri": "https://auth.example/token"}
    document["credentials"]["catalog"] = {"clientId": "id", "clientSecret": "s3cr3t"}
    assert "scope" not in open_connection(document).properties


def test_a_bearer_token_becomes_the_token() -> None:
    document = connection()
    document["catalog"]["rest"]["auth"] = {"type": "bearer"}
    document["credentials"]["catalog"] = {"token": "bearer-token"}
    assert open_connection(document).properties["token"] == "bearer-token"


def test_vended_credentials_are_declared_to_the_hygiene_and_no_secret_is_sent_for_the_storage() -> None:
    document = connection()
    document["catalog"]["rest"]["accessDelegation"] = "vended-credentials"
    document["storage"]["credentials"] = "vended"
    del document["credentials"]["storage"]
    opened = open_connection(document)
    assert opened.hygiene.access_delegation == "vended-credentials"
    assert not [key for key in opened.properties if key.startswith("s3.access") or key.startswith("s3.secret")]
    assert open_connection(document).hygiene.table_config({"s3.access-key-id": "k"}) == {"s3.access-key-id": "k"}


def test_an_http_catalog_on_the_loopback_is_allowed_for_the_tests_of_the_platform() -> None:
    document = changed(("catalog", "rest", "uri"), "http://127.0.0.1:8181")
    assert open_connection(document).properties["uri"] == "http://127.0.0.1:8181"
    document = changed(("catalog", "rest", "uri"), "http://localhost:8181/api")
    assert open_connection(document).properties["uri"] == "http://localhost:8181/api"


def test_the_secrets_are_not_part_of_a_representation() -> None:
    opened = open_connection(connection())
    assert "secret" not in repr(opened)
    assert "AKIA" not in repr(opened)


# ------------------------------------------------------------------------------------------------------ credentials
def test_static_credentials_that_are_missing_fail_closed_with_the_code_of_the_hygiene() -> None:
    document = changed(("credentials", "storage"), {"accessKeyId": "AKIA"})
    with pytest.raises(Failure) as caught:
        open_connection(document)
    assert caught.value.code == CREDENTIAL_UNAVAILABLE


def test_no_credentials_at_all_fail_closed_too() -> None:
    document = changed(("credentials",), {})
    with pytest.raises(Failure) as caught:
        open_connection(document)
    assert caught.value.code == CREDENTIAL_UNAVAILABLE


def test_credentials_for_vended_storage_are_a_break_because_nothing_will_use_them() -> None:
    document = connection()
    document["catalog"]["rest"]["accessDelegation"] = "vended-credentials"
    document["storage"]["credentials"] = "vended"
    refused(document)  # the static secret is still in `credentials.storage`


def test_vended_storage_needs_the_catalog_to_be_asked_for_vended_credentials() -> None:
    document = connection()
    document["storage"]["credentials"] = "vended"
    del document["credentials"]["storage"]
    refused(document)


def test_static_storage_does_not_let_the_catalog_delegate() -> None:
    document = connection()
    document["catalog"]["rest"]["accessDelegation"] = "vended-credentials"
    refused(document)


@pytest.mark.parametrize("client_id", ["a:b", "", "x" * 257])
def test_a_client_id_that_cannot_be_joined_with_its_secret_is_a_break(client_id: str) -> None:
    document = connection()
    document["catalog"]["rest"]["auth"] = {"type": "oauth2", "tokenUri": "https://auth.example/token"}
    document["credentials"]["catalog"] = {"clientId": client_id, "clientSecret": "s"}
    refused(document)


def test_the_credentials_of_the_catalog_must_be_those_of_its_auth() -> None:
    document = connection()
    document["catalog"]["rest"]["auth"] = {"type": "bearer"}
    document["credentials"]["catalog"] = {"clientId": "id", "clientSecret": "s"}
    refused(document)
    document["catalog"]["rest"]["auth"] = {"type": "oauth2", "tokenUri": "https://auth.example/token"}
    document["credentials"]["catalog"] = {"token": "t"}
    refused(document)


def test_a_catalog_without_auth_takes_no_catalog_credentials() -> None:
    document = connection()
    document["credentials"]["catalog"] = {"token": "t"}
    refused(document)


# -------------------------------------------------------------------------------------------- what is not served yet
@pytest.mark.parametrize(
    "path_and_value",
    [
        (("catalog", "type"), "glue"),
        (("catalog", "type"), "hive"),
        (("catalog", "type"), "jdbc"),
        (("catalog", "rest", "auth"), {"type": "sigv4", "region": "eu-west-1", "signingName": "execute-api"}),
        (("catalog", "rest", "remoteSigning"), True),
        (("storage", "type"), "gcs"),
        (("storage", "type"), "adls"),
        (("storage", "s3", "roleArn"), "arn:aws:iam::1:role/x"),
        (("storage", "s3", "externalId"), "x"),
    ],
)
def test_what_the_host_does_not_serve_yet_is_a_break(path_and_value: tuple[tuple[str, ...], Any]) -> None:
    refused(changed(*path_and_value))


# ------------------------------------------------------------------------------------------------ shape and limits
@pytest.mark.parametrize(
    "path",
    [
        ("catalog",),
        ("catalog", "rest"),
        ("catalog", "rest", "uri"),
        ("catalog", "rest", "warehouse"),
        ("catalog", "rest", "accessDelegation"),
        ("storage",),
        ("storage", "s3"),
        ("storage", "s3", "region"),
        ("storage", "s3", "pathStyle"),
        ("storage", "credentials"),
        ("warehouseLocations",),
    ],
)
def test_a_part_of_the_connection_that_is_missing_is_a_break(path: tuple[str, ...]) -> None:
    refused(changed(path, ...))


@pytest.mark.parametrize(
    "path",
    [("catalog",), ("catalog", "rest"), ("storage",), ("storage", "s3"), ("credentials",), ("credentials", "storage")],
)
def test_a_part_of_the_connection_with_a_field_the_host_does_not_know_is_a_break(path: tuple[str, ...]) -> None:
    document = connection()
    part = document
    for key in path:
        part = part[key]
    part["extra"] = "x"
    refused(document)


@pytest.mark.parametrize(
    "uri",
    [
        "http://catalog.example/api",  # credentials in the clear outside the loopback
        "ftp://catalog.example",
        "https://user:pw@catalog.example",
        "https://catalog.example/api?token=x",
        "https://catalog.example/api#frag",
        "https://",
        "catalog.example",
        "",
        "https://catalog.example/\x00",
        "https://catalog.example/a b",
        "https://catalog.example/" + "a" * 2048,
        5,
    ],
)
def test_an_address_that_would_leak_or_is_not_one_is_a_break(uri: Any) -> None:
    refused(changed(("catalog", "rest", "uri"), uri))


@pytest.mark.parametrize("value", ["", "a b", "a" * 65, "UPPER", "a/b", 5, None])
def test_a_region_that_is_not_one_is_a_break(value: Any) -> None:
    refused(changed(("storage", "s3", "region"), value))


@pytest.mark.parametrize(
    "endpoint", ["http://minio.example", "minio.example", "https://u:p@minio.example", "https://x?y=1", ""]
)
def test_a_storage_endpoint_that_would_leak_or_is_not_one_is_a_break(endpoint: str) -> None:
    refused(changed(("storage", "s3", "endpoint"), endpoint))


@pytest.mark.parametrize("value", ["yes", 1, None])
def test_the_path_style_is_a_boolean(value: Any) -> None:
    refused(changed(("storage", "s3", "pathStyle"), value))


@pytest.mark.parametrize("prefix", ["", "a b", "../x", "a" * 257, 5])
def test_a_prefix_that_is_not_a_path_segment_is_a_break(prefix: Any) -> None:
    refused(changed(("catalog", "rest", "prefix"), prefix))


@pytest.mark.parametrize("value", ["", "everything", 5, None])
def test_the_delegation_is_one_of_the_two_the_platform_knows(value: Any) -> None:
    refused(changed(("catalog", "rest", "accessDelegation"), value))


@pytest.mark.parametrize(
    "locations",
    [["gs://lake/warehouse/"], ["s3://"], ["s3://lake/../x/"], ["/etc"], [""], ["s3://lake/a b/"]],
)
def test_a_warehouse_location_that_does_not_fit_the_storage_is_a_break(locations: list[str]) -> None:
    refused(changed(("warehouseLocations",), locations))


def test_a_token_uri_is_held_to_the_same_rule_as_the_address() -> None:
    for uri in ("http://auth.example/token", "https://u:p@auth.example/token", "https://auth.example/token?x=1", ""):
        document = connection()
        document["catalog"]["rest"]["auth"] = {"type": "oauth2", "tokenUri": uri}
        document["credentials"]["catalog"] = {"clientId": "id", "clientSecret": "s"}
        refused(document)


def test_a_failure_carries_nothing_of_the_connection() -> None:
    failure = refused(changed(("catalog", "rest", "uri"), "http://secret-host.example/"))
    assert str(failure) == "LAKE_HOST_PROTOCOL"
    assert "secret-host" not in repr(failure.as_member())


# ------------------------------------------------------------------------------------------ the catalog it opens
@pytest.fixture
def stub(tmp_path: pathlib.Path) -> Generator[Catalog]:
    sql = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    sql.create_namespace("sales")
    sql.create_table("sales.orders", schema=Schema(NestedField(1, "id", LongType(), required=False)))
    with counting_catalog.serve(sql) as catalog:
        yield catalog


def test_the_catalog_of_a_connection_loads_a_table_and_sends_only_what_the_connection_declares(
    stub: Catalog, monkeypatch: pytest.MonkeyPatch
) -> None:
    for name in ("HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1")
    here = stub.sql.load_table("sales.orders").metadata.location.rsplit("/sales/", 1)[0]
    stub.rewrite = (here, "s3://lake/warehouse")
    stub.config_overrides = {"py-io-impl": "evil.IO", "uri": "http://attacker.invalid:9"}
    document = changed(("catalog", "rest", "uri"), stub.uri)

    catalog = open_connection(document).catalog(request_timeout=5.0)
    table = catalog.load_table("sales.orders")

    assert table.name() == ("sales", "orders")
    assert catalog.properties["py-io-impl"] == "pyiceberg.io.pyarrow.PyArrowFileIO"
    assert {headers.get("X-Iceberg-Access-Delegation") for headers in stub.headers} == {None}
    assert table.io.properties["s3.access-key-id"] == "AKIA"
    assert table.io.properties["s3.region"] == "eu-west-1"


def test_the_catalog_of_a_connection_has_a_time_limit(stub: Catalog, monkeypatch: pytest.MonkeyPatch) -> None:
    for name in ("HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1")
    stub.delay = 3.0
    with pytest.raises(requests.exceptions.ReadTimeout):
        open_connection(changed(("catalog", "rest", "uri"), stub.uri)).catalog(request_timeout=0.3)
