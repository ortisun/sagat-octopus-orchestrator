"""Configuration hygiene of the lake host (feature 035, T035-018, D035-050, FR-035-004).

A catalog is not trusted: what it sends in `/v1/config` and in `LoadTable` is filtered to a closed allowlist, the
FileIO implementation is pinned, remote signing is refused unless the connection declares it, a table that points
outside the declared warehouse locations is refused, the ambient credential chains are closed, and every denial is
counted. The hostile catalog of `hostile_catalog.py` is queried over real HTTP, so the documents are the ones a
client receives.
"""

from __future__ import annotations

import pytest

import hostile_catalog as hostile
from sagat_lake.config_hygiene import (
    CREDENTIAL_UNAVAILABLE,
    LOCATION_NOT_ALLOWED,
    Declaration,
    Denial,
    Hygiene,
    HygieneError,
    StorageType,
)

LAKE = ("s3://lake/warehouse/",)


def s3(*, vended: bool = False, signing: bool = False, locations: tuple[str, ...] = LAKE) -> Hygiene:
    return Hygiene(
        Declaration(
            StorageType.S3,
            locations,
            vended_credentials=vended,
            remote_signing=signing,
        )
    )


def as_dict(value: object) -> dict[str, object]:
    assert isinstance(value, dict)
    return value


# -- /v1/config --------------------------------------------------------------------------------------------------------


def test_every_override_of_a_hostile_catalog_config_is_dropped_and_counted() -> None:
    with hostile.serve() as url:
        document = hostile.fetch(url, "/v1/config")
    hygiene = s3()

    accepted = hygiene.catalog_config(as_dict(document["defaults"]), as_dict(document["overrides"]))

    assert accepted == {"warehouse": hostile.WAREHOUSE, "prefix": "tenant-a"}
    # nine hostile keys in the defaults and two in the overrides, one denial for each
    assert hygiene.denials == len(hostile.HOSTILE_KEYS) + 2
    assert hygiene.denied[Denial.KEY_NOT_ALLOWED] == hygiene.denials


def test_an_override_wins_over_a_default_only_for_the_keys_that_are_accepted() -> None:
    accepted = s3().catalog_config({"warehouse": "s3://lake/warehouse", "prefix": "a"}, {"prefix": "b"})

    assert accepted == {"warehouse": "s3://lake/warehouse", "prefix": "b"}


def test_a_value_that_is_not_a_string_is_dropped_even_under_an_allowed_key() -> None:
    hygiene = s3()

    accepted = hygiene.catalog_config({"warehouse": 5, "prefix": ["a"]}, {})

    assert accepted == {}
    assert hygiene.denials == 2


def test_a_warehouse_value_is_bounded() -> None:
    hygiene = s3()

    assert hygiene.catalog_config({"warehouse": "a" * 1024}, {}) == {"warehouse": "a" * 1024}
    assert hygiene.denials == 0
    assert hygiene.catalog_config({"warehouse": "a" * 1025}, {}) == {}
    assert hygiene.denials == 1


@pytest.mark.parametrize("prefix", ["../other", "a/../b", "a b", "a\nb", "", "x" * 257, "a?x=1", "a#f", "%2e%2e"])
def test_a_prefix_that_is_not_a_path_of_safe_characters_is_dropped(prefix: str) -> None:
    hygiene = s3()

    assert hygiene.catalog_config({"prefix": prefix}, {}) == {}
    assert hygiene.denials == 1


def test_a_warehouse_that_is_a_location_outside_the_declared_ones_is_refused() -> None:
    hygiene = s3()

    with pytest.raises(HygieneError) as refused:
        hygiene.catalog_config({"warehouse": "s3://attacker-bucket/w"}, {})

    assert refused.value.code == LOCATION_NOT_ALLOWED
    assert hygiene.denied[Denial.LOCATION_NOT_ALLOWED] == 1
    # a warehouse that is an identifier of the catalog, not a location, is accepted as it is
    assert s3().catalog_config({"warehouse": "analytics"}, {}) == {"warehouse": "analytics"}


# -- LoadTable ---------------------------------------------------------------------------------------------------------


def test_vended_credentials_are_kept_only_when_the_connection_declares_them_and_the_rest_is_dropped() -> None:
    with hostile.serve() as url:
        document = hostile.fetch(url, "/v1/namespaces/hostile/tables/honest")
    declared = s3(vended=True)

    accepted = declared.table_config(as_dict(document["config"]))

    assert accepted == hostile.vended_credentials()
    assert declared.denials == len(hostile.HOSTILE_KEYS)

    undeclared = s3(vended=False)
    assert undeclared.table_config(as_dict(document["config"])) == {}
    assert undeclared.denials == len(hostile.HOSTILE_KEYS) + len(hostile.vended_credentials())


