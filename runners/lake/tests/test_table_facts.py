"""The facts the pure checks need, read from a table PyIceberg loaded (feature 035: T035-057, FR-035-011, D035-030).

The checks of ``compat.py`` and ``lookup.py`` take plain data. These adapters build it from a real table: a local SQLite
catalog with a file warehouse, so nothing here needs a network or a container.
"""

from __future__ import annotations

import pathlib

import pyarrow as pa  # type: ignore[import-untyped]
import pytest
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.partitioning import PartitionField, PartitionSpec
from pyiceberg.schema import Schema
from pyiceberg.table import Table
from pyiceberg.table.snapshots import MetadataLogEntry
from pyiceberg.table.statistics import PartitionStatisticsFile, StatisticsFile
from pyiceberg.transforms import BucketTransform, IdentityTransform
from pyiceberg.types import NestedField, StringType

from sagat_lake.compat import TableFacts, check
from sagat_lake.failures import Failure
from sagat_lake.lookup import Absent, Found, lookup
from sagat_lake.table_facts import facts_of, refs_of, snapshots_of

SCHEMA = Schema(NestedField(1, "id", StringType(), required=False))
NAME = "sales.orders"
ROWS = pa.table({"id": ["a", "b"]})
EVERYTHING_RECENT = 0  # the first attempt began at the epoch: the walk can only end at the first snapshot


@pytest.fixture
def catalog(tmp_path: pathlib.Path) -> SqlCatalog:
    catalog = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    catalog.create_namespace("sales")
    return catalog


def create(catalog: SqlCatalog, name: str = "orders", **kwargs: object) -> Table:
    return catalog.create_table(f"sales.{name}", schema=SCHEMA, **kwargs)  # type: ignore[arg-type]


def head_of(table: Table) -> int:
    head = table.metadata.current_snapshot_id
    assert head is not None, "the table has no commit"
    return head


def warehouse_of(table: Table) -> str:
    return table.metadata.location.rsplit("/sales/", 1)[0] + "/"


def facts(table: Table) -> TableFacts:
    return facts_of(table, warehouse_locations=[warehouse_of(table)])


OUTSIDE = "s3://elsewhere/x"


# ------------------------------------------------------------------------------------------------------------- facts
def test_the_facts_of_a_new_unpartitioned_table(catalog: SqlCatalog) -> None:
    table = create(catalog)
    got = facts(table)
    assert got.format_version == 2
    assert got.default_file_format == "parquet"
    assert tuple(got.partition_transforms) == ()
    assert table.metadata.location in got.locations
    assert table.metadata_location in got.locations
    assert got.metadata_bytes > 0
    # `main` is a ref the first commit creates, and the host never creates one: it is there for a table with no commit
    assert "main" in got.refs


def test_the_transforms_are_those_of_the_current_partition_spec(catalog: SqlCatalog) -> None:
    spec = PartitionSpec(PartitionField(source_id=1, field_id=1000, transform=IdentityTransform(), name="id"))
    assert tuple(facts(create(catalog, "partitioned", partition_spec=spec)).partition_transforms) == ("identity",)


def test_the_transforms_are_those_of_the_spec_in_force_after_the_table_was_evolved(catalog: SqlCatalog) -> None:
    # writes follow the current spec; the first one would accept a table that now needs a transform it cannot apply
    spec = PartitionSpec(PartitionField(source_id=1, field_id=1000, transform=IdentityTransform(), name="id"))
    create(catalog, "evolved", partition_spec=spec)
    table = catalog.load_table("sales.evolved")
    with table.update_spec() as update:
        update.remove_field("id")
        update.add_field("id", BucketTransform(4), "id_bucket")
    evolved = catalog.load_table("sales.evolved")
    assert len(evolved.specs()) == 2
    assert tuple(facts(evolved).partition_transforms) == ("bucket[4]",)


def test_the_format_version_is_that_of_the_table(catalog: SqlCatalog) -> None:
    assert facts(create(catalog, "one", properties={"format-version": "1"})).format_version == 1
    assert facts(create(catalog, "two", properties={"format-version": "2"})).format_version == 2


def test_the_default_file_format_is_the_property_of_the_table(catalog: SqlCatalog) -> None:
    table = create(catalog, properties={"write.format.default": "orc"})
    assert facts(table).default_file_format == "orc"


