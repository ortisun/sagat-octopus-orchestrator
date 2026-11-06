"""The compatibility of a table, checked before a file is written (feature 035: T035-049, FR-035-011, FR-035-012).

All of it is refused before any file: a format version the host cannot write, a default file format other than
Parquet, a partition transform it cannot apply, a location outside the warehouse, metadata above the bound, and a
missing ref.
"""

from __future__ import annotations

import dataclasses
from typing import Any

import pytest

from sagat_lake.compat import TableFacts, check
from sagat_lake.failures import Failure

WAREHOUSE = ["s3://lake/warehouse/"]
LOCATION = "s3://lake/warehouse/sales/orders"


def facts(**changes: Any) -> TableFacts:
    base = TableFacts(
        format_version=2,
        default_file_format="parquet",
        partition_transforms=("identity", "day", "bucket[16]", "truncate[4]", "year", "month", "hour"),
        locations=(LOCATION, LOCATION + "/data", LOCATION + "/metadata"),
        metadata_bytes=1024,
        refs=("main", "audit"),
    )
    return dataclasses.replace(base, **changes)


def verdict(
    table: TableFacts, *, ref: str = "main", warehouse: list[str] = WAREHOUSE, limit: int = 4096
) -> Failure | None:
    try:
        check(table, ref=ref, warehouse_locations=warehouse, max_metadata_bytes=limit)
    except Failure as failure:
        return failure
    return None


def refused(table: TableFacts, code: str, reason: str | None = None, **kwargs: object) -> None:
    failure = verdict(table, **kwargs)  # type: ignore[arg-type]
    assert failure is not None, "the table was accepted"
    assert (failure.code, failure.reason) == (code, reason)


# ------------------------------------------------------------------------------------------------------- compatible
def test_a_table_the_host_can_write_is_accepted() -> None:
    assert verdict(facts()) is None


def test_format_version_one_and_two_are_written() -> None:
    assert verdict(facts(format_version=1)) is None
    assert verdict(facts(format_version=2)) is None


def test_the_file_format_is_read_without_regard_to_case() -> None:
    assert verdict(facts(default_file_format="PARQUET")) is None


# ----------------------------------------------------------------------------------------------------- unsupported
@pytest.mark.parametrize("version", [3, 4, 0, -1])
def test_a_format_version_the_host_does_not_write_is_refused(version: int) -> None:
    refused(facts(format_version=version), "ICEBERG_UNSUPPORTED_TABLE", "FORMAT_VERSION")


@pytest.mark.parametrize("name", ["orc", "avro", "", "parquet2"])
def test_a_default_file_format_other_than_parquet_is_refused(name: str) -> None:
    refused(facts(default_file_format=name), "ICEBERG_UNSUPPORTED_TABLE", "FILE_FORMAT")


@pytest.mark.parametrize(
    "transform",
    [
        "void",
        "foo",
        "bucket",
        "bucket[0]",
        "bucket[x]",
        "truncate[]",
        "Identity",
        "",
        "day2",
        "yearly",
        "bucket[16]x",
        "identity(a)",
    ],
)
def test_a_partition_transform_the_host_cannot_apply_is_refused(transform: str) -> None:
    refused(facts(partition_transforms=("identity", transform)), "ICEBERG_UNSUPPORTED_TABLE", "TRANSFORM")


def test_an_unpartitioned_table_is_accepted() -> None:
    assert verdict(facts(partition_transforms=())) is None


# ----------------------------------------------------------------------------------------------------- the locations
@pytest.mark.parametrize(
    "location",
    [
        "s3://other/warehouse/sales/orders",  # another bucket
        "s3://lake/elsewhere/orders",  # another prefix
        "s3://lake/warehouse-evil/orders",  # a prefix that is not a path prefix
        "s3://lake/warehouse/../secret/orders",  # a way out
        "s3://lake/warehouse/sales/./../../x",
        "gs://lake/warehouse/sales/orders",  # another scheme
        "file:///etc/passwd",
        "/warehouse/sales/orders",
        "",
    ],
)
def test_a_location_outside_the_warehouse_is_refused(location: str) -> None:
    refused(facts(locations=(LOCATION, location)), "ICEBERG_LOCATION_NOT_ALLOWED")


