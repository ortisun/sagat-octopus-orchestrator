"""Writing to a table of every partition transform, with the packages of the runtime (feature 035: G035-040, D035-103).

`compat.py` accepts a transform only if the runtime can apply it. PyIceberg 0.12 applies every transform but
`identity` and `void` with the optional package `pyiceberg-core`, so the rule can only be as wide as the lock. These
tests do not read the rule: they write to a table partitioned by each transform, scan it back, and ask the rule about
the same table. A transform the rule accepts and the runtime cannot write is the failure they exist to catch.
"""

from __future__ import annotations

import datetime
import pathlib
from decimal import Decimal
from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]
import pytest
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.partitioning import PartitionField, PartitionSpec
from pyiceberg.schema import Schema
from pyiceberg.table import Table
from pyiceberg.transforms import (
    BucketTransform,
    DayTransform,
    HourTransform,
    IdentityTransform,
    MonthTransform,
    Transform,
    TruncateTransform,
    VoidTransform,
    YearTransform,
)
from pyiceberg.types import DecimalType, LongType, NestedField, StringType, TimestampType

from sagat_lake.compat import check
from sagat_lake.table_facts import facts_of

SCHEMA = Schema(
    NestedField(1, "id", LongType(), required=False),
    NestedField(2, "name", StringType(), required=False),
    NestedField(3, "at", TimestampType(), required=False),
    NestedField(4, "amount", DecimalType(12, 2), required=False),
)
ROWS = pa.table(
    {
        "id": pa.array([1, 2, 3, 4], pa.int64()),
        "name": ["alpha", "alphabet", "beta", None],
        "at": pa.array(
            [
                datetime.datetime(2026, 1, 15, 10, 30),
                datetime.datetime(2026, 1, 15, 11, 45),
                datetime.datetime(2026, 6, 1, 0, 0),
                datetime.datetime(2027, 12, 31, 23, 59),
            ],
            pa.timestamp("us"),
        ),
        "amount": pa.array([Decimal("1.00"), Decimal("2.50"), None, Decimal("9999999999.99")], pa.decimal128(12, 2)),
    }
)

# (source column id, source column name, transform)
CASES: list[tuple[str, int, Transform[Any, Any]]] = [
    ("identity", 2, IdentityTransform()),
    ("void", 2, VoidTransform()),
    ("year", 3, YearTransform()),
    ("month", 3, MonthTransform()),
    ("day", 3, DayTransform()),
    ("hour", 3, HourTransform()),
    ("bucket[16]", 1, BucketTransform(16)),
    ("bucket[7] on a string", 2, BucketTransform(7)),
    ("truncate[4]", 2, TruncateTransform(4)),
    ("truncate[10] on a long", 1, TruncateTransform(10)),
]


@pytest.fixture
def catalog(tmp_path: pathlib.Path) -> SqlCatalog:
    catalog = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    catalog.create_namespace("sales")
    return catalog


def partitioned(catalog: SqlCatalog, name: str, source: int, transform: Transform[Any, Any]) -> Table:
    spec = PartitionSpec(PartitionField(source_id=source, field_id=1000, transform=transform, name="part"))
    return catalog.create_table(f"sales.{name}", schema=SCHEMA, partition_spec=spec)


@pytest.mark.parametrize(("label", "source", "transform"), CASES, ids=[case[0] for case in CASES])
def test_the_runtime_writes_to_a_table_of_every_transform_the_rule_accepts(
    catalog: SqlCatalog, label: str, source: int, transform: Transform[Any, Any]
) -> None:
    table = partitioned(catalog, "t", source, transform)
    warehouse = table.metadata.location.rsplit("/sales/", 1)[0] + "/"

    table.append(ROWS)

    scanned = catalog.load_table("sales.t").scan().to_arrow().sort_by("id")
    assert scanned.column("id").to_pylist() == [1, 2, 3, 4], label
    assert scanned.column("name").to_pylist() == ["alpha", "alphabet", "beta", None], label
    # and the rule says yes to the very table that was just written
    check(
        facts_of(catalog.load_table("sales.t"), warehouse_locations=[warehouse]),
        ref="main",
        warehouse_locations=[warehouse],
        max_metadata_bytes=10**7,
    )


def test_a_transform_over_several_partitions_writes_one_file_per_partition(catalog: SqlCatalog) -> None:
    table = partitioned(catalog, "months", 3, MonthTransform())
    table.append(ROWS)
    # January 2026 (two rows), June 2026 and December 2027
    assert len(list(catalog.load_table("sales.months").scan().plan_files())) == 3