def test_the_vended_keys_of_another_storage_type_are_dropped() -> None:
    hygiene = s3(vended=True)

    accepted = hygiene.table_config({"gcs.oauth2.token": "t", "adls.sas-token": "s", **hostile.vended_credentials()})

    assert accepted == hostile.vended_credentials()
    assert hygiene.denials == 2


def test_a_vended_region_never_replaces_the_declared_one() -> None:
    hygiene = s3(vended=True)

    accepted = hygiene.table_config({"s3.region": "attacker-region-1", "client.region": "attacker-region-1"})

    assert accepted == {}
    assert hygiene.denials == 2


def test_remote_signing_is_refused_unless_the_connection_declares_it() -> None:
    signer = {"s3.signer": "x", "s3.signer.uri": "https://catalog.example/sign", "s3.signer.endpoint": "v1/sign"}
    refused = s3(signing=False)
    allowed = s3(signing=True)

    assert refused.table_config(signer) == {}
    assert refused.denied[Denial.KEY_NOT_ALLOWED] == 3
    assert allowed.table_config(signer) == signer
    assert allowed.denials == 0


def test_the_file_io_implementation_is_pinned_whatever_the_catalog_says() -> None:
    hygiene = s3(vended=True)
    document = hostile.config_response()
    accepted = hygiene.catalog_config(as_dict(document["defaults"]), as_dict(document["overrides"]))

    properties = hygiene.properties({"s3.endpoint": "https://s3.eu-west-1.amazonaws.com"}, accepted)

    assert properties["py-io-impl"] == "pyiceberg.io.pyarrow.PyArrowFileIO"
    assert hostile.METADATA_SERVICE not in str(properties)
    # even a catalog key that reached this call cannot replace the pin
    assert hygiene.properties({}, {"py-io-impl": "evil.module.FileIO"})["py-io-impl"] == (
        "pyiceberg.io.pyarrow.PyArrowFileIO"
    )
    # what the connection declares wins over what the catalog proposes
    assert properties["s3.endpoint"] == "https://s3.eu-west-1.amazonaws.com"
    assert (
        Hygiene(Declaration(StorageType.ADLS, ("abfss://c@acct.dfs.core.windows.net/w/",))).properties({}, {})[
            "py-io-impl"
        ]
        == "pyiceberg.io.fsspec.FsspecFileIO"
    )


# -- locations ---------------------------------------------------------------------------------------------------------


@pytest.mark.parametrize("name", ["metadata-service", "platform-database"])
def test_a_table_the_catalog_places_at_the_metadata_service_or_the_platform_database_is_refused(name: str) -> None:
    with hostile.serve() as url:
        document = hostile.fetch(url, f"/v1/namespaces/hostile/tables/{name}")
    hygiene = s3(vended=True)

    with pytest.raises(HygieneError) as refused:
        hygiene.accept_table(document)

    assert refused.value.code == LOCATION_NOT_ALLOWED
    # no location, host or key in the message
    assert hostile.METADATA_SERVICE not in str(refused.value)
    assert hostile.PLATFORM_DATABASE not in str(refused.value)
    assert hygiene.denied[Denial.LOCATION_NOT_ALLOWED] == 1


def test_a_table_inside_the_warehouse_is_accepted_with_its_filtered_config() -> None:
    with hostile.serve() as url:
        document = hostile.fetch(url, "/v1/namespaces/hostile/tables/honest")
    hygiene = s3(vended=True)

    accepted = hygiene.accept_table(document)

    assert accepted.config == hostile.vended_credentials()
    assert accepted.location == "s3://lake/warehouse/sales/orders"


def test_the_table_properties_that_move_the_data_files_are_checked_too() -> None:
    with hostile.serve() as url:
        document = hostile.fetch(url, "/v1/namespaces/hostile/tables/redirected-data")
    hygiene = s3(vended=True)

    with pytest.raises(HygieneError) as refused:
        hygiene.accept_table(document)

    assert refused.value.code == LOCATION_NOT_ALLOWED
    assert "attacker-bucket" not in str(refused.value)