def test_every_location_is_checked_not_only_the_table() -> None:
    refused(
        facts(locations=(LOCATION, "s3://elsewhere/data", LOCATION + "/metadata")),
        "ICEBERG_LOCATION_NOT_ALLOWED",
    )


def test_the_warehouse_root_itself_and_a_missing_trailing_slash_are_inside() -> None:
    assert verdict(facts(locations=("s3://lake/warehouse",))) is None
    assert verdict(facts(locations=("s3://lake/warehouse/",))) is None
    assert verdict(facts(locations=(LOCATION,)), warehouse=["s3://lake/warehouse"]) is None


def test_any_of_the_declared_locations_will_do() -> None:
    table = facts(locations=("s3://second/x/orders",))
    assert verdict(table, warehouse=["s3://lake/warehouse/", "s3://second/x/"]) is None


def test_no_declared_location_allows_nothing() -> None:
    refused(facts(), "ICEBERG_LOCATION_NOT_ALLOWED", warehouse=[])


@pytest.mark.parametrize("root", ["", "/", "//"])
def test_a_root_that_is_empty_allows_nothing_and_not_everything(root: str) -> None:
    refused(facts(locations=("/warehouse/sales/orders",)), "ICEBERG_LOCATION_NOT_ALLOWED", warehouse=[root])
    refused(facts(locations=(LOCATION,)), "ICEBERG_LOCATION_NOT_ALLOWED", warehouse=[root])


# ----------------------------------------------------------------------------------------------------------- limits
def test_metadata_above_the_bound_is_refused_and_at_the_bound_is_not() -> None:
    assert verdict(facts(metadata_bytes=4096), limit=4096) is None
    refused(facts(metadata_bytes=4097), "ICEBERG_LIMIT_EXCEEDED", limit=4096)


# ------------------------------------------------------------------------------------------------------- the target
def test_a_ref_the_table_does_not_have_is_a_table_not_found() -> None:
    refused(facts(), "ICEBERG_TABLE_NOT_FOUND", ref="release")


def test_the_target_ref_may_be_a_branch_that_exists() -> None:
    assert verdict(facts(), ref="audit") is None


# -------------------------------------------------------------------------------------------------------- the rest
def test_a_refusal_carries_nothing_but_its_code_and_reason() -> None:
    failure = verdict(facts(format_version=3))
    assert failure is not None
    assert failure.as_member() == {"code": "ICEBERG_UNSUPPORTED_TABLE", "reason": "FORMAT_VERSION"}


def test_the_checks_run_in_a_fixed_order_so_the_answer_is_always_the_same() -> None:
    # size first (nothing else is read when the metadata is too big), then the ref, the format, the transforms, the
    # locations
    table = facts(
        metadata_bytes=10_000,
        format_version=3,
        default_file_format="orc",
        partition_transforms=("void",),
        locations=("s3://elsewhere/x",),
    )
    refused(table, "ICEBERG_LIMIT_EXCEEDED", limit=4096, ref="release")
    refused(dataclasses.replace(table, metadata_bytes=1), "ICEBERG_TABLE_NOT_FOUND", ref="release")
    refused(dataclasses.replace(table, metadata_bytes=1), "ICEBERG_UNSUPPORTED_TABLE", "FORMAT_VERSION")
    refused(dataclasses.replace(table, metadata_bytes=1, format_version=2), "ICEBERG_UNSUPPORTED_TABLE", "FILE_FORMAT")
    refused(
        dataclasses.replace(table, metadata_bytes=1, format_version=2, default_file_format="parquet"),
        "ICEBERG_UNSUPPORTED_TABLE",
        "TRANSFORM",
    )
    refused(
        dataclasses.replace(
            table, metadata_bytes=1, format_version=2, default_file_format="parquet", partition_transforms=()
        ),
        "ICEBERG_LOCATION_NOT_ALLOWED",
    )
