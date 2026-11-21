"""The commit is made once (feature 035: T035-019 and T035-034, D035-030 step 5, FR-035-018, G035-020).

The host commits a step once and, when it does not know the outcome, searches the history of the table for the effect
key before it tries again. A commit that PyIceberg repeats on its own would defeat that search. These tests count the
commit requests a REST catalog receives, over real HTTP, for every answer the catalog can give, with and without the
host's override. The catalog is real behind the wire, so a conflict is a real conflict.

Some tests state what PyIceberg 0.12 does *without* the override. They are the reason the override exists, and they
fail when a newer PyIceberg changes it, which is when the override has to be looked at again.
"""

from __future__ import annotations

import pathlib
from collections.abc import Generator

import pyarrow as pa  # type: ignore[import-untyped]
import pytest
from pyiceberg.catalog.rest import RestCatalog
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.exceptions import (
    CommitFailedException,
    CommitStateUnknownException,
    ServiceUnavailableError,
    TooManyRequestsError,
    UnauthorizedError,
)
from pyiceberg.schema import Schema
from pyiceberg.table import Table, TableProperties
from pyiceberg.types import NestedField, StringType

import counting_catalog
from counting_catalog import RESET, Catalog
from sagat_lake.commit import COMMIT_RETRIES, pin_commit_once
from sagat_lake.lookup import EFFECT_KEY

ROWS = pa.table({"id": ["a", "b"]})
NAME = f"{counting_catalog.NAMESPACE}.{counting_catalog.TABLE}"
# short waits, so that the tests of the retries PyIceberg makes on its own do not sleep for seconds
QUICK = {"commit.retry.min-wait-ms": "1", "commit.retry.max-wait-ms": "2"}


@pytest.fixture(autouse=True)
def loopback(monkeypatch: pytest.MonkeyPatch) -> None:
    """The requests library reads the proxy variables of the environment: the loopback is reached directly."""
    for name in ("HTTP_PROXY", "http_proxy", "HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"):
        monkeypatch.delenv(name, raising=False)
    monkeypatch.setenv("NO_PROXY", "127.0.0.1")


@pytest.fixture
def sql(tmp_path: pathlib.Path) -> SqlCatalog:
    catalog = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    catalog.create_namespace(counting_catalog.NAMESPACE)
    catalog.create_table(NAME, schema=Schema(NestedField(1, "id", StringType(), required=False)))
    return catalog


@pytest.fixture
def rest(sql: SqlCatalog) -> Generator[Catalog]:
    with counting_catalog.serve(sql) as catalog:
        catalog.properties = dict(QUICK)
        yield catalog


def load(rest: Catalog, *, pin: bool) -> Table:
    table: Table = RestCatalog("r", uri=rest.uri).load_table(NAME)
    if pin:
        pin_commit_once(table)
    return table


def snapshots(sql: SqlCatalog) -> int:
    return len(sql.load_table(NAME).metadata.snapshots)


# ------------------------------------------------------------------------------------------ what PyIceberg does alone
def test_pyiceberg_repeats_a_refused_commit_four_times_on_its_own(rest: Catalog) -> None:
    rest.answer = 409
    with pytest.raises(CommitFailedException):
        load(rest, pin=False).append(ROWS)
    assert len(rest.commits) == 5


def test_the_table_chooses_how_many_times_pyiceberg_repeats_a_commit(rest: Catalog) -> None:
    # the count is read from the properties of the table the catalog returned: its owner decides, not the host
    rest.properties[COMMIT_RETRIES] = "6"
    rest.answer = 409
    with pytest.raises(CommitFailedException):
        load(rest, pin=False).append(ROWS)
    assert len(rest.commits) == 7


def test_a_commit_that_lost_a_race_is_rebased_and_landed_by_pyiceberg_on_its_own(
    rest: Catalog, sql: SqlCatalog
) -> None:
    # the hazard of D035-030: between the search and the commit another writer moved the ref, and PyIceberg refreshes
    # and commits the same rows again on top of it, without the host having searched the new history
    ours = load(rest, pin=False)
    sql.load_table(NAME).append(ROWS)
    ours.append(ROWS)
    assert snapshots(sql) == 2


# ------------------------------------------------------------------------------------------------ with the override
@pytest.mark.parametrize(
    ("answer", "raised"),
    [
        (409, CommitFailedException),
        (500, CommitStateUnknownException),
        (502, CommitStateUnknownException),
        (504, CommitStateUnknownException),
        (503, ServiceUnavailableError),
        (429, TooManyRequestsError),
    ],
)
def test_one_commit_request_is_sent_whatever_the_catalog_answers(rest: Catalog, answer: int, raised: type) -> None:
    rest.answer = answer
    with pytest.raises(raised):
        load(rest, pin=True).append(ROWS)
    assert len(rest.commits) == 1


def test_one_commit_request_is_sent_when_the_connection_is_closed_without_a_reply(rest: Catalog) -> None:
    rest.answer = RESET
    with pytest.raises(Exception, match="onnection"):
        load(rest, pin=True).append(ROWS)
    assert len(rest.commits) == 1


def test_a_table_that_asks_for_retries_gets_none(rest: Catalog) -> None:
    rest.properties[COMMIT_RETRIES] = "1000"
    rest.answer = 409
    with pytest.raises(CommitFailedException):
        load(rest, pin=True).append(ROWS)
    assert len(rest.commits) == 1


def test_a_commit_that_lost_a_race_is_refused_and_nothing_of_ours_lands(rest: Catalog, sql: SqlCatalog) -> None:
    ours = load(rest, pin=True)
    sql.load_table(NAME).append(ROWS)
    with pytest.raises(CommitFailedException):
        ours.append(ROWS)
    assert len(rest.commits) == 1
    assert snapshots(sql) == 1


def test_a_commit_that_is_not_contested_lands_once_with_its_effect_key(rest: Catalog, sql: SqlCatalog) -> None:
    load(rest, pin=True).append(ROWS, snapshot_properties={EFFECT_KEY: "run-1:save"})
    assert len(rest.commits) == 1
    (snapshot,) = sql.load_table(NAME).metadata.snapshots
    assert snapshot.summary is not None
    assert snapshot.summary.additional_properties[EFFECT_KEY] == "run-1:save"


def test_an_unauthorised_commit_is_asked_again_once_after_the_token_is_refreshed(rest: Catalog) -> None:
    # the one repeat the override cannot remove: PyIceberg's REST client asks again after a 401. The catalog applied
    # nothing, so it is safe, and it is at most one more request.
    rest.answer = 401
    with pytest.raises(UnauthorizedError):
        load(rest, pin=True).append(ROWS)
    assert len(rest.commits) == 2


# --------------------------------------------------------------------------------------------------- the override
def test_the_property_pinned_is_the_one_pyiceberg_reads() -> None:
    assert COMMIT_RETRIES == TableProperties.COMMIT_NUM_RETRIES


def test_the_override_is_in_memory_and_the_stored_table_never_has_it(rest: Catalog, sql: SqlCatalog) -> None:
    load(rest, pin=True).append(ROWS)
    assert COMMIT_RETRIES not in sql.load_table(NAME).metadata.properties


def test_the_override_wins_over_the_table_and_keeps_its_other_properties(rest: Catalog) -> None:
    rest.properties[COMMIT_RETRIES] = "9"
    rest.properties["owner"] = "sales"
    table = load(rest, pin=True)
    assert table.metadata.properties[COMMIT_RETRIES] == "0"
    assert table.metadata.properties["owner"] == "sales"
    assert table.metadata.properties["commit.retry.min-wait-ms"] == "1"
