"""The compatibility of a table, checked before a file is written (feature 035: FR-035-011, FR-035-012).

The host never writes a table it cannot write correctly, and it decides that from facts it has already read, before
any data file exists: the format version, the default data file format, the partition transforms of the current
spec, every location the table names, the size of its metadata and whether the target ref exists. Each refusal is a
code of the closed set with, for an unsupported table, a reason; none carries free text.

The function is pure over ``TableFacts``, so it is tested without a catalog. The host builds the facts from the
table it loaded.
"""

from __future__ import annotations

import re
from collections.abc import Collection, Sequence
from dataclasses import dataclass

from .config_hygiene import clean_path
from .failures import Failure

UNSUPPORTED_TABLE = "ICEBERG_UNSUPPORTED_TABLE"
LOCATION_NOT_ALLOWED = "ICEBERG_LOCATION_NOT_ALLOWED"
LIMIT_EXCEEDED = "ICEBERG_LIMIT_EXCEEDED"
TABLE_NOT_FOUND = "ICEBERG_TABLE_NOT_FOUND"

# The format versions of the Iceberg specification that PyIceberg writes. Version 3 (and anything newer) is refused.
SUPPORTED_FORMAT_VERSIONS = (1, 2)
# The transforms the runtime can apply when it writes. PyIceberg 0.12 needs the ``pyiceberg-core`` package to apply any
# other, and the runtime image does not have it (``requirements.lock``): writing to a table partitioned by ``year``,
# ``month``, ``day``, ``hour``, ``bucket`` or ``truncate`` fails with ``NotInstalledError``. Those tables are refused,
# with the reason ``TRANSFORM``, rather than failing in the middle of a write (G035-040).
_TRANSFORM = re.compile(r"identity|void")


@dataclass(frozen=True)
class TableFacts:
    """What the check needs to know of a loaded table."""

    format_version: int
    default_file_format: str
    # The names of the transforms of the current partition spec: ``identity``, ``day``, ``bucket[16]``...
    partition_transforms: Sequence[str]
    # The table location and every other location its metadata names: data, metadata and delete paths.
    locations: Sequence[str]
    metadata_bytes: int
    refs: Collection[str]
    # The class a table names to decide where its files go (``write.py-location-provider.impl``), or ``None``. PyIceberg
    # imports the module and calls the class, so a table that names one hands the placement of every file to code the
    # host has not read: no check of a location can see it, and the table is refused.
    location_provider: str | None = None


def check(
    table: TableFacts,
    *,
    ref: str,
    warehouse_locations: Sequence[str],
    max_metadata_bytes: int,
) -> None:
    """Raises the ``Failure`` that stops the write, or returns when the table can be written.

    The order is fixed so that the answer to a given table is always the same: the size of the metadata first,
    because nothing else is worth reading when it is too big, then the ref, the format version, the file format, the
    transforms, the location provider and the locations.
    """
    if table.metadata_bytes > max_metadata_bytes:
        raise Failure(LIMIT_EXCEEDED)
    if ref not in table.refs:
        raise Failure(TABLE_NOT_FOUND)
    if table.format_version not in SUPPORTED_FORMAT_VERSIONS:
        raise Failure(UNSUPPORTED_TABLE, reason="FORMAT_VERSION")
    if table.default_file_format.lower() != "parquet":
        raise Failure(UNSUPPORTED_TABLE, reason="FILE_FORMAT")
    if any(not _TRANSFORM.fullmatch(name) for name in table.partition_transforms):
        raise Failure(UNSUPPORTED_TABLE, reason="TRANSFORM")
    if table.location_provider is not None:
        raise Failure(LOCATION_NOT_ALLOWED)
    if any(not inside(location, warehouse_locations) for location in table.locations):
        raise Failure(LOCATION_NOT_ALLOWED)


def inside(location: str, allowed: Sequence[str]) -> bool:
    """Whether ``location`` is one of the allowed locations or below one, by path and not by prefix of text.

    ``s3://lake/warehouse-evil/t`` is not below ``s3://lake/warehouse/``. Below a root the path must be one the object
    store reads as it is written: ``clean_path`` of the connection's own hygiene refuses a ``..`` or ``.`` segment, a
    query or a fragment (dropped by the URL parser, so every file would land on one key), a backslash, an encoded dot or
    separator and a control character. With no allowed location nothing is inside, and a root that is no more than a
    scheme (or empty once its slashes are taken off) allows nothing either, rather than everything.
    """
    for root in (root.rstrip("/") for root in allowed):
        if not root.partition("://")[2]:
            continue
        if location == root or (location.startswith(root + "/") and clean_path(location[len(root) :])):
            return True
    return False
