"""The directory of one attempt (feature 035: T035-057, D035-030 step 4, FR-035-020, D035-095).

The data files of an attempt are written under one directory of the table's data location, so that the host can
delete exactly its own files when a commit is definitively refused. The directory is set on the table object in memory,
like the retry of ``commit.py``, and the stored table never has it. The deletion is the dangerous half: it refuses
anything but a directory that is inside the warehouse and ends in the attempt's own id.
"""

from __future__ import annotations

import pathlib

import pyarrow as pa  # type: ignore[import-untyped]
import pytest
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.exceptions import CommitFailedException
from pyiceberg.schema import Schema
from pyiceberg.table import Table
from pyiceberg.types import NestedField, StringType

from sagat_lake.attempt import pin_attempt_directory, remove_attempt_files
from sagat_lake.failures import Failure

NAME = "sales.orders"
ROWS = pa.table({"id": [str(i) for i in range(100)]})
SCHEMA = Schema(NestedField(1, "id", StringType(), required=False))


@pytest.fixture
def catalog(tmp_path: pathlib.Path) -> SqlCatalog:
    catalog = SqlCatalog("t", uri=f"sqlite:///{tmp_path}/catalog.db", warehouse=f"file://{tmp_path}/warehouse")
    catalog.create_namespace("sales")
    return catalog


def create(catalog: SqlCatalog, **properties: str) -> Table:
    return catalog.create_table(NAME, schema=SCHEMA, properties=properties)


def warehouse_of(table: Table) -> list[str]:
    return [table.metadata.location.rsplit("/sales/", 1)[0] + "/"]


def files_under(location: str) -> list[str]:
    root = pathlib.Path(location.removeprefix("file://"))
    return sorted(str(path.relative_to(root)) for path in root.rglob("*") if path.is_file())


def parquet_files(table: Table) -> list[str]:
    return [name for name in files_under(table.metadata.location) if name.endswith(".parquet")]


# ---------------------------------------------------------------------------------------------------------------- pin
def test_the_directory_is_below_the_data_location_of_the_table_and_is_named_by_the_attempt(catalog: SqlCatalog) -> None:
    table = create(catalog)
    assert pin_attempt_directory(table, "att-42") == f"{table.metadata.location}/data/att-42"


def test_a_table_location_with_a_trailing_slash_does_not_make_a_double_slash(catalog: SqlCatalog) -> None:
    # PyIceberg trims the location of a table it creates; the metadata written by another engine may not
    table = create(catalog)
    table.metadata = table.metadata.model_copy(update={"location": table.metadata.location + "/"})
    directory = pin_attempt_directory(table, "att-42")
    assert "//" not in directory.removeprefix("file://")
    assert directory.endswith("/data/att-42")


def test_a_table_that_moves_its_data_location_keeps_the_attempt_below_it(
    catalog: SqlCatalog, tmp_path: pathlib.Path
) -> None:
    moved = f"file://{tmp_path}/warehouse/elsewhere/"
    table = create(catalog, **{"write.data.path": moved})
    assert pin_attempt_directory(table, "att-42") == f"file://{tmp_path}/warehouse/elsewhere/att-42"


def test_the_files_of_the_attempt_are_written_in_its_directory_and_nowhere_else_in_the_data_location(
    catalog: SqlCatalog,
) -> None:
    table = create(catalog)
    table.append(ROWS)
    table = catalog.load_table(NAME)
    pin_attempt_directory(table, "att-42")

    table.append(ROWS)

    written = parquet_files(table)
    assert len(written) == 2
    assert sum(name.startswith("data/att-42/") for name in written) == 1
    assert sum(name.startswith("data/00000") for name in written) == 1


def test_the_directory_is_in_memory_only_and_the_stored_table_never_has_it(catalog: SqlCatalog) -> None:
    table = create(catalog)
    pin_attempt_directory(table, "att-42")
    table.append(ROWS)
    assert "write.data.path" not in catalog.load_table(NAME).metadata.properties


