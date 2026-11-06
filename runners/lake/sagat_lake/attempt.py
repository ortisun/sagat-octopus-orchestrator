"""The directory of one attempt (feature 035: T035-057, D035-030 step 4, FR-035-020, D035-095).

The data files of an attempt are written under one directory of the table's data location, named by the attempt's id,
so that the host can delete exactly its own files when a commit is definitively refused. PyIceberg removes the
manifests it built for a commit that was refused but not the data files, which it cannot know to remove.

``pin_attempt_directory`` sets ``write.data.path`` on the table object in memory, the way ``commit.py`` sets the
retry: the stored table never has it. ``remove_attempt_files`` is the dangerous half and trusts nothing: it deletes only
below a directory that is inside the warehouse (by path, and clean by the rule of the connection's hygiene) and ends in
the attempt's own id.
"""

from __future__ import annotations

import re
from typing import TYPE_CHECKING

from .compat import inside
from .failures import WORKER_ERROR, Failure

if TYPE_CHECKING:
    from collections.abc import Sequence

    from pyiceberg.table import Table

# An id of the platform (a UUID or a short slug), and nothing that could name another directory.
_ATTEMPT_ID = re.compile(r"[A-Za-z0-9][A-Za-z0-9-]{0,63}")
_DATA_PATH = "write.data.path"


def _checked(attempt_id: str) -> str:
    if not _ATTEMPT_ID.fullmatch(attempt_id):
        raise ValueError("an attempt id is 1 to 64 letters, digits and hyphens, and starts with a letter or a digit")
    return attempt_id


def pin_attempt_directory(table: Table, attempt_id: str) -> str:
    """Makes PyIceberg write the data files of the next append under ``<data location>/<attempt id>``, in memory.

    Returns the directory. The data location is the table's own ``write.data.path`` or ``<location>/data``, computed
    here from the properties and not by asking PyIceberg for its location provider, which imports the class a table
    names.
    """
    _checked(attempt_id)
    properties = table.metadata.properties
    root = (properties.get(_DATA_PATH) or f"{table.metadata.location.rstrip('/')}/data").rstrip("/")
    directory = f"{root}/{attempt_id}"
    table.metadata = table.metadata.model_copy(update={"properties": {**properties, _DATA_PATH: directory}})
    return directory


def remove_attempt_files(table: Table, directory: str, attempt_id: str, *, warehouse_locations: Sequence[str]) -> int:
    """Deletes the files below ``directory``, the directory of the attempt, and returns how many it deleted.

    A directory that is not inside the warehouse, that is not clean, or whose last segment is not the attempt's id is
    refused with ``WORKER_ERROR`` before anything is listed: it would be a bug of the host, and the host must not delete
    on the strength of one. A directory that does not exist has nothing in it.
    """
    _checked(attempt_id)
    directory = directory.rstrip("/")
    if directory.rpartition("/")[2] != attempt_id or not inside(directory, warehouse_locations):
        raise Failure(WORKER_ERROR)
    return _delete_below(table, directory)


def _delete_below(table: Table, directory: str) -> int:
    from pyarrow.fs import FileSelector, FileType  # type: ignore[import-untyped]
    from pyiceberg.io.pyarrow import PyArrowFileIO

    io = table.io
    if not isinstance(io, PyArrowFileIO):
        # The file IO of ADLS is fsspec's; its listing is not written until it can be tried against that store
        # (G035-044).
        raise Failure(WORKER_ERROR)
    scheme, netloc, path = io.parse_location(directory, io.properties)
    fs = io.fs_by_scheme(scheme, netloc)
    removed = 0
    for info in fs.get_file_info(FileSelector(path, recursive=True, allow_not_found=True)):
        if info.type == FileType.File:
            fs.delete_file(info.path)
            removed += 1
    return removed