def test_the_data_and_metadata_paths_of_the_properties_are_facts(catalog: SqlCatalog, tmp_path: pathlib.Path) -> None:
    data, meta = f"file://{tmp_path}/elsewhere/data", f"file://{tmp_path}/elsewhere/meta"
    table = create(catalog, properties={"write.data.path": data, "write.metadata.path": meta})
    assert {data, meta} <= set(facts(table).locations)


def test_an_empty_path_property_means_the_default_as_it_does_for_pyiceberg(catalog: SqlCatalog) -> None:
    table = create(catalog, properties={"write.data.path": "", "write.metadata.path": ""})
    assert "" not in facts(table).locations


def test_the_java_only_paths_are_not_facts_because_pyiceberg_ignores_them(catalog: SqlCatalog) -> None:
    table = create(
        catalog,
        properties={
            "write.folder-storage.path": "s3://legacy/data",
            "write.object-storage.path": "s3://legacy/objects",
        },
    )
    assert not {"s3://legacy/data", "s3://legacy/objects"} & set(facts(table).locations)
    # and they do not move a single file: the provider PyIceberg builds still places data under the table
    assert table.location_provider().new_data_location("f.parquet").startswith(table.metadata.location)


def with_properties(table: Table, **properties: str) -> Table:
    """The table as if its metadata carried these properties. A catalog that loads the class a table names would fail to
    create one, so the property is set on the loaded metadata."""
    table.metadata = table.metadata.model_copy(update={"properties": {**table.metadata.properties, **properties}})
    return table


def test_a_table_with_no_location_provider_of_its_own_names_none(catalog: SqlCatalog) -> None:
    assert facts(create(catalog)).location_provider is None
    blank = with_properties(create(catalog, "blank"), **{"write.py-location-provider.impl": ""})
    assert facts(blank).location_provider is None


def test_a_table_that_names_a_location_provider_is_refused(catalog: SqlCatalog) -> None:
    table = with_properties(create(catalog), **{"write.py-location-provider.impl": "evil.module.Provider"})
    assert facts(table).location_provider == "evil.module.Provider"
    with pytest.raises(Failure) as caught:
        check(facts(table), ref="main", warehouse_locations=[warehouse_of(table)], max_metadata_bytes=10**7)
    assert caught.value.code == "ICEBERG_LOCATION_NOT_ALLOWED"


def test_pyiceberg_does_import_the_class_a_table_names(catalog: SqlCatalog) -> None:
    # why the property is a refusal and not a detail: creating a table that names a class makes PyIceberg import it
    with pytest.raises(ValueError, match="Could not initialize LocationProvider"):
        create(catalog, "named", properties={"write.py-location-provider.impl": "evil.module.Provider"})


def test_a_location_outside_the_warehouse_that_only_a_property_names_is_still_caught(
    catalog: SqlCatalog, tmp_path: pathlib.Path
) -> None:
    table = create(catalog, properties={"write.data.path": f"file://{tmp_path}/elsewhere/data"})
    with pytest.raises(Failure) as caught:
        check(facts(table), ref="main", warehouse_locations=[warehouse_of(table)], max_metadata_bytes=10**7)
    assert caught.value.code == "ICEBERG_LOCATION_NOT_ALLOWED"


def test_the_size_of_the_metadata_is_that_of_the_file(catalog: SqlCatalog, tmp_path: pathlib.Path) -> None:
    table = create(catalog)
    on_disk = pathlib.Path(table.metadata_location.removeprefix("file://")).stat().st_size
    assert facts(table).metadata_bytes == on_disk


def test_a_branch_is_a_ref_of_the_table(catalog: SqlCatalog) -> None:
    table = create(catalog)
    table.append(ROWS)
    table = catalog.load_table("sales.orders")
    with table.manage_snapshots() as manage:
        manage.create_branch(head_of(table), "audit")
    assert {"main", "audit"} <= set(facts(catalog.load_table("sales.orders")).refs)