def test_the_other_properties_of_the_table_are_kept(catalog: SqlCatalog) -> None:
    table = create(catalog, owner="sales")
    pin_attempt_directory(table, "att-42")
    assert table.metadata.properties["owner"] == "sales"


def test_files_of_a_table_laid_out_by_hash_stay_inside_the_directory_of_the_attempt(catalog: SqlCatalog) -> None:
    table = create(catalog, **{"write.object-storage.enabled": "true"})
    pin_attempt_directory(table, "att-42")
    table.append(ROWS)
    assert parquet_files(table)
    assert all(name.startswith("data/att-42/") for name in parquet_files(table))


@pytest.mark.parametrize(
    "attempt_id",
    ["", "-lead", "a/b", "../x", "a b", "a.b", "a\x00b", "x" * 65, "ä", "a\n"],
)
def test_an_id_that_could_name_another_directory_is_refused(catalog: SqlCatalog, attempt_id: str) -> None:
    with pytest.raises(ValueError, match="attempt id"):
        pin_attempt_directory(create(catalog), attempt_id)


def test_ids_of_the_platform_are_accepted(catalog: SqlCatalog) -> None:
    table = create(catalog)
    for attempt_id in ["0", "a", "att-42", "0190a5e2-7b3c-7d2e-9f10-aabbccddeeff", "A" + "b" * 63]:
        assert pin_attempt_directory(table, attempt_id).endswith("/" + attempt_id)


# --------------------------------------------------------------------------------------------------------------- remove
def refused_commit(catalog: SqlCatalog, attempt_id: str) -> tuple[Table, str]:
    """An attempt whose commit lost a race: its data file was written, the catalog said no."""
    table = catalog.load_table(NAME)
    stale = catalog.load_table(NAME)
    directory = pin_attempt_directory(table, attempt_id)
    table.metadata = table.metadata.model_copy(
        update={"properties": {**table.metadata.properties, "commit.retry.num-retries": "0"}}
    )
    stale.append(ROWS)  # moves the ref
    with pytest.raises(CommitFailedException):
        table.append(ROWS)
    return table, directory


def test_a_refused_commit_leaves_the_data_file_of_the_attempt_behind_and_no_metadata_of_it(
    catalog: SqlCatalog,
) -> None:
    # PyIceberg removes the manifests it built for the commit that was refused; the data file it cannot know to remove
    create(catalog)
    table = catalog.load_table(NAME)
    stale = catalog.load_table(NAME)
    directory = pin_attempt_directory(table, "att-43")
    table.metadata = table.metadata.model_copy(
        update={"properties": {**table.metadata.properties, "commit.retry.num-retries": "0"}}
    )
    stale.append(ROWS)
    metadata_before = files_under(f"{table.metadata.location}/metadata")

    with pytest.raises(CommitFailedException):
        table.append(ROWS)

    assert len(files_under(directory)) == 1
    assert files_under(f"{table.metadata.location}/metadata") == metadata_before


def test_what_is_in_the_directory_of_the_attempt_is_deleted_and_nothing_else(catalog: SqlCatalog) -> None:
    create(catalog)
    table, directory = refused_commit(catalog, "att-43")
    kept = [name for name in files_under(table.metadata.location) if not name.startswith("data/att-43/")]
    assert kept  # the other writer's data and every metadata file

    removed = remove_attempt_files(table, directory, "att-43", warehouse_locations=warehouse_of(table))

    assert removed == 1
    assert files_under(directory) == []
    assert [name for name in files_under(table.metadata.location) if not name.startswith("data/att-43/")] == kept


def test_the_files_of_another_attempt_are_not_touched(catalog: SqlCatalog) -> None:
    table = create(catalog)
    other = pin_attempt_directory(table, "att-1")
    table.append(ROWS)
    table = catalog.load_table(NAME)
    mine = pin_attempt_directory(table, "att-2")
    table.append(ROWS)
    assert files_under(other)
    assert files_under(mine)

    remove_attempt_files(table, mine, "att-2", warehouse_locations=warehouse_of(table))

    assert files_under(mine) == []
    assert files_under(other)


