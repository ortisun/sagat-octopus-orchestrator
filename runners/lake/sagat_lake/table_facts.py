"""The facts the pure checks need, read from the table PyIceberg loaded (feature 035: T035-057, FR-035-011, D035-030).

``compat.check`` and ``lookup.lookup`` take plain data so they are tested without a catalog. These three functions are
the only place that reads a loaded table to build it, and they read nothing else: no file is listed, no data is
scanned. They are called after the catalog answered and before the first file is written.

What is read is what PyIceberg 0.12 itself uses to decide where it writes, checked against its source: the table
location, the metadata file, ``write.data.path`` and ``write.metadata.path`` (an empty value means the default, as it
does for PyIceberg), and ``write.py-location-provider.impl``, the class PyIceberg imports and calls to place every file.
The Java-only ``write.folder-storage.path`` and ``write.object-storage.path`` are ignored by PyIceberg, so they are not
facts about where the host writes.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

from .compat import TableFacts
from .lookup import Snapshot

if TYPE_CHECKING:
    from pyiceberg.table import Table

DEFAULT_REF = "main"
_DEFAULT_FILE_FORMAT = "write.format.default"
_DATA_PATH = "write.data.path"
_METADATA_PATH = "write.metadata.path"
_LOCATION_PROVIDER = "write.py-location-provider.impl"


def facts_of(table: Table) -> TableFacts:
    """What ``compat.check`` needs to know of ``table``."""
    properties = table.metadata.properties
    locations = [table.metadata.location, table.metadata_location]
    locations.extend(path for path in (properties.get(_DATA_PATH), properties.get(_METADATA_PATH)) if path)
    return TableFacts(
        format_version=table.metadata.format_version,
        default_file_format=properties.get(_DEFAULT_FILE_FORMAT, "parquet"),
        partition_transforms=tuple(str(field.transform) for field in table.spec().fields),
        locations=tuple(locations),
        metadata_bytes=len(table.io.new_input(table.metadata_location)),
        # The first commit to a table creates ``main``, and the host never creates another ref, so ``main`` is a ref
        # of a table that has no commit yet.
        refs=frozenset(table.metadata.refs) | {DEFAULT_REF},
        location_provider=properties.get(_LOCATION_PROVIDER) or None,
    )


def refs_of(table: Table) -> dict[str, int]:
    """The head of every branch and tag of ``table`` that points at a snapshot."""
    return {name: ref.snapshot_id for name, ref in table.metadata.refs.items()}


def snapshots_of(table: Table) -> dict[int, Snapshot]:
    """Every snapshot the table still holds, by id, as the walk of ``lookup`` reads them.

    The size of the metadata is checked first (``compat.check``), so this never reads more than the bound allows.
    """
    return {
        snapshot.snapshot_id: Snapshot(
            snapshot_id=snapshot.snapshot_id,
            parent_id=snapshot.parent_snapshot_id,
            timestamp_ms=snapshot.timestamp_ms,
            summary=dict(snapshot.summary.additional_properties) if snapshot.summary else {},
        )
        for snapshot in table.metadata.snapshots
    }