def test_the_file_io_keys_a_table_carries_in_its_own_properties_are_dropped_and_counted() -> None:
    # PyIceberg builds the file IO of a table from the catalog's properties overridden by the table's own, so a table
    # could name the class to import (`py-io-impl`) or move the endpoint, the proxy, the signer and the region
    with hostile.serve() as url:
        document = hostile.fetch(url, "/v1/namespaces/hostile/tables/io-in-properties")
    hygiene = s3(vended=True)

    accepted = hygiene.accept_table(document)

    assert accepted.properties == hostile.HONEST_PROPERTIES
    assert hygiene.denied[Denial.KEY_NOT_ALLOWED] == len(hostile.HOSTILE_KEYS)
    assert hygiene.denials == len(hostile.HOSTILE_KEYS)


def test_a_table_with_no_properties_keeps_none() -> None:
    document = hostile.table_response("s3://lake/warehouse/sales/orders", config={}, properties={})
    hygiene = s3()
    assert hygiene.accept_table(document).properties == {}
    assert hygiene.denials == 0


@pytest.mark.parametrize(
    "key",
    [
        "write",  # a family without a key
        "write.",  # an empty key
        "writer.format",  # a longer family name
        "x.write.format.default",  # the family is not at the start
        "Write.format.default",  # case is not folded: PyIceberg reads the exact key
        "py-io-impl",
        "s3.endpoint",
        "client.region",
        "uri",
        "warehouse",
        "token",
        "credential",
        "rest.signing-name",
        "write.format.default\n",  # nothing after the key
    ],
)
def test_only_the_families_of_properties_the_host_needs_are_kept(key: str) -> None:
    document = hostile.table_response("s3://lake/warehouse/sales/orders", config={}, properties={key: "x"})
    hygiene = s3()
    assert hygiene.accept_table(document).properties == {}
    assert hygiene.denied[Denial.KEY_NOT_ALLOWED] == 1


@pytest.mark.parametrize("family", ["write", "commit", "gc", "history", "read", "schema"])
def test_every_family_the_host_needs_is_kept(family: str) -> None:
    properties = {f"{family}.a-b_c": "x"}
    document = hostile.table_response("s3://lake/warehouse/sales/orders", config={}, properties=properties)
    assert s3().accept_table(document).properties == properties


def test_a_property_that_is_not_text_is_dropped_and_counted() -> None:
    document = hostile.table_response("s3://lake/warehouse/sales/orders", config={})
    as_dict(document["metadata"])["properties"] = {"write.format.default": 5, "commit.retry.min-wait-ms": None}
    hygiene = s3()
    assert hygiene.accept_table(document).properties == {}
    assert hygiene.denied[Denial.KEY_NOT_ALLOWED] == 2


def test_the_location_provider_a_table_names_is_kept_so_the_compatibility_check_can_refuse_it() -> None:
    # dropping it would hand the table a default placement without anyone having said no; D035-092 refuses it
    properties = {"write.py-location-provider.impl": "evil.module.Provider"}
    document = hostile.table_response("s3://lake/warehouse/sales/orders", config={}, properties=properties)
    assert s3().accept_table(document).properties == properties


@pytest.mark.parametrize(
    "field",
    ["snapshots", "metadata-log", "statistics", "partition-statistics"],
)
def test_the_files_a_table_names_besides_its_location_are_checked_too(field: str) -> None:
    key = {
        "snapshots": "manifest-list",
        "metadata-log": "metadata-file",
        "statistics": "statistics-path",
        "partition-statistics": "statistics-path",
    }[field]
    inside = hostile.table_response("s3://lake/warehouse/sales/orders")
    metadata = as_dict(inside["metadata"])
    metadata[field] = [{key: "s3://lake/warehouse/sales/orders/metadata/ok"}]
    assert s3().accept_table(inside).location == "s3://lake/warehouse/sales/orders"

    metadata[field] = [{key: "s3://lake/warehouse/sales/orders/metadata/ok"}, {key: "s3://attacker-bucket/x"}]
    hygiene = s3()
    with pytest.raises(HygieneError) as refused:
        hygiene.accept_table(inside)

    assert refused.value.code == LOCATION_NOT_ALLOWED
    assert "attacker-bucket" not in str(refused.value)
    assert hygiene.denied[Denial.LOCATION_NOT_ALLOWED] == 1


