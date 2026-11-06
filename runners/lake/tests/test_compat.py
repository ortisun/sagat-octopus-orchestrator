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
        partition_transforms=("identity", "void"),
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


# The runtime image has PyIceberg without `pyiceberg-core`, and PyIceberg 0.12 needs it to apply every transform but
# these two (checked by writing to a table of each: `year`, `month`, `day`, `hour`, `bucket` and `truncate` fail
# with NotInstalledError).
@pytest.mark.parametrize("transform", ["year", "month", "day", "hour", "bucket[16]", "truncate[4]"])
def test_a_transform_that_needs_pyiceberg_core_is_refused_until_the_runtime_has_it(transform: str) -> None:
    refused(facts(partition_transforms=("identity", transform)), "ICEBERG_UNSUPPORTED_TABLE", "TRANSFORM")


@pytest.mark.parametrize(
    "transform",
    ["foo", "bucket", "bucket[0]", "bucket[x]", "truncate[]", "Identity", "", "identity2", "voided", "identity(a)"],
)
def test_a_partition_transform_that_is_not_one_is_refused(transform: str) -> None:
    refused(facts(partition_transforms=("identity", transform)), "ICEBERG_UNSUPPORTED_TABLE", "TRANSFORM")


def test_identity_and_void_are_the_transforms_the_runtime_applies() -> None:
    assert verdict(facts(partition_transforms=("identity",))) is None
    assert verdict(facts(partition_transforms=("void",))) is None
    assert verdict(facts(partition_transforms=("identity", "identity", "void"))) is None


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
        # inside by text, but the object store reads another key: a fragment or a query is dropped, so every file of the
        # table would land on one key; a backslash, an encoded dot or separator and a dot segment are read as a path
        "s3://lake/warehouse/sales/data#",
        "s3://lake/warehouse/sales/data?x=1",
        "s3://lake/warehouse/sales\\..\\x",
        "s3://lake/warehouse/sales/%2e%2e/x",
        "s3://lake/warehouse/sales/%2E%2F/x",
        "s3://lake/warehouse/sales/./x",
        "s3://lake/warehouse/sales/a\x00b",
        "s3://lake/warehouse/sales/a b",
    ],
)
def test_a_location_outside_the_warehouse_is_refused(location: str) -> None:
    refused(facts(locations=(LOCATION, location)), "ICEBERG_LOCATION_NOT_ALLOWED")


def test_a_table_that_names_its_own_location_provider_is_refused() -> None:
    # PyIceberg imports the module the property names and calls the class with the table: code from the table's
    # metadata would decide where files go, and no check of a location could see it.
    refused(facts(location_provider="evil.Provider"), "ICEBERG_LOCATION_NOT_ALLOWED")


def test_a_table_with_no_location_provider_of_its_own_is_accepted() -> None:
    assert verdict(facts(location_provider=None)) is None


@pytest.mark.parametrize("root", ["s3://", "s3:///", "s3:", "://", "lake/warehouse"])
def test_a_root_that_is_no_more_than_a_scheme_allows_nothing(root: str) -> None:
    refused(facts(locations=("s3://anywhere/orders",)), "ICEBERG_LOCATION_NOT_ALLOWED", warehouse=[root])


def test_only_what_is_below_the_root_is_checked_because_the_root_is_the_connections_own() -> None:
    # the roots come from the declaration of the connection, which validated them; a location is judged by its path
    # below one, so a root the connection declared stays usable whatever characters it has
    assert verdict(facts(locations=("s3://lake/my warehouse/t",)), warehouse=["s3://lake/my warehouse/"]) is None


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
        partition_transforms=("day",),
        locations=("s3://elsewhere/x",),
        location_provider="evil.Provider",
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
    # the provider is a location problem too, and it is judged with the locations, after the rest
    only_the_provider = dataclasses.replace(
        table,
        metadata_bytes=1,
        format_version=2,
        default_file_format="parquet",
        partition_transforms=(),
        locations=(LOCATION,),
    )
    refused(only_the_provider, "ICEBERG_LOCATION_NOT_ALLOWED")
    assert verdict(dataclasses.replace(only_the_provider, location_provider=None)) is None
