"""The premise of the filter on a table's own properties (feature 035: G035-041, D035-050).

``Hygiene.table_properties`` keeps only the families of properties a write needs because PyIceberg builds the file IO of
a table from the catalog's properties overridden by the table's own. These tests show it with PyIceberg 0.12 itself: the
catalog pins ``py-io-impl``, the table names another class, and PyIceberg builds the table's. A newer PyIceberg that
stops doing so fails them, and the filter is looked at again then.
"""

from __future__ import annotations

import pathlib
from collections.abc import Iterator

import pytest
from pyiceberg.catalog.sql import SqlCatalog
from pyiceberg.schema import Schema
from pyiceberg.types import NestedField, StringType

import recording_io

PINNED = "pyiceberg.io.pyarrow.PyArrowFileIO"
NAMED = "recording_io.RecordingFileIO"
SCHEMA = Schema(NestedField(1, "id", StringType(), required=False))


@pytest.fixture
def catalog(tmp_path: pathlib.Path) -> Iterator[SqlCatalog]:
    catalog = SqlCatalog(
        "t",
        uri=f"sqlite:///{tmp_path}/catalog.db",
        warehouse=f"file://{tmp_path}/warehouse",
        **{"py-io-impl": PINNED},
    )
    catalog.create_namespace("sales")
    recording_io.BUILT.clear()
    yield catalog
    recording_io.BUILT.clear()


def test_a_table_that_names_a_file_io_class_gets_it_built_over_the_one_the_catalog_pins(catalog: SqlCatalog) -> None:
    catalog.create_table("sales.named", schema=SCHEMA, properties={"py-io-impl": NAMED})
    recording_io.BUILT.clear()

    catalog.load_table("sales.named")

    assert len(recording_io.BUILT) == 1
    assert recording_io.BUILT[0]["py-io-impl"] == NAMED


def test_a_table_that_names_nothing_gets_the_one_the_catalog_pins(catalog: SqlCatalog) -> None:
    # the control: the class above is built because the table named it, not because the test builds it anyway
    catalog.create_table("sales.plain", schema=SCHEMA)
    catalog.load_table("sales.plain")

    assert recording_io.BUILT == []
