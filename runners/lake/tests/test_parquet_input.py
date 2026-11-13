"""Reading the records of an attempt from a Parquet file the runner wrote (T035-057, D035-063, D035-107).

A Parquet blob is a table of records: each row is one. The host turns each cell into the JSON representation of the
mapping table (a date as `YYYY-MM-DD`, a timestamp with its offset, a binary as the typed cell with padded base64, a
decimal with its digits, a map as an array of key and value pairs), so one mapping decides for NDJSON and for Parquet.

The file is not trusted. Its footer is bounded before it is parsed, the uncompressed size of each row group is spent
from the shared budget before the row group is decoded, a column named twice is refused (a row would silently lose
one), and what the mapping cannot hold is a type mismatch that names the column and never carries a value.
"""

from __future__ import annotations

import datetime
import uuid
from collections.abc import Iterator
from decimal import Decimal
from pathlib import Path
from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]
import pyarrow.parquet as pq  # type: ignore[import-untyped]
import pytest
from pyiceberg.schema import Schema
from pyiceberg.types import (
    BinaryType,
    BooleanType,
    DateType,
    DecimalType,
    DoubleType,
    FixedType,
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

from sagat_lake.failures import Failure
from sagat_lake.mapping import map_record
from sagat_lake.parquet_input import read_parquet
from sagat_lake.records import Budget

MiB = 1024 * 1024


def write(directory: Path, table: pa.Table, name: str = "blob-0.parquet", **options: Any) -> Path:
    path = directory / name
    pq.write_table(table, path, **options)
    return path


def read(path: Path, *, limit: int = 100 * MiB, **kwargs: Any) -> list[dict[str, Any]]:
    return list(read_parquet(path, Budget(limit), **kwargs))


def refused(path: Path, code: str, reason: str | None, column: str | None = None, **kwargs: Any) -> Failure:
    with pytest.raises(Failure) as caught:
        read(path, **kwargs)
    failure = caught.value
    assert (failure.code, failure.reason, failure.column) == (code, reason, column)
    assert str(failure) == code
    return failure


# ------------------------------------------------------------------------------------------------------ what is read
def test_the_rows_come_out_in_order_across_row_groups_and_batches(tmp_path: Path) -> None:
    table = pa.table({"id": pa.array(range(10), pa.int64())})
    path = write(tmp_path, table, row_group_size=3)
    assert [row["id"] for row in read(path, batch_rows=4)] == list(range(10))


def test_a_file_with_no_row_is_no_record_and_a_column_of_nothing_is_null(tmp_path: Path) -> None:
    assert read(write(tmp_path, pa.table({"id": pa.array([], pa.int64())}))) == []
    nothing = pa.table({"a": pa.nulls(2), "id": pa.array([1, 2], pa.int64())})
    assert read(write(tmp_path, nothing, "nothing.parquet")) == [{"a": None, "id": 1}, {"a": None, "id": 2}]


def test_scalars_keep_their_values(tmp_path: Path) -> None:
    table = pa.table(
        {
            "b": [True, None],
            "i": pa.array([2**62, -5], pa.int64()),
            "u": pa.array([2**63 + 5, 0], pa.uint64()),
            "f": pa.array([1.5, float("inf")], pa.float64()),
            "n": pa.array([float("nan"), 0.0], pa.float64()),
            "s": ["olá 😀", ""],
            "d": pa.array([Decimal("12.50"), Decimal("-0.01")], pa.decimal128(12, 2)),
        }
    )
    first, second = read(write(tmp_path, table))
    assert first["b"] is True
    assert second["b"] is None
    assert (first["i"], second["i"]) == (2**62, -5)
    assert first["u"] == 2**63 + 5
    assert (first["f"], second["f"]) == (1.5, float("inf"))
    assert str(first["n"]) == "nan"
    assert (first["s"], second["s"]) == ("olá 😀", "")
    assert str(first["d"]) == "12.50"
    assert str(second["d"]) == "-0.01"


def test_dates_times_and_timestamps_become_the_text_of_the_mapping_table(tmp_path: Path) -> None:
    table = pa.table(
        {
            "date": pa.array([datetime.date(2026, 11, 6)], pa.date32()),
            "date64": pa.array([datetime.date(1970, 1, 2)], pa.date64()),
            "time": pa.array([datetime.time(10, 15, 30, 123456)], pa.time64("us")),
            "ts": pa.array([datetime.datetime(2026, 11, 6, 10, 15, 30, 123456)], pa.timestamp("us")),
            "tz": pa.array(
                [datetime.datetime(2026, 11, 6, 10, 15, 30, tzinfo=datetime.UTC)], pa.timestamp("us", tz="UTC")
            ),
            "ms": pa.array([datetime.datetime(2026, 11, 6, 10, 15, 30, 5000)], pa.timestamp("ms")),
        }
    )
    (row,) = read(write(tmp_path, table))
    assert row["date"] == "2026-11-06"
    assert row["date64"] == "1970-01-02"
    assert row["time"] == "10:15:30.123456"
    assert row["ts"] == "2026-11-06T10:15:30.123456"
    assert row["tz"] == "2026-11-06T10:15:30+00:00"
    assert row["ms"] == "2026-11-06T10:15:30.005000"


def test_binary_is_the_typed_cell_with_padded_base64_whatever_its_form(tmp_path: Path) -> None:
    table = pa.table(
        {
            "bin": pa.array([b"\x01\x02\x03\x04"], pa.binary()),
            "large": pa.array([b"ab"], pa.large_binary()),
            "fixed": pa.array([b"abc"], pa.binary(3)),
            "empty": pa.array([b""], pa.binary()),
        }
    )
    (row,) = read(write(tmp_path, table))
    assert row["bin"] == {"type": "binary", "value": "AQIDBA=="}
    assert row["large"] == {"type": "binary", "value": "YWI="}
    assert row["fixed"] == {"type": "binary", "value": "YWJj"}
    assert row["empty"] == {"type": "binary", "value": ""}


def test_a_uuid_is_its_canonical_text(tmp_path: Path) -> None:
    value = uuid.UUID("0190a5e2-7b3c-7d2e-9f10-aabbccddeeff")
    table = pa.table({"u": pa.array([value.bytes], pa.binary(16)).cast(pa.uuid())})
    assert read(write(tmp_path, table)) == [{"u": str(value)}]


def test_lists_structs_and_maps_are_read_down_to_their_cells(tmp_path: Path) -> None:
    table = pa.table(
        {
            "li": pa.array([[1, None, 3]], pa.list_(pa.int32())),
            "large": pa.array([["a"]], pa.large_list(pa.string())),
            "st": pa.array([{"x": 7, "d": datetime.date(2026, 1, 2), "b": b"z"}]),
            "map": pa.array([[("k", Decimal("1.5")), ("l", None)]], pa.map_(pa.string(), pa.decimal128(5, 1))),
            "deep": pa.array([[{"p": [datetime.date(2026, 1, 3)]}]]),
        }
    )
    (row,) = read(write(tmp_path, table))
    assert row["li"] == [1, None, 3]
    assert row["large"] == ["a"]
    assert row["st"] == {"x": 7, "d": "2026-01-02", "b": {"type": "binary", "value": "eg=="}}
    assert row["map"] == [{"key": "k", "value": Decimal("1.5")}, {"key": "l", "value": None}]
    assert row["deep"] == [{"p": ["2026-01-03"]}]


def test_a_dictionary_encoded_column_is_read_as_its_values(tmp_path: Path) -> None:
    table = pa.table({"s": pa.array(["a", "b", "a"]).dictionary_encode()})
    assert [row["s"] for row in read(write(tmp_path, table))] == ["a", "b", "a"]


def test_compression_makes_no_difference(tmp_path: Path) -> None:
    table = pa.table({"id": pa.array(range(100), pa.int64()), "s": ["x" * 50] * 100})
    for codec in ("none", "snappy", "gzip", "zstd"):
        path = write(tmp_path, table, f"{codec}.parquet", compression=codec)
        assert len(read(path)) == 100


# ----------------------------------------------------------------------------------- one mapping for both formats
def test_a_row_of_every_type_maps_to_the_table_it_was_written_for(tmp_path: Path) -> None:
    value = uuid.UUID("0190a5e2-7b3c-7d2e-9f10-aabbccddeeff")
    table = pa.table(
        {
            "id": pa.array([1], pa.int64()),
            "b": [True],
            "d": pa.array([0.1], pa.float64()),
            "dec": pa.array([Decimal("12.50")], pa.decimal128(12, 2)),
            "date": pa.array([datetime.date(2026, 11, 6)], pa.date32()),
            "time": pa.array([datetime.time(10, 15, 30, 123456)], pa.time64("us")),
            "ts": pa.array([datetime.datetime(2026, 11, 6, 10, 15, 30, 123456)], pa.timestamp("us")),
            "tstz": pa.array(
                [
                    datetime.datetime(
                        2026, 11, 6, 12, 15, 30, 123456, tzinfo=datetime.timezone(datetime.timedelta(hours=2))
                    )
                ],
                pa.timestamp("us", tz="+02:00"),
            ),
            "s": ["olá"],
            "u": pa.array([value.bytes], pa.binary(16)).cast(pa.uuid()),
            "bin": pa.array([b"\x01\x02\x03"], pa.binary()),
            "fix": pa.array([b"abc"], pa.binary(3)),
            "st": pa.array([{"x": 7}]),
            "li": pa.array([[1, 2]], pa.list_(pa.int32())),
            "m": pa.array([[("k", 1)]], pa.map_(pa.string(), pa.int32())),
        }
    )
    schema = Schema(
        NestedField(1, "id", LongType(), required=True),
        NestedField(2, "b", BooleanType(), required=False),
        NestedField(3, "d", DoubleType(), required=False),
        NestedField(4, "dec", DecimalType(12, 2), required=False),
        NestedField(5, "date", DateType(), required=False),
        NestedField(6, "time", TimeType(), required=False),
        NestedField(7, "ts", TimestampType(), required=False),
        NestedField(8, "tstz", TimestamptzType(), required=False),
        NestedField(9, "s", StringType(), required=False),
        NestedField(10, "u", UUIDType(), required=False),
        NestedField(11, "bin", BinaryType(), required=False),
        NestedField(12, "fix", FixedType(3), required=False),
        NestedField(13, "st", StructType(NestedField(20, "x", IntegerType(), required=False)), required=False),
        NestedField(
            14, "li", ListType(element_id=21, element_type=IntegerType(), element_required=False), required=False
        ),
        NestedField(
            15,
            "m",
            MapType(key_id=22, key_type=StringType(), value_id=23, value_type=IntegerType(), value_required=False),
            required=False,
        ),
    )
    (record,) = read(write(tmp_path, table))
    row, dropped = map_record(schema, record)
    assert dropped == 0
    assert row["id"] == 1
    assert row["dec"] == Decimal("12.50")
    assert row["date"] == datetime.date(2026, 11, 6)
    assert row["tstz"] == datetime.datetime(2026, 11, 6, 10, 15, 30, 123456, tzinfo=datetime.UTC)
    assert row["u"] == value
    assert row["bin"] == b"\x01\x02\x03"
    assert row["fix"] == b"abc"
    assert row["st"] == {"x": 7}
    assert row["m"] == [("k", 1)]


# ------------------------------------------------------------------------------------------------ what is refused
def test_what_is_not_a_parquet_file_is_a_type_mismatch(tmp_path: Path) -> None:
    good = write(tmp_path, pa.table({"id": pa.array(range(100), pa.int64())})).read_bytes()
    cases = {
        "empty": b"",
        "text": b"id,name\n1,a\n",
        "json": b'{"id": 1}\n',
        "magic only": b"PAR1PAR1",
        "short": b"PAR1",
        "cut in the middle": good[: len(good) // 2],
        "cut before the end": good[:-1],
        "wrong end": good[:-4] + b"XXXX",
        "wrong start": b"XXXX" + good[4:],
        "footer longer than the file": good[:-8] + (10**9).to_bytes(4, "little") + b"PAR1",
        "footer of zero": good[:-8] + (0).to_bytes(4, "little") + b"PAR1",
    }
    for name, content in cases.items():
        path = tmp_path / f"{name.replace(' ', '-')}.parquet"
        path.write_bytes(content)
        with pytest.raises(Failure) as caught:
            read(path)
        assert (caught.value.code, caught.value.reason, caught.value.column) == (
            "ICEBERG_SCHEMA_MISMATCH",
            "TYPE",
            None,
        ), name


def test_a_footer_above_the_bound_is_refused_before_it_is_parsed(tmp_path: Path) -> None:
    path = write(tmp_path, pa.table({"id": pa.array([1], pa.int64())}))
    footer = int.from_bytes(path.read_bytes()[-8:-4], "little")
    assert read(path, max_footer_bytes=footer) == [{"id": 1}]
    refused(path, "ICEBERG_LIMIT_EXCEEDED", None, max_footer_bytes=footer - 1)


def test_the_uncompressed_size_of_a_row_group_is_spent_before_it_is_decoded(tmp_path: Path) -> None:
    path = write(tmp_path, pa.table({"s": ["x" * 1000] * 100}), row_group_size=50)
    groups = pq.ParquetFile(path).metadata
    sizes = [groups.row_group(index).total_byte_size for index in range(groups.num_row_groups)]
    footer = int.from_bytes(path.read_bytes()[-8:-4], "little")
    assert len(sizes) == 2
    assert len(read(path, limit=footer + sum(sizes))) == 100
    # one byte less: the second row group is not decoded, and nothing of it reaches the caller
    stream = read_parquet(path, Budget(footer + sum(sizes) - 1))
    first = [next(stream) for _ in range(50)]
    assert len(first) == 50
    with pytest.raises(Failure) as caught:
        next(stream)
    assert caught.value.code == "ICEBERG_LIMIT_EXCEEDED"


def test_a_budget_smaller_than_the_first_row_group_yields_nothing(tmp_path: Path) -> None:
    path = write(tmp_path, pa.table({"s": ["x" * 1000] * 10}))
    stream = read_parquet(path, Budget(100))
    with pytest.raises(Failure) as caught:
        next(stream)
    assert caught.value.code == "ICEBERG_LIMIT_EXCEEDED"


def test_the_budget_is_shared_with_the_other_files_of_the_step(tmp_path: Path) -> None:
    path = write(tmp_path, pa.table({"id": pa.array([1], pa.int64())}))
    budget = Budget(10**6)
    assert len(list(read_parquet(path, budget))) == 1
    spent = budget.used
    assert spent > 0
    assert len(list(read_parquet(path, budget))) == 1
    assert budget.used == 2 * spent


def test_a_column_named_twice_is_refused_because_a_row_would_lose_one(tmp_path: Path) -> None:
    table = pa.Table.from_arrays([pa.array([1]), pa.array([2])], names=["a", "a"])
    refused(write(tmp_path, table), "ICEBERG_SCHEMA_MISMATCH", "TYPE")


@pytest.mark.parametrize(
    "column",
    [
        pa.array([datetime.timedelta(seconds=1)], pa.duration("s")),
    ],
)
def test_a_type_the_mapping_has_no_form_for_names_its_column(tmp_path: Path, column: pa.Array) -> None:
    path = tmp_path / "odd.parquet"
    pq.write_table(pa.table({"id": pa.array([1], pa.int64()), "odd": column}), path)
    failure = None
    try:
        read(path)
    except Failure as caught:
        failure = caught
    assert failure is not None
    assert (failure.code, failure.reason, failure.column) == ("ICEBERG_SCHEMA_MISMATCH", "TYPE", "odd")


def test_nanoseconds_that_would_be_lost_are_a_scale_overflow_and_those_that_would_not_are_read(tmp_path: Path) -> None:
    lossless = pa.table({"ts": pa.array([1_000_000_000], pa.timestamp("ns"))})
    assert read(write(tmp_path, lossless, "ok.parquet")) == [{"ts": "1970-01-01T00:00:01"}]
    lossy = pa.table({"id": pa.array([1], pa.int64()), "ts": pa.array([1_000_000_001], pa.timestamp("ns"))})
    refused(write(tmp_path, lossy, "lossy.parquet"), "ICEBERG_SCHEMA_MISMATCH", "SCALE_OVERFLOW", "ts")
    lossy_time = pa.table({"t": pa.array([1_000_000_001], pa.time64("ns"))})
    refused(write(tmp_path, lossy_time, "lossy-time.parquet"), "ICEBERG_SCHEMA_MISMATCH", "SCALE_OVERFLOW", "t")


def test_a_file_that_is_not_there_is_not_a_mismatch_of_the_data(tmp_path: Path) -> None:
    with pytest.raises(FileNotFoundError):
        read(tmp_path / "missing.parquet")


def test_a_failure_carries_no_value_of_the_file(tmp_path: Path) -> None:
    table = pa.table(
        {"id": pa.array([1], pa.int64()), "secret": pa.array([datetime.timedelta(seconds=424242)], pa.duration("s"))}
    )
    failure = refused(write(tmp_path, table), "ICEBERG_SCHEMA_MISMATCH", "TYPE", "secret")
    assert "424242" not in repr(failure.as_member())


def test_a_batch_size_below_one_is_refused() -> None:
    with pytest.raises(ValueError, match="batch"):
        list(read_parquet(Path("/nowhere"), Budget(1), batch_rows=0))


def test_rows_are_decoded_a_batch_at_a_time_and_not_all_at_once(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    path = write(tmp_path, pa.table({"id": pa.array(range(1000), pa.int64())}), row_group_size=100)
    seen: list[int] = []
    real = pq.ParquetFile.iter_batches

    def spy(self: Any, *args: Any, **kwargs: Any) -> Iterator[Any]:
        seen.append(kwargs.get("batch_size", -1))
        return iter(real(self, *args, **kwargs))

    monkeypatch.setattr(pq.ParquetFile, "iter_batches", spy)
    stream = read_parquet(path, Budget(10**9), batch_rows=10)
    next(stream)
    stream.close()
    assert seen == [10]
