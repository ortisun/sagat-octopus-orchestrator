"""The request line of a `probe`, checked field by field (feature 035: D035-150, slice S5).

The server leases the probe as `sagat.lake.probe-input.v1` (the connection by name and revision, the checks in the order
they run, the time one check may take) and the runner turns that into this line by adding the pinned connection with its
resolved secrets. Like the write's request, everything is checked for its type, its shape and its bounds, and a field
the host does not know, one that is missing and one that does not hold what the contract says are the same thing, a
break of the protocol with no field name and no value.
"""

from __future__ import annotations

import copy
import json
from pathlib import Path
from typing import Any

import pytest

from sagat_lake.failures import LAKE_HOST_PROTOCOL, Failure
from sagat_lake.request import parse_probe

LEASE = Path(__file__).resolve().parents[3] / "tests/contract/iceberg-writes/fixtures/probe-input/probe.json"

CONNECTION: dict[str, Any] = {
    "catalog": {
        "type": "rest",
        "rest": {"uri": "https://catalog.example.com", "warehouse": "w", "accessDelegation": "none"},
    },
    "storage": {"type": "s3", "s3": {"region": "eu-west-1", "pathStyle": False}, "credentials": "static"},
    "warehouseLocations": ["s3://lake/warehouse/"],
    "credentials": {"storage": {"accessKeyId": "AKIA", "secretAccessKey": "secret"}},
}


def request(**changes: Any) -> dict[str, Any]:
    """The request the runner would write for the leased probe of the corpus (the lease's members, nested)."""
    lease = json.loads(LEASE.read_text(encoding="utf-8"))
    document: dict[str, Any] = {
        "protocol": "sagat.lake.v1",
        "operation": "probe",
        "connection": copy.deepcopy(CONNECTION),
        "probe": {key: lease[key] for key in ("connection", "checks", "checkTimeoutMs")},
    }
    return {**document, **changes}


def with_probe(**changes: Any) -> dict[str, Any]:
    probe = request()["probe"]
    return request(probe={**probe, **changes})


def test_the_request_built_from_the_lease_of_the_corpus_is_read() -> None:
    parsed = parse_probe(request())

    assert (parsed.name, parsed.revision) == ("lake-main", 3)
    assert parsed.check_timeout == pytest.approx(7.0)
    assert parsed.connection["warehouseLocations"] == ["s3://lake/warehouse/"]


def test_the_relay_member_is_accepted_and_not_required() -> None:
    assert parse_probe(request(relay={"socket": "/work/a/relay.sock", "token": "t"})).name == "lake-main"
    assert "relay" not in request()


def test_the_connection_with_its_secrets_is_never_part_of_a_representation() -> None:
    parsed = parse_probe(request())
    assert "secret" not in repr(parsed)
    assert "AKIA" not in repr(parsed)


@pytest.mark.parametrize(
    "document",
    [
        None,
        [],
        "probe",
        {},
        {**request(), "extra": 1},
        {key: value for key, value in request().items() if key != "probe"},
        {key: value for key, value in request().items() if key != "connection"},
        request(protocol="sagat.lake.v2"),
        request(operation="icebergWrite"),
        request(operation="Probe"),
        request(probe=None),
        request(probe=[]),
        request(connection=None),
        request(connection={**CONNECTION, "extra": 1}),
        request(connection={**CONNECTION, "warehouseLocations": []}),
        request(connection={**CONNECTION, "credentials": {"storage": {"accessKeyId": 1}}}),
    ],
)
def test_a_request_that_is_not_a_probe_is_a_break_of_the_protocol(document: object) -> None:
    with pytest.raises(Failure) as raised:
        parse_probe(document)
    assert raised.value.code == LAKE_HOST_PROTOCOL
    assert raised.value.column is None
    assert raised.value.reason is None


