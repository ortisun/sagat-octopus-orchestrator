"""The facts the pure checks need, read from the table PyIceberg loaded (feature 035: T035-057, FR-035-011, D035-030).

``compat.check`` and ``lookup.lookup`` take plain data so they are tested without a catalog. These three functions are
the only place that reads a loaded table to build it, and they read nothing else: no file is listed, no data is
scanned. They are called after the catalog answered and before the first file is written.

What is read is what PyIceberg 0.12 itself uses to decide where it reads, writes and deletes, checked against its
source: the table location, the metadata file, ``write.data.path`` and ``write.metadata.path`` (an empty value means the
default, as it does for PyIceberg), ``write.py-location-provider.impl`` (the class PyIceberg imports and calls to place
every file), the manifest list of every snapshot (read to build the next one), the metadata log (old metadata files are
deleted from it after a commit) and the statistics files. The Java-only ``write.folder-storage.path`` and
``write.object-storage.path`` are ignored by PyIceberg, so they are not facts about where the host writes.

The table is already loaded and parsed when these functions run: they bound what the host goes on to do, not what the
load cost.
"""

from __future__ import annotations

from collections.abc import Sequence
from typing import TYPE_CHECKING

from pyiceberg.table.refs import SnapshotRefType

from .compat import TableFacts, inside
from .lookup import Snapshot

if TYPE_CHECKING:
    from pyiceberg.table import Table

DEFAULT_REF = "main"
_DEFAULT_FILE_FORMAT = "write.format.default"
_DATA_PATH = "write.data.path"
_METADATA_PATH = "write.metadata.path"
_LOCATION_PROVIDER = "write.py-location-provider.impl"


def facts_of(table: Table, *, warehouse_locations: Sequence[str]) -> TableFacts:
    """What ``compat.check`` needs to know of ``table``.

    The size of the metadata file is asked of the object store only when the file lies inside the warehouse. For one
    that does not, the size is ``0`` and the check refuses the location: the host does not ask about someone else's
    object.
    """
    properties = table.metadata.properties
    locations = [table.metadata.location, table.metadata_location]
    locations.extend(path for path in (properties.get(_DATA_PATH), properties.get(_METADATA_PATH)) if path)
    locations.extend(snapshot.manifest_list for snapshot in table.metadata.snapshots if snapshot.manifest_list)
    locations.extend(entry.metadata_file for entry in table.metadata.metadata_log)
    locations.extend(file.statistics_path for file in table.metadata.statistics)
    locations.extend(file.statistics_path for file in table.metadata.partition_statistics)
    readable = inside(table.metadata_location, warehouse_locations)
    return TableFacts(
        format_version=table.metadata.format_version,
        default_file_format=properties.get(_DEFAULT_FILE_FORMAT, "parquet"),
        partition_transforms=tuple(str(field.transform) for field in table.spec().fields),
        locations=tuple(locations),
        metadata_bytes=len(table.io.new_input(table.metadata_location)) if readable else 0,
        # Only a branch can be written to (PyIceberg refuses a tag, and an overwrite refuses only after it has written
        # its data files). The first commit to a table creates ``main`` and the host never creates another ref, so
        # ``main`` is a ref of a table that has no commit yet.
        refs=frozenset(
            name for name, ref in table.metadata.refs.items() if ref.snapshot_ref_type == SnapshotRefType.BRANCH
        )
        | {DEFAULT_REF},
        location_provider=properties.get(_LOCATION_PROVIDER) or None,
    )


def refs_of(table: Table) -> dict[str, int]:
    """The head of every branch and tag of ``table``: the lookup reads both, because the key found under a tag is an
    inconsistency it must report."""
    return {name: ref.snapshot_id for name, ref in table.metadata.refs.items()}


def snapshots_of(table: Table) -> dict[int, Snapshot]:
    """Every snapshot the table still holds, by id, as the walk of ``lookup`` reads them."""
    return {
        snapshot.snapshot_id: Snapshot(
            snapshot_id=snapshot.snapshot_id,
            parent_id=snapshot.parent_snapshot_id,
            timestamp_ms=snapshot.timestamp_ms,
            summary=dict(snapshot.summary.additional_properties) if snapshot.summary else {},
        )
        for snapshot in table.metadata.snapshots
    }
