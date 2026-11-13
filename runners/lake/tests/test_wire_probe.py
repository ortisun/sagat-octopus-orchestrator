"""The request line of a probe that the runner builds, read by the host (feature 035: D035-150, slice S6a).

``tests/contract/iceberg-writes/fixtures/requests/probe.json`` is the line the Rust builder writes
(``crates/infra-lake/tests/probe_wire.rs`` asserts that the builder's output *is* this file). Here the host reads the
same file with its own ``parse_probe`` and ``open_connection``, and the members of the lease
(``fixtures/probe-input/probe.json``, which the server writes byte for byte) are checked to be the ones the line
nests in ``probe``. A member spelled differently on any side fails a test on that side, and not in a container
where the only symptom would be ``LAKE_HOST_PROTOCOL``.
"""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import pytest

from sagat_lake.connection import open_connection
from sagat_lake.failures import Failure
from sagat_lake.request import parse_probe

FIXTURES = Path(__file__).resolve().parents[3] / "tests/contract/iceberg-writes/fixtures"
SECRETS = ("client-secret-canary", "storage-secret-canary", "AKIAEXAMPLE", "relay-token-canary")


def fixture(relative: str) -> dict[str, Any]:
    document: dict[str, Any] = json.loads((FIXTURES / relative).read_text(encoding="utf-8"))
    return document


def test_the_host_parses_the_line_the_runner_builds() -> None:
    request = parse_probe(fixture("requests/probe.json"))

    assert (request.name, request.revision) == ("lake-main", 3)
    assert request.check_timeout == pytest.approx(7.0)
    assert request.connection["warehouseLocations"] == ["s3://lake/warehouse/"]


def test_the_host_parses_the_line_of_a_development_runner_and_the_ordinary_line_is_not_one() -> None:
    assert parse_probe(fixture("requests/probe-development.json")).development is True
    assert parse_probe(fixture("requests/probe.json")).development is False
    line = fixture("requests/probe-development.json")
    assert {key: value for key, value in line.items() if key != "development"} == fixture("requests/probe.json")


def test_the_host_opens_the_connection_the_runner_builds() -> None:
    request = parse_probe(fixture("requests/probe.json"))

    opened = open_connection(request.connection)

    assert opened.locations == ("s3://lake/warehouse/",)
    assert opened.vended is False
    assert opened.properties["s3.access-key-id"] == "AKIAEXAMPLE"
    assert opened.properties["credential"] == "client-1:client-secret-canary"


def test_the_members_of_the_lease_are_the_ones_the_line_nests() -> None:
    lease = fixture("probe-input/probe.json")
    nested = fixture("requests/probe.json")["probe"]

    assert nested == {key: lease[key] for key in ("connection", "checks", "checkTimeoutMs")}


def test_the_line_has_the_members_of_a_probe_and_no_member_of_a_write() -> None:
    line = fixture("requests/probe.json")

    assert sorted(line) == ["connection", "operation", "probe", "protocol", "relay"]
    for member in ("effectKey", "firstAttemptStartedAt", "table", "inputs", "limits"):
        assert member not in line


@pytest.mark.parametrize("member", ["effectKey", "firstAttemptStartedAt"])
def test_a_probe_line_with_a_member_of_a_write_is_a_break_of_the_protocol(member: str) -> None:
    line = fixture("requests/probe.json")
    line[member] = "run-7f3a/probe/1"

    with pytest.raises(Failure) as raised:
        parse_probe(line)

    assert raised.value.code == "LAKE_HOST_PROTOCOL"


def test_the_repr_of_what_the_host_read_shows_none_of_the_secrets() -> None:
    request = parse_probe(fixture("requests/probe.json"))
    shown = repr(request) + repr(open_connection(request.connection))
    for secret in SECRETS:
        assert secret not in shown
