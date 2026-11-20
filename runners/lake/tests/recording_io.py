"""A FileIO that records that PyIceberg built it (feature 035, G035-041).

PyIceberg builds the file IO of a table from the catalog's properties overridden by the table's own, and imports the
class named by ``py-io-impl``. A table that names this class shows it: the class is built, and the properties it was
built with are kept for the test to read.
"""

from __future__ import annotations

from pyiceberg.io.pyarrow import PyArrowFileIO
from pyiceberg.typedef import Properties

BUILT: list[dict[str, str]] = []


class RecordingFileIO(PyArrowFileIO):
    def __init__(self, properties: Properties) -> None:
        BUILT.append(dict(properties))
        super().__init__(properties)