# --------------------------------------------------------------------------------------------------------- snapshots
def test_the_snapshots_keep_their_parents_times_and_summaries(catalog: SqlCatalog) -> None:
    table = create(catalog)
    table.append(ROWS, snapshot_properties={"sagat.effect-key": "k1"})
    table = catalog.load_table("sales.orders")
    table.append(ROWS, snapshot_properties={"sagat.effect-key": "k2"})
    table = catalog.load_table("sales.orders")

    snapshots = snapshots_of(table)
    first, second = table.metadata.snapshots
    assert set(snapshots) == {first.snapshot_id, second.snapshot_id}
    assert snapshots[first.snapshot_id].parent_id is None
    assert snapshots[second.snapshot_id].parent_id == first.snapshot_id
    assert snapshots[second.snapshot_id].timestamp_ms == second.timestamp_ms
    assert snapshots[first.snapshot_id].summary["sagat.effect-key"] == "k1"
    assert snapshots[second.snapshot_id].summary["sagat.effect-key"] == "k2"


def test_a_snapshot_with_no_summary_has_an_empty_one(catalog: SqlCatalog) -> None:
    # a format v1 snapshot may have none, and the walk reads the summary of every snapshot it passes
    table = create(catalog)
    table.append(ROWS)
    table = catalog.load_table("sales.orders")
    bare = [snapshot.model_copy(update={"summary": None}) for snapshot in table.metadata.snapshots]
    table.metadata = table.metadata.model_copy(update={"snapshots": bare})
    assert snapshots_of(table)[head_of(table)].summary == {}


def test_a_table_with_no_commit_has_no_snapshots_and_no_ref_with_a_head(catalog: SqlCatalog) -> None:
    table = create(catalog)
    assert snapshots_of(table) == {}
    assert refs_of(table) == {}


def test_the_refs_point_to_their_heads(catalog: SqlCatalog) -> None:
    table = create(catalog)
    table.append(ROWS)
    table = catalog.load_table("sales.orders")
    head = head_of(table)
    with table.manage_snapshots() as manage:
        manage.create_branch(head, "audit")
    assert refs_of(catalog.load_table("sales.orders")) == {"main": head, "audit": head}


# ------------------------------------------------------------------------------------------- with the checks themselves
def test_the_checks_accept_a_table_the_host_can_write(catalog: SqlCatalog) -> None:
    table = create(catalog)
    check(facts(table), ref="main", warehouse_locations=[warehouse_of(table)], max_metadata_bytes=10**7)


def test_the_checks_refuse_the_same_table_for_another_warehouse(catalog: SqlCatalog) -> None:
    table = create(catalog)
    with pytest.raises(Failure) as caught:
        check(facts(table), ref="main", warehouse_locations=["s3://other/"], max_metadata_bytes=10**7)
    assert caught.value.code == "ICEBERG_LOCATION_NOT_ALLOWED"


def test_the_lookup_finds_the_commit_of_a_step_in_a_real_table(catalog: SqlCatalog) -> None:
    table = create(catalog)
    table.append(ROWS, snapshot_properties={"sagat.effect-key": "run-1:save"})
    table = catalog.load_table("sales.orders")
    table.append(ROWS, snapshot_properties={"sagat.effect-key": "run-2:save"})
    table = catalog.load_table("sales.orders")
    first = table.metadata.snapshots[0]

    found = lookup(
        "run-1:save",
        ref="main",
        refs=refs_of(table),
        snapshots=snapshots_of(table),
        first_attempt_started_at_ms=EVERYTHING_RECENT,
    )
    assert found == Found(snapshot_id=first.snapshot_id)
    absent = lookup(
        "run-3:save",
        ref="main",
        refs=refs_of(table),
        snapshots=snapshots_of(table),
        first_attempt_started_at_ms=EVERYTHING_RECENT,
    )
    assert absent == Absent()


# -------------------------------------------------------------------------------------------- what the review found
def test_a_tag_is_not_a_ref_the_host_can_write_but_the_lookup_still_reads_it(catalog: SqlCatalog) -> None:
    # PyIceberg refuses to write to a tag, and an overwrite refuses only after it has written its data files
    table = create(catalog)
    table.append(ROWS)
    table = catalog.load_table(NAME)
    head = head_of(table)
    with table.manage_snapshots() as manage:
        manage.create_tag(head, "v1")
        manage.create_branch(head, "audit")
    loaded = catalog.load_table(NAME)
    assert "audit" in facts(loaded).refs
    assert "v1" not in facts(loaded).refs
    assert refs_of(loaded)["v1"] == head
    with pytest.raises(Failure) as caught:
        check(facts(loaded), ref="v1", warehouse_locations=[warehouse_of(loaded)], max_metadata_bytes=10**7)
    assert caught.value.code == "ICEBERG_TABLE_NOT_FOUND"