@pytest.mark.parametrize(
    "changes",
    [
        {"extra": 1},
        {"connection": None},
        {"connection": {"name": "lake-main"}},
        {"connection": {"revision": 3}},
        {"connection": {"name": "lake-main", "revision": 3, "extra": 1}},
        # the name of a connection is a slug: 3 to 63 of a-z, 0-9 and '-', a letter first, no '-' at the end or twice
        {"connection": {"name": "ab", "revision": 3}},
        {"connection": {"name": "a" * 64, "revision": 3}},
        {"connection": {"name": "Lake-main", "revision": 3}},
        {"connection": {"name": "lake_main", "revision": 3}},
        {"connection": {"name": "1lake", "revision": 3}},
        {"connection": {"name": "lake-", "revision": 3}},
        {"connection": {"name": "lake--main", "revision": 3}},
        {"connection": {"name": "lake main", "revision": 3}},
        {"connection": {"name": "lake-main\n", "revision": 3}},
        {"connection": {"name": 7, "revision": 3}},
        # the revision is a whole number from 1 to 2**32 - 1
        {"connection": {"name": "lake-main", "revision": 0}},
        {"connection": {"name": "lake-main", "revision": -1}},
        {"connection": {"name": "lake-main", "revision": 2**32}},
        {"connection": {"name": "lake-main", "revision": 3.0}},
        {"connection": {"name": "lake-main", "revision": "3"}},
        {"connection": {"name": "lake-main", "revision": True}},
        # the three checks, once each, in the order they run: nothing else is served
        {"checks": []},
        {"checks": ["CATALOG_AUTH"]},
        {"checks": ["CATALOG_AUTH", "WAREHOUSE"]},
        {"checks": ["WAREHOUSE", "CATALOG_AUTH", "STORAGE_ACCESS"]},
        {"checks": ["CATALOG_AUTH", "WAREHOUSE", "STORAGE_ACCESS", "STORAGE_ACCESS"]},
        {"checks": ["CATALOG_AUTH", "WAREHOUSE", "CONDITIONAL_CREATE"]},
        {"checks": ["catalog_auth", "warehouse", "storage_access"]},
        {"checks": "CATALOG_AUTH,WAREHOUSE,STORAGE_ACCESS"},
        {"checks": None},
        # the time one check may take, in milliseconds: 100 to 60 000
        {"checkTimeoutMs": 99},
        {"checkTimeoutMs": 60_001},
        {"checkTimeoutMs": 0},
        {"checkTimeoutMs": -7000},
        {"checkTimeoutMs": 7000.0},
        {"checkTimeoutMs": "7000"},
        {"checkTimeoutMs": True},
        {"checkTimeoutMs": None},
    ],
)
def test_a_probe_member_that_does_not_hold_what_the_lease_says_is_a_break(changes: dict[str, Any]) -> None:
    probe = request()["probe"]
    document = request(probe={**probe, **changes})
    with pytest.raises(Failure) as raised:
        parse_probe(document)
    assert raised.value.code == LAKE_HOST_PROTOCOL


def test_a_member_of_the_probe_that_is_missing_is_a_break() -> None:
    for missing in ("connection", "checks", "checkTimeoutMs"):
        probe = {key: value for key, value in request()["probe"].items() if key != missing}
        with pytest.raises(Failure) as raised:
            parse_probe(request(probe=probe))
        assert raised.value.code == LAKE_HOST_PROTOCOL


@pytest.mark.parametrize("milliseconds", [100, 7000, 60_000])
def test_the_bounds_of_the_time_limit_are_served(milliseconds: int) -> None:
    assert parse_probe(with_probe(checkTimeoutMs=milliseconds)).check_timeout == pytest.approx(milliseconds / 1000)


@pytest.mark.parametrize("name", ["abc", "lake-main", "a" * 63, "a1-b2-c3"])
def test_a_slug_as_the_connection_name_is_served_and_echoed(name: str) -> None:
    parsed = parse_probe(with_probe(connection={"name": name, "revision": 1}))
    assert (parsed.name, parsed.revision) == (name, 1)


def test_the_largest_revision_is_served() -> None:
    assert parse_probe(with_probe(connection={"name": "lake-main", "revision": 2**32 - 1})).revision == 2**32 - 1