@pytest.mark.parametrize(
    "document",
    [
        {},
        {"metadata": "not a document"},
        {"metadata": {}},
        {"metadata": {"location": 5}},
        {"metadata": {"location": "s3://lake/warehouse/t", "properties": "not a map"}},
        {"metadata": {"location": "s3://lake/warehouse/t", "properties": {"write.data.path": 5}}},
        {"metadata": {"location": "s3://lake/warehouse/t", "snapshots": "not a list"}},
        {"metadata": {"location": "s3://lake/warehouse/t", "snapshots": ["not a map"]}},
        {"metadata": {"location": "s3://lake/warehouse/t", "snapshots": [{"manifest-list": 5}]}},
    ],
)
def test_a_table_response_that_does_not_show_where_its_files_are_is_refused(document: dict[str, object]) -> None:
    hygiene = s3()

    with pytest.raises(HygieneError) as refused:
        hygiene.accept_table(document)

    assert refused.value.code == LOCATION_NOT_ALLOWED
    assert hygiene.denied[Denial.LOCATION_NOT_ALLOWED] == 1


@pytest.mark.parametrize(
    "location",
    [
        "s3://lake/warehouse/db/t",
        "s3://lake/warehouse",
        "s3://lake/warehouse/",
        "s3://lake/warehouse/a/b/c/data/00000.parquet",
    ],
)
def test_a_location_under_a_declared_prefix_is_allowed(location: str) -> None:
    hygiene = s3()

    assert hygiene.check_location(location) == location
    assert hygiene.denials == 0


@pytest.mark.parametrize(
    "location",
    [
        "s3://lake/warehouse-evil/t",  # a longer name that starts with the prefix
        "s3://lake/other/t",
        "s3://lake/",
        "s3://lake",
        "s3://lake/warehouse/../other/t",
        "s3://lake/warehouse/./../other/t",
        "s3://lake/warehouse/%2e%2e/other",
        "s3://lake/warehouse/%2E%2E/other",
        "s3://lake/warehouse/a%2f..%2fb",
        "s3://lake/warehouse/a\\..\\b",
        "s3://user:secret@lake/warehouse/t",
        "s3://lake/warehouse/t?x=1",
        "s3://lake/warehouse/t#frag",
        "s3://lake/warehouse/t\n",
        "s3://lake/warehouse/t t",
        "S3://lake/warehouse/t",
        "s3a://lake/warehouse/t",
        "http://169.254.169.254/latest/meta-data/",
        "https://lake.s3.amazonaws.com/warehouse/t",
        "postgresql://sagat-platform-db:5432/sagat",
        "file:///etc/passwd",
        "",
    ],
)
def test_a_location_outside_the_declared_prefixes_or_written_to_slip_past_the_check_is_refused(location: str) -> None:
    hygiene = s3()

    with pytest.raises(HygieneError) as refused:
        hygiene.check_location(location)

    assert refused.value.code == LOCATION_NOT_ALLOWED
    assert hygiene.denials == 1


def test_a_prefix_declared_without_a_closing_slash_still_ends_at_a_path_boundary() -> None:
    hygiene = s3(locations=("s3://lake/warehouse",))

    assert hygiene.check_location("s3://lake/warehouse/t") == "s3://lake/warehouse/t"
    for location in ("s3://lake/warehouse-evil/t", "s3://lake/warehouse2", "s3://lake/warehous"):
        with pytest.raises(HygieneError):
            hygiene.check_location(location)
    assert hygiene.denials == 3


def test_every_location_of_a_batch_is_checked_and_the_first_bad_one_stops_it() -> None:
    hygiene = s3()

    assert hygiene.check_locations(["s3://lake/warehouse/a", "s3://lake/warehouse/b"]) == [
        "s3://lake/warehouse/a",
        "s3://lake/warehouse/b",
    ]
    with pytest.raises(HygieneError):
        hygiene.check_locations(["s3://lake/warehouse/a", "s3://elsewhere/b", "s3://lake/warehouse/c"])
    assert hygiene.denials == 1


# -- the declaration ---------------------------------------------------------------------------------------------------


@pytest.mark.parametrize(
    ("storage", "prefix"),
    [
        (StorageType.S3, "gs://bucket/w/"),  # the scheme of another storage type
        (StorageType.S3, "s3://"),
        (StorageType.S3, "s3://user@lake/w/"),
        (StorageType.S3, "s3://lake/w/?x=1"),
        (StorageType.S3, "s3://lake/../w/"),
        (StorageType.GCS, "s3://lake/w/"),
        (StorageType.ADLS, "s3://lake/w/"),
        (StorageType.S3, ""),
    ],
)
def test_a_declaration_with_a_prefix_that_does_not_fit_its_storage_type_is_refused(
    storage: StorageType, prefix: str
) -> None:
    with pytest.raises(ValueError, match="warehouse location"):
        Declaration(storage, (prefix,))


