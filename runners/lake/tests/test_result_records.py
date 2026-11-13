"""The records of a lake write that takes a SQL result, against the host that reads them.

Feature 035: D035-145, FR-035-010. The server turns each cell of the typed result a SQL step retains into a value of a
record, and the host (``mapping.py``) stores that value or refuses it.
``tests/contract/iceberg-writes/fixtures/result-records/cells.json`` is the one list of cells both sides are held to:
the Rust test of the lease (``lake_result_records.rs``) holds the value each cell becomes, and this test holds what the
host does with that value for each type of column. A change to one side that the other does not follow fails one of
them.
"""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from pyiceberg.schema import Schema
from pyiceberg.types import (
    BinaryType,
    BooleanType,
    DateType,
    DecimalType,
    DoubleType,
    FixedType,
    FloatType,
    IcebergType,
    IntegerType,
    LongType,
    NestedField,
    StringType,
    TimestampType,
    TimestamptzType,
    UUIDType,
)

from sagat_lake.failures import Failure
from sagat_lake.mapping import map_record

FIXTURE = Path(__file__).resolve().parents[3] / "tests/contract/iceberg-writes/fixtures/result-records/cells.json"

COLUMNS: dict[str, tuple[IcebergType, bool]] = {
    "long": (LongType(), False),
    "int": (IntegerType(), False),
    "double": (DoubleType(), False),
    "float": (FloatType(), False),
    "string": (StringType(), False),
    "required string": (StringType(), True),
    "boolean": (BooleanType(), False),
    "date": (DateType(), False),
    "timestamp": (TimestampType(), False),
    "timestamptz": (TimestamptzType(), False),
    "binary": (BinaryType(), False),
    "uuid": (UUIDType(), False),
    "decimal(12,0)": (DecimalType(12, 0), False),
    "decimal(12,1)": (DecimalType(12, 1), False),
    "decimal(12,2)": (DecimalType(12, 2), False),
    "fixed(3)": (FixedType(3), False),
    "fixed(4)": (FixedType(4), False),
}


def cases() -> list[tuple[str, object, str, str, object]]:
    document = json.loads(FIXTURE.read_text(encoding="utf-8"))
    rows = []
    for cell in document["cells"]:
        for host in cell["host"]:
            rows.append(
                (
                    f"{cell['name']} -> {host['column']}",
                    cell["record"],
                    host["column"],
                    host["outcome"],
                    host.get("stored", "<not stated>"),
                )
            )
    return rows


def test_every_column_the_fixture_names_is_one_this_test_knows() -> None:
    document = json.loads(FIXTURE.read_text(encoding="utf-8"))
    named = {host["column"] for cell in document["cells"] for host in cell["host"]}
    assert named <= set(COLUMNS), sorted(named - set(COLUMNS))


@pytest.mark.parametrize(("label", "value", "column", "outcome", "stored"), cases(), ids=[case[0] for case in cases()])
def test_the_host_stores_the_value_of_a_cell_or_refuses_it_for_the_reason_the_fixture_says(
    label: str, value: object, column: str, outcome: str, stored: object
) -> None:
    kind, required = COLUMNS[column]
    schema = Schema(NestedField(field_id=1, name="x", field_type=kind, required=required))
    if outcome == "ok":
        row, dropped = map_record(schema, {"x": value})
        assert dropped == 0, label
        if stored != "<not stated>":
            assert row["x"] == stored, label
    else:
        with pytest.raises(Failure) as refused:
            map_record(schema, {"x": value})
        assert refused.value.code == "ICEBERG_SCHEMA_MISMATCH", label
        assert refused.value.reason == outcome, label
        assert refused.value.column == "x", label
