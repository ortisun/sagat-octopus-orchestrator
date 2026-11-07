"""The mapped rows of an attempt as Arrow batches, and through an append (feature 035: T035-057, D035-033, D035-098).

``mapping.map_record`` gives rows of Python values by column. PyIceberg writes Arrow. Between them a uuid has to
become the 16 bytes Arrow stores it as (Arrow cannot build its uuid extension type from Python values), and the batches
have to be bounded in rows and in bytes so that a pathological row does not become a pathological batch. These tests go
through a real table: a row of every Iceberg type is mapped, batched, appended and scanned back, and what comes back is
what went in.
"""

from __future__ import annotations

import math
import pathlib
import uuid
from collections.abc import Iterable, Iterator
from decimal import Decimal
from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]
import pytest
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.partitioning import PartitionField, PartitionSpec
from pyiceberg.schema import Schema
from pyiceberg.table import Table
from pyiceberg.transforms import IdentityTransform
from pyiceberg.types import (
    BinaryType,
    BooleanType,
    DateType,
    DecimalType,
    DoubleType,
    FixedType,
    FloatType,
    IntegerType,
    ListType,
    LongType,
    MapType,
    NestedField,
    StringType,
    StructType,
    TimestampType,
    TimestamptzType,
    TimeType,
    UUIDType,
)

from sagat_lake.batches import to_batches
from sagat_lake.failures import Failure
from sagat_lake.mapping import map_record

UUID_TEXT = "0190a5e2-7b3c-7d2e-9f10-aabbccddeeff"
OTHER_UUID_TEXT = "11111111-2222-7333-8444-555555555555"

EVERY_TYPE = Schema(
    NestedField(1, "id", LongType(), required=True),
    NestedField(2, "b", BooleanType(), required=False),
    NestedField(3, "i", IntegerType(), required=False),
    NestedField(4, "f", FloatType(), required=False),
    NestedField(5, "d", DoubleType(), required=False),
    NestedField(6, "dec", DecimalType(12, 2), required=False),
    NestedField(7, "date", DateType(), required=False),
    NestedField(8, "time", TimeType(), required=False),
    NestedField(9, "ts", TimestampType(), required=False),
    NestedField(10, "tstz", TimestamptzType(), required=False),
    NestedField(11, "s", StringType(), required=False),
    NestedField(12, "u", UUIDType(), required=False),
    NestedField(13, "bin", BinaryType(), required=False),
    NestedField(14, "fix", FixedType(3), required=False),
    NestedField(
        15,
        "st",
        StructType(
            NestedField(30, "x", IntegerType(), required=False),
            NestedField(31, "u", UUIDType(), required=False),
        ),
        required=False,
    ),
    NestedField(16, "li", ListType(element_id=32, element_type=UUIDType(), element_required=False), required=False),
    NestedField(
        17,
        "m",
        MapType(key_id=33, key_type=StringType(), value_id=34, value_type=UUIDType(), value_required=False),
        required=False,
    ),
    NestedField(
        18,
        "nested",
        ListType(
            element_id=35,
            element_type=StructType(NestedField(36, "u", UUIDType(), required=False)),
            element_required=False,
        ),
        required=False,
    ),
)

FULL = {
    "id": 1,
    "b": True,
    "i": 2147483647,
    "f": 1.5,
    "d": 0.1,
    "dec": Decimal("12.50"),
    "date": "2026-11-06",
    "time": "10:15:30.123456",
    "ts": "2026-11-06T10:15:30.123456",
    "tstz": "2026-11-06T12:15:30.123456+02:00",
    "s": "olá 😀",
    "u": UUID_TEXT,
    "bin": {"type": "binary", "value": "AQID"},
    "fix": {"type": "binary", "value": "AQID"},
    "st": {"x": 7, "u": UUID_TEXT},
    "li": [UUID_TEXT, None, OTHER_UUID_TEXT],
    "m": {"k": UUID_TEXT, "n": None},
    "nested": [{"u": UUID_TEXT}, {"u": None}, None],
}


@pytest.fixture
def catalog(tmp_path: pathlib.Path) -> SqlCatalog:
    catalog = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    catalog.create_namespace("sales")
    return catalog


def rec(**values: Any) -> dict[str, Any]:
    """A record of EVERY_TYPE. Its list of structs is empty unless given: a null there is refused (G035-046)."""
    return {"nested": [], **values}


def table_of(catalog: SqlCatalog, schema: Schema = EVERY_TYPE, name: str = "t", **kwargs: Any) -> Table:
    return catalog.create_table(f"sales.{name}", schema=schema, **kwargs)


def mapped(table: Table, records: Iterable[dict[str, Any]]) -> list[dict[str, Any]]:
    return [map_record(table.schema(), record)[0] for record in records]