@pytest.mark.parametrize(
    "prefix",
    [
        "abfss://acct.dfs.core.windows.net/w/",  # no container
        "abfss://@acct.dfs.core.windows.net/w/",  # an empty container
        "abfss://c@/w/",  # no account
        "abfss://c:secret@acct.dfs.core.windows.net/w/",  # a password
        "abfss://c@acct.dfs.core.windows.net@evil.example/w/",  # a second user info
    ],
)
def test_an_adls_location_names_a_container_and_an_account_and_nothing_else(prefix: str) -> None:
    with pytest.raises(ValueError, match="warehouse location"):
        Declaration(StorageType.ADLS, (prefix,))


def test_a_declaration_holds_between_one_and_sixteen_locations() -> None:
    with pytest.raises(ValueError, match="warehouse location"):
        Declaration(StorageType.S3, ())
    with pytest.raises(ValueError, match="warehouse location"):
        Declaration(StorageType.S3, tuple(f"s3://b{i}/w/" for i in range(17)))
    assert len(Declaration(StorageType.S3, tuple(f"s3://b{i}/w/" for i in range(16))).warehouse_locations) == 16


# -- the environment and the credentials ------------------------------------------------------------------------------


def test_the_environment_of_the_host_keeps_nothing_that_could_reach_a_credential_or_a_proxy() -> None:
    hostile_environment = {
        "PATH": "/usr/bin",
        "LANG": "C.UTF-8",
        "AWS_ACCESS_KEY_ID": "ambient",
        "AWS_SECRET_ACCESS_KEY": "ambient",
        "AWS_SESSION_TOKEN": "ambient",
        "AWS_PROFILE": "default",
        "AWS_WEB_IDENTITY_TOKEN_FILE": "/var/run/token",
        "AWS_CONTAINER_CREDENTIALS_FULL_URI": "http://169.254.170.2/creds",
        "AWS_EC2_METADATA_SERVICE_ENDPOINT": "http://attacker.example",
        "AWS_EC2_METADATA_DISABLED": "false",
        "GOOGLE_APPLICATION_CREDENTIALS": "/keys/sa.json",
        "GCE_METADATA_HOST": "attacker.example",
        "AZURE_CLIENT_SECRET": "ambient",
        "AZURE_FEDERATED_TOKEN_FILE": "/var/run/azure",
        "HTTP_PROXY": "http://attacker.example:3128",
        "HTTPS_PROXY": "http://attacker.example:3128",
        "ALL_PROXY": "socks5://attacker.example",
        "NO_PROXY": "",
        "SSL_CERT_FILE": "/tmp/evil.pem",
        "REQUESTS_CA_BUNDLE": "/tmp/evil.pem",
        "PYTHONPATH": "/tmp/evil",
        "LD_PRELOAD": "/tmp/evil.so",
        "HOME": "/root",
    }

    from sagat_lake.config_hygiene import scrubbed_environment

    environment = scrubbed_environment(hostile_environment)

    assert environment["AWS_EC2_METADATA_DISABLED"] == "true"
    assert environment["PATH"] == "/usr/bin"
    assert environment["LANG"] == "C.UTF-8"
    assert environment["AWS_CONFIG_FILE"] == "/dev/null"
    assert environment["AWS_SHARED_CREDENTIALS_FILE"] == "/dev/null"
    allowed = {
        "PATH",
        "LANG",
        "AWS_EC2_METADATA_DISABLED",
        "AWS_CONFIG_FILE",
        "AWS_SHARED_CREDENTIALS_FILE",
    }
    assert set(environment) == allowed


@pytest.mark.parametrize(
    ("storage", "properties"),
    [
        (StorageType.S3, {}),
        (StorageType.S3, {"s3.access-key-id": "k"}),
        (StorageType.S3, {"s3.access-key-id": "k", "s3.secret-access-key": ""}),
        (StorageType.GCS, {}),
        (StorageType.GCS, {"gcs.oauth2.token": ""}),
        (StorageType.ADLS, {"adls.account-name": "acct"}),
        (StorageType.ADLS, {"adls.account-key": "k"}),
        (StorageType.ADLS, {"adls.tenant-id": "t", "adls.client-id": "c"}),
    ],
)
def test_credentials_that_are_missing_fail_closed(storage: StorageType, properties: dict[str, str]) -> None:
    hygiene = Hygiene(Declaration(storage, (_prefix(storage),)))

    with pytest.raises(HygieneError) as refused:
        hygiene.require_explicit_credentials(properties)

    assert refused.value.code == CREDENTIAL_UNAVAILABLE
    assert hygiene.denied[Denial.CREDENTIAL_MISSING] == 1