def test_a_directory_that_does_not_exist_has_nothing_to_remove(catalog: SqlCatalog) -> None:
    table = create(catalog)
    directory = pin_attempt_directory(table, "never-written")
    assert remove_attempt_files(table, directory, "never-written", warehouse_locations=warehouse_of(table)) == 0


def test_a_directory_written_with_a_trailing_slash_is_the_same_directory(catalog: SqlCatalog) -> None:
    create(catalog)
    table, directory = refused_commit(catalog, "att-43")
    assert remove_attempt_files(table, directory + "/", "att-43", warehouse_locations=warehouse_of(table)) == 1
    assert files_under(directory) == []


def test_removing_twice_is_the_same_as_removing_once(catalog: SqlCatalog) -> None:
    create(catalog)
    table, directory = refused_commit(catalog, "att-43")
    first = remove_attempt_files(table, directory, "att-43", warehouse_locations=warehouse_of(table))
    second = remove_attempt_files(table, directory, "att-43", warehouse_locations=warehouse_of(table))
    assert (first, second) == (1, 0)


def test_the_files_of_a_hash_layout_are_found_below_the_directory(catalog: SqlCatalog) -> None:
    table = create(catalog, **{"write.object-storage.enabled": "true"})
    directory = pin_attempt_directory(table, "att-42")
    table.append(ROWS)
    assert files_under(directory)
    assert remove_attempt_files(table, directory, "att-42", warehouse_locations=warehouse_of(table)) >= 1
    assert files_under(directory) == []


@pytest.mark.parametrize(
    "directory",
    [
        "{location}/data",  # the whole data location, not an attempt's directory
        "{location}/data/att-other",  # another attempt's directory
        "{location}/data/att-420",  # an id that starts with the attempt's
        "{location}/data/x-att-42",  # and one that ends with it
        "{location}/data/att-42/..",  # climbs out of it
        "{location}/data/att-42/../att-42",
        "{location}/data/att-42/sub",  # below, not the directory itself
        "{location}/data/att-42?x",
        "{location}/data/att-42#",
        "{location}/data/att-42/%2e%2e/att-42",
        "file:///etc/att-42",  # outside the warehouse
        "s3://other-bucket/data/att-42",
        "",
    ],
)
def test_only_the_directory_of_the_attempt_inside_the_warehouse_is_ever_deleted(
    catalog: SqlCatalog, directory: str
) -> None:
    table = create(catalog)
    bystander = pin_attempt_directory(table, "att-42")
    table.append(ROWS)
    before = files_under(table.metadata.location)

    with pytest.raises(Failure) as refused:
        remove_attempt_files(
            table,
            directory.format(location=table.metadata.location),
            "att-42",
            warehouse_locations=warehouse_of(table),
        )

    assert refused.value.code == "WORKER_ERROR"
    assert files_under(table.metadata.location) == before
    assert files_under(bystander)


def test_an_id_that_is_not_one_is_refused_before_anything_is_listed(catalog: SqlCatalog) -> None:
    table = create(catalog)
    with pytest.raises(ValueError, match="attempt id"):
        remove_attempt_files(table, f"{table.metadata.location}/data/", "", warehouse_locations=warehouse_of(table))


def test_a_file_io_whose_listing_is_not_written_deletes_nothing(tmp_path: pathlib.Path) -> None:
    # the file IO of ADLS is fsspec's (G035-044); the local one stands in for it
    catalog = SqlCatalog(
        "t",
        uri=f"sqlite:///{tmp_path}/fsspec.db",
        warehouse=f"file://{tmp_path}/fsspec-warehouse",
        **{"py-io-impl": "pyiceberg.io.fsspec.FsspecFileIO"},
    )
    catalog.create_namespace("sales")
    table = create(catalog)
    directory = pin_attempt_directory(table, "att-42")
    table.append(ROWS)
    assert files_under(directory)

    with pytest.raises(Failure) as refused:
        remove_attempt_files(table, directory, "att-42", warehouse_locations=warehouse_of(table))

    assert refused.value.code == "WORKER_ERROR"
    assert files_under(directory)