def append(table: Table, rows: list[dict[str, Any]], **kwargs: Any) -> None:
    batches = list(to_batches(table.schema(), rows, meter=lambda: 0, **kwargs))
    table.append(pa.Table.from_batches(batches, schema=batches[0].schema))


def scanned(catalog: SqlCatalog, name: str = "t") -> list[dict[str, Any]]:
    return sorted(catalog.load_table(f"sales.{name}").scan().to_arrow().to_pylist(), key=lambda row: row["id"])


# ---------------------------------------------------------------------------------------------- what goes in comes out
def test_a_row_of_every_type_is_what_a_scan_gives_back(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    rows = mapped(table, [FULL])
    append(table, rows)
    (back,) = scanned(catalog)
    assert back == rows[0]
    assert back["u"] == uuid.UUID(UUID_TEXT)


def test_a_row_of_nothing_but_nulls_comes_back_as_nulls(catalog: SqlCatalog) -> None:
    schema = Schema(*[field for field in EVERY_TYPE.fields if field.name != "nested"])
    table = table_of(catalog, schema)
    rows = mapped(table, [{"id": 2}])
    append(table, rows)
    (back,) = scanned(catalog)
    assert back == {name: (2 if name == "id" else None) for name in back}


def test_a_null_struct_is_not_a_struct_of_nulls(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    rows = mapped(table, [rec(id=1, st=None), rec(id=2, st={}), rec(id=3, st={"x": None, "u": None})])
    append(table, rows)
    first, second, third = scanned(catalog)
    assert first["st"] is None
    assert second["st"] == {"x": None, "u": None}
    assert third["st"] == {"x": None, "u": None}


def test_the_edges_of_the_ranges_survive(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    records = [
        rec(
            id=9_223_372_036_854_775_807,
            i=-2_147_483_648,
            date="0001-01-01",
            ts="0001-01-01T00:00:00",
            tstz="9999-12-31T23:59:59.999999Z",
            dec=Decimal("9999999999.99"),
            d=5e-324,
            s="",
            bin={"type": "binary", "value": ""},
        ),
        rec(id=-9_223_372_036_854_775_808, date="9999-12-31", ts="9999-12-31T23:59:59.999999", dec=Decimal("-0.01")),
    ]
    rows = mapped(table, records)
    append(table, rows)
    assert scanned(catalog) == sorted(rows, key=lambda row: row["id"])


def test_not_a_number_and_the_infinities_come_back(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    rows = mapped(table, [rec(id=1, f=float("nan"), d=float("inf")), rec(id=2, f=float("-inf"), d=float("nan"))])
    append(table, rows)
    first, second = scanned(catalog)
    assert math.isnan(first["f"])
    assert first["d"] == float("inf")
    assert second["f"] == float("-inf")
    assert math.isnan(second["d"])


def test_a_map_with_uuid_keys_survives_too(catalog: SqlCatalog) -> None:
    schema = Schema(
        NestedField(1, "id", LongType(), required=True),
        NestedField(
            2,
            "m",
            MapType(key_id=3, key_type=UUIDType(), value_id=4, value_type=IntegerType(), value_required=False),
            required=False,
        ),
    )
    table = table_of(catalog, schema, "keyed")
    rows = mapped(table, [{"id": 1, "m": [{"key": UUID_TEXT, "value": 1}, {"key": OTHER_UUID_TEXT, "value": None}]}])
    append(table, rows)
    (back,) = scanned(catalog, "keyed")
    assert dict(back["m"]) == {uuid.UUID(UUID_TEXT): 1, uuid.UUID(OTHER_UUID_TEXT): None}


# ------------------------------------------------------------------------------------------------------ the batches
def ids(batch: pa.RecordBatch) -> list[int]:
    return batch.column("id").to_pylist()  # type: ignore[no-any-return]


def rows_with_ids(count: int) -> Iterator[dict[str, Any]]:
    """Rows as the mapping gives them: every column present, and null but for the id."""
    for number in range(count):
        yield {
            field.name: (number if field.name == "id" else [] if field.name == "nested" else None)
            for field in EVERY_TYPE.fields
        }


def test_the_batches_have_the_arrow_schema_of_the_table_with_its_field_ids(catalog: SqlCatalog) -> None:
    from pyiceberg.io.pyarrow import schema_to_pyarrow

    table = table_of(catalog)
    (batch,) = to_batches(table.schema(), rows_with_ids(1), meter=lambda: 0)
    assert batch.schema.equals(schema_to_pyarrow(table.schema()), check_metadata=True)


def test_rows_are_cut_into_batches_of_at_most_the_row_limit_in_order(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    batches = list(to_batches(table.schema(), rows_with_ids(25), meter=lambda: 0, max_rows=10))
    assert [len(batch) for batch in batches] == [10, 10, 5]
    assert [number for batch in batches for number in ids(batch)] == list(range(25))


def test_rows_are_cut_when_the_bytes_read_reach_the_byte_limit(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    read = {"bytes": 0}

    def rows() -> Iterator[dict[str, Any]]:
        for row in rows_with_ids(7):
            read["bytes"] += 10  # the reader has just spent ten bytes on this row
            yield row

    batches = list(to_batches(table.schema(), rows(), meter=lambda: read["bytes"], max_bytes=25, max_rows=100))
    # the third row takes the batch to 30 bytes, which is past 25: a batch never waits for a fourth
    assert [len(batch) for batch in batches] == [3, 3, 1]


def test_the_byte_limit_is_reached_when_a_batch_has_taken_exactly_that_many(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    read = {"bytes": 0}

    def rows() -> Iterator[dict[str, Any]]:
        for row in rows_with_ids(6):
            read["bytes"] += 10
            yield row

    batches = list(to_batches(table.schema(), rows(), meter=lambda: read["bytes"], max_bytes=30, max_rows=100))
    assert [len(batch) for batch in batches] == [3, 3]


def test_the_bytes_a_batch_has_taken_are_counted_from_where_the_reader_was_when_it_began(catalog: SqlCatalog) -> None:
    # the reader has already spent a million bytes on earlier files of the step
    table = table_of(catalog)
    read = {"bytes": 1_000_000}

    def rows() -> Iterator[dict[str, Any]]:
        for row in rows_with_ids(7):
            read["bytes"] += 10
            yield row

    batches = list(to_batches(table.schema(), rows(), meter=lambda: read["bytes"], max_bytes=25, max_rows=100))
    assert [len(batch) for batch in batches] == [3, 3, 1]


def test_a_single_row_past_the_byte_limit_is_a_batch_of_one(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    read = {"bytes": 0}

    def rows() -> Iterator[dict[str, Any]]:
        for row in rows_with_ids(3):
            read["bytes"] += 1_000
            yield row

    batches = list(to_batches(table.schema(), rows(), meter=lambda: read["bytes"], max_bytes=10, max_rows=100))
    assert [len(batch) for batch in batches] == [1, 1, 1]


def test_no_rows_is_no_batch(catalog: SqlCatalog) -> None:
    assert list(to_batches(table_of(catalog).schema(), iter(()), meter=lambda: 0)) == []


def test_the_rows_are_taken_as_they_are_needed_and_not_all_at_once(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    taken = {"rows": 0}

    def rows() -> Iterator[dict[str, Any]]:
        for row in rows_with_ids(1_000):
            taken["rows"] += 1
            yield row

    batches = to_batches(table.schema(), rows(), meter=lambda: 0, max_rows=10)
    next(batches)
    assert taken["rows"] == 10


@pytest.mark.parametrize("bad", [0, -1])
def test_a_limit_below_one_is_refused(catalog: SqlCatalog, bad: int) -> None:
    schema = table_of(catalog).schema()
    with pytest.raises(ValueError, match="limit"):
        list(to_batches(schema, rows_with_ids(1), meter=lambda: 0, max_rows=bad))
    with pytest.raises(ValueError, match="limit"):
        list(to_batches(schema, rows_with_ids(1), meter=lambda: 0, max_bytes=bad))


# --------------------------------------------------------------------------- what PyIceberg 0.12 does with a stream
def test_a_stream_of_batches_is_appended_to_an_unpartitioned_table(catalog: SqlCatalog) -> None:
    table = table_of(catalog)
    rows = mapped(table, [rec(id=number) for number in range(25)])
    first = next(iter(to_batches(table.schema(), rows, meter=lambda: 0)))
    stream = to_batches(table.schema(), rows, meter=lambda: 0, max_rows=10)
    table.append(pa.RecordBatchReader.from_batches(first.schema, stream))
    assert [row["id"] for row in scanned(catalog)] == list(range(25))


def test_a_stream_of_batches_cannot_be_appended_to_a_partitioned_table(catalog: SqlCatalog) -> None:
    # PyIceberg 0.12 streams only into an unpartitioned table (G035-045): a partitioned one is written from a whole
    # Arrow table, so its rows are held in memory and its ceiling is lower
    spec = PartitionSpec(PartitionField(source_id=11, field_id=1000, transform=IdentityTransform(), name="s"))
    table = table_of(catalog, name="parted", partition_spec=spec)
    rows = mapped(table, [rec(id=1, s="a")])
    batches = list(to_batches(table.schema(), rows, meter=lambda: 0))
    reader = pa.RecordBatchReader.from_batches(batches[0].schema, iter(batches))
    with pytest.raises(NotImplementedError):
        table.append(reader)
    table.append(pa.Table.from_batches(batches, schema=batches[0].schema))
    assert len(scanned(catalog, "parted")) == 1


# ---------------------------------------------------------------------- the nulls PyIceberg 0.12 would lose (G035-046)
def lossy_schema() -> Schema:
    point = StructType(NestedField(10, "x", IntegerType(), required=False))
    return Schema(
        NestedField(1, "id", LongType(), required=True),
        NestedField(2, "points", ListType(element_id=3, element_type=point, element_required=False), required=False),
        NestedField(
            4,
            "by_name",
            MapType(key_id=5, key_type=StringType(), value_id=6, value_type=point, value_required=False),
            required=False,
        ),
        NestedField(
            7,
            "box",
            StructType(
                NestedField(
                    8,
                    "points",
                    ListType(element_id=9, element_type=point, element_required=False),
                    required=False,
                )
            ),
            required=False,
        ),
        NestedField(
            11,
            "grid",
            ListType(
                element_id=12,
                element_type=ListType(element_id=13, element_type=point, element_required=False),
                element_required=False,
            ),
            required=False,
        ),
        NestedField(
            14,
            "numbers",
            ListType(element_id=15, element_type=IntegerType(), element_required=False),
            required=False,
        ),
        NestedField(
            16,
            "index",
            MapType(
                key_id=17,
                key_type=StringType(),
                value_id=18,
                value_type=ListType(element_id=19, element_type=point, element_required=False),
                value_required=False,
            ),
            required=False,
        ),
    )


def test_pyiceberg_writes_a_null_list_of_structs_as_an_empty_list(catalog: SqlCatalog) -> None:
    # the premise of the refusal below: it is PyIceberg that loses the null, in the data file. When a newer PyIceberg
    # keeps it, this test fails and the refusal in batches.py can go.
    from pyiceberg.io.pyarrow import schema_to_pyarrow

    table = table_of(catalog, lossy_schema(), "lossy")
    arrow = pa.Table.from_pylist([{"id": 1}], schema=schema_to_pyarrow(table.schema()))
    assert arrow.to_pylist()[0]["points"] is None
    table.append(arrow)
    (back,) = scanned(catalog, "lossy")
    assert back["points"] == []
    assert back["by_name"] == []
    assert back["numbers"] is None


@pytest.mark.parametrize(
    ("record", "column"),
    [
        ({"id": 1}, "points"),  # absent: an optional column is null
        ({"id": 1, "points": None}, "points"),
        ({"id": 1, "points": []}, "by_name"),  # the map is the one that is null
        ({"id": 1, "points": [], "by_name": None}, "by_name"),
        ({"id": 1, "points": [], "by_name": {}, "box": {}}, "box.points"),
        ({"id": 1, "points": [], "by_name": {}, "box": {"points": None}}, "box.points"),
        ({"id": 1, "points": [], "by_name": {}, "box": None, "grid": [None]}, "grid[]"),
        ({"id": 1, "points": [], "by_name": {}, "box": {"points": []}, "grid": [], "index": {"k": None}}, "index{}"),
    ],
)
def test_a_null_that_pyiceberg_would_store_as_empty_is_refused_with_its_column(
    catalog: SqlCatalog, record: dict[str, Any], column: str
) -> None:
    table = table_of(catalog, lossy_schema(), "lossy")
    rows = mapped(table, [record])
    with pytest.raises(Failure) as refused:
        list(to_batches(table.schema(), rows, meter=lambda: 0))
    failure = refused.value
    assert (failure.code, failure.reason, failure.column) == ("ICEBERG_SCHEMA_MISMATCH", "TYPE", column)


def test_a_list_of_structs_that_is_empty_or_has_nulls_inside_is_written_as_it_is(catalog: SqlCatalog) -> None:
    table = table_of(catalog, lossy_schema(), "lossy")
    filled: dict[str, Any] = {"points": [], "by_name": {}, "box": {"points": []}, "grid": []}
    records: list[dict[str, Any]] = [
        {
            "id": 1,
            "points": [],
            "by_name": {},
            "box": {"points": [{"x": 1}, None]},
            "grid": [[{"x": 2}], []],
            "numbers": None,
            "index": {"k": [{"x": 3}, None], "empty": []},
        },
        # a null map of lists of structs is kept: PyIceberg only loses the null of a map whose values are structs
        {"id": 2, **filled, "index": None},
    ]
    rows = mapped(table, records)
    append(table, rows)
    first, second = scanned(catalog, "lossy")
    assert first["points"] == []
    assert first["by_name"] == []
    assert first["box"] == {"points": [{"x": 1}, None]}
    assert first["grid"] == [[{"x": 2}], []]
    assert first["numbers"] is None
    assert dict(first["index"]) == {"k": [{"x": 3}, None], "empty": []}
    assert second["index"] is None
    assert second["numbers"] is None