@pytest.mark.parametrize(
    ("storage", "properties"),
    [
        (StorageType.S3, {"s3.access-key-id": "k", "s3.secret-access-key": "s"}),
        (StorageType.GCS, {"gcs.oauth2.token": "t"}),
        (StorageType.ADLS, {"adls.account-name": "acct", "adls.account-key": "k"}),
        (StorageType.ADLS, {"adls.account-name": "acct", "adls.sas-token": "s"}),
        (
            StorageType.ADLS,
            {"adls.account-name": "acct", "adls.tenant-id": "t", "adls.client-id": "c", "adls.client-secret": "s"},
        ),
        (StorageType.ADLS, {"adls.connection-string": "DefaultEndpointsProtocol=https"}),
    ],
)
def test_explicit_credentials_are_accepted(storage: StorageType, properties: dict[str, str]) -> None:
    hygiene = Hygiene(Declaration(storage, (_prefix(storage),)))

    hygiene.require_explicit_credentials(properties)

    assert hygiene.denials == 0


def _prefix(storage: StorageType) -> str:
    return {
        StorageType.S3: "s3://lake/w/",
        StorageType.GCS: "gs://lake/w/",
        StorageType.ADLS: "abfss://c@acct.dfs.core.windows.net/w/",
    }[storage]


# -- storage credentials of a table --------------------------------------------------------------------------------
def entry(prefix: object, config: object) -> dict[str, object]:
    return {"prefix": prefix, "config": config}


VENDED = {"s3.access-key-id": "k", "s3.secret-access-key": "s", "s3.session-token": "t"}


def test_the_storage_credentials_of_the_warehouse_keep_only_the_keys_of_the_declared_storage() -> None:
    hygiene = s3(vended=True)
    kept = hygiene.storage_credentials(
        [entry("s3://lake/warehouse/sales", {**VENDED, "s3.endpoint": "http://attacker", "py-io-impl": "evil.IO"})]
    )
    assert kept == [entry("s3://lake/warehouse/sales", VENDED)]
    assert hygiene.denied[Denial.KEY_NOT_ALLOWED] == 2


def test_a_credential_for_a_prefix_outside_the_warehouse_is_dropped_and_counted() -> None:
    hygiene = s3(vended=True)
    kept = hygiene.storage_credentials(
        [
            entry("s3://attacker-bucket/", VENDED),
            entry("s3://lake/warehouse-evil/", VENDED),
            entry("s3://lake/warehouse/../secret/", VENDED),
            entry("s3://lake/warehouse/ok/", VENDED),
        ]
    )
    assert kept == [entry("s3://lake/warehouse/ok/", VENDED)]
    assert hygiene.denied[Denial.LOCATION_NOT_ALLOWED] == 3


def test_the_warehouse_itself_and_a_prefix_without_a_trailing_slash_are_inside() -> None:
    hygiene = s3(vended=True)
    kept = hygiene.storage_credentials([entry("s3://lake/warehouse/", VENDED), entry("s3://lake/warehouse", VENDED)])
    assert len(kept) == 2


def test_without_vended_credentials_declared_no_credential_is_kept() -> None:
    hygiene = s3(vended=False)
    assert hygiene.storage_credentials([entry("s3://lake/warehouse/", VENDED)]) == []
    assert hygiene.denied[Denial.KEY_NOT_ALLOWED] == 3


@pytest.mark.parametrize(
    "bad",
    [
        entry(5, VENDED),
        entry(None, VENDED),
        entry("s3://lake/warehouse/", "not a map"),
        entry("s3://lake/warehouse/", None),
        {"config": VENDED},
        {"prefix": "s3://lake/warehouse/"},
        {},
    ],
)
def test_an_entry_that_is_not_a_prefix_with_a_config_is_dropped_and_counted(bad: dict[str, object]) -> None:
    hygiene = s3(vended=True)
    assert hygiene.storage_credentials([bad]) == []
    assert hygiene.denials == 1


def test_no_credentials_is_no_credentials() -> None:
    assert s3(vended=True).storage_credentials([]) == []