def test_the_manifest_lists_and_the_metadata_log_a_table_names_are_facts_too(catalog: SqlCatalog) -> None:
    table = create(catalog)
    table.append(ROWS)
    table.append(ROWS)
    table = catalog.load_table(NAME)
    locations = set(facts(table).locations)
    assert {snapshot.manifest_list for snapshot in table.metadata.snapshots} <= locations
    assert {entry.metadata_file for entry in table.metadata.metadata_log} <= locations
    assert table.metadata.metadata_log


def pointing_outside(table: Table, where: str) -> Table:
    """The table as if one location of its metadata had been written to point outside the warehouse."""
    head = head_of(table)
    update: dict[str, object]
    if where == "manifest-list":
        update = {"snapshots": [s.model_copy(update={"manifest_list": OUTSIDE}) for s in table.metadata.snapshots]}
    elif where == "metadata-log":
        update = {"metadata_log": [MetadataLogEntry.model_validate({"metadata-file": OUTSIDE, "timestamp-ms": 1})]}
    elif where == "statistics":
        statistics = StatisticsFile.model_validate(
            {
                "snapshot-id": head,
                "statistics-path": OUTSIDE,
                "file-size-in-bytes": 1,
                "file-footer-size-in-bytes": 1,
                "blob-metadata": [],
            }
        )
        update = {"statistics": [statistics]}
    else:
        update = {
            "partition_statistics": [
                PartitionStatisticsFile.model_validate(
                    {"snapshot-id": head, "statistics-path": OUTSIDE, "file-size-in-bytes": 1}
                )
            ]
        }
    table.metadata = table.metadata.model_copy(update=update)
    return table


@pytest.mark.parametrize("where", ["manifest-list", "metadata-log", "statistics", "partition-statistics"])
def test_a_location_the_metadata_names_outside_the_warehouse_is_refused(catalog: SqlCatalog, where: str) -> None:
    # PyIceberg reads the manifest list of the parent snapshot and deletes the old metadata files of the log
    table = create(catalog)
    table.append(ROWS)
    table = pointing_outside(catalog.load_table(NAME), where)
    with pytest.raises(Failure) as caught:
        check(facts(table), ref="main", warehouse_locations=[warehouse_of(table)], max_metadata_bytes=10**7)
    assert caught.value.code == "ICEBERG_LOCATION_NOT_ALLOWED"


def test_the_metadata_file_outside_the_warehouse_is_not_even_looked_at(
    catalog: SqlCatalog, tmp_path: pathlib.Path
) -> None:
    # the size of a file is asked of the object store; asking about a location the connection does not allow would
    # answer a question about someone else's object
    elsewhere = tmp_path / "elsewhere" / "meta.json"
    elsewhere.parent.mkdir()
    elsewhere.write_text("{" + " " * 500 + "}")
    table = create(catalog)
    table.metadata_location = f"file://{elsewhere}"
    assert facts(table).metadata_bytes == 0
    with pytest.raises(Failure) as caught:
        check(facts(table), ref="main", warehouse_locations=[warehouse_of(table)], max_metadata_bytes=10**7)
    assert caught.value.code == "ICEBERG_LOCATION_NOT_ALLOWED"


def test_a_metadata_file_that_only_starts_like_the_warehouse_is_not_asked_about(
    catalog: SqlCatalog, tmp_path: pathlib.Path
) -> None:
    evil = tmp_path / "warehouse-evil" / "meta.json"
    evil.parent.mkdir()
    evil.write_text("{" + " " * 500 + "}")
    table = create(catalog)
    table.metadata_location = f"file://{evil}"
    # the root has no trailing slash: by text `warehouse-evil` starts with it, by path it is another directory
    assert facts_of(table, warehouse_locations=[f"file://{tmp_path}/warehouse"]).metadata_bytes == 0
