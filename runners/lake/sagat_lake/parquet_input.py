"""The records of an attempt, read from a Parquet file the runner wrote (T035-057, D035-033, D035-063, D035-107).

A Parquet blob is a table of records: each row is one. The host reads it a row group and a batch at a time and turns
each cell into the JSON representation of the mapping table (``docs/guides/iceberg-type-mapping.md``): a date as
``YYYY-MM-DD``, a timestamp as text with its offset, a binary as the typed cell with padded base64, a decimal with its
digits, a map as an array of ``{"key", "value"}``. The record then goes through ``mapping.map_record`` like one from
an NDJSON line, so a single table decides what a value may become, whatever the file it came in.

The file is not trusted (D035-063):

* the length of the footer is read from the last eight bytes and bounded **before** the footer is parsed;
* the uncompressed size of each row group, which the footer states, is spent from the budget of the step **before**
  that row group is decoded, so the first byte over the limit stops the step without a row of it reaching the caller.
  A footer can lie about it; the memory of the container (``memory.max``) is what stops that;
* a column named twice, at any depth, is refused: a row is a dictionary and would silently keep one of them;
* a type the mapping has no form for (a duration, an interval, a union) is ``TYPE`` and names the column, and a
  nanosecond that would be lost is ``SCALE_OVERFLOW``, as it is in a string;
* a file that is not Parquet, or is cut, or has a footer that does not fit it, is ``TYPE`` without a column. Nothing
  here carries a value of the file.
"""

from __future__ import annotations

import base64
import datetime
import uuid
from collections.abc import Generator, Iterator
from decimal import Decimal
from pathlib import Path
from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]
import pyarrow.parquet as pq  # type: ignore[import-untyped]

from .compat import LIMIT_EXCEEDED
from .failures import Failure
from .mapping import SCHEMA_MISMATCH
from .records import Budget

DEFAULT_BATCH_ROWS = 1024
# The footer holds the schema and the statistics of every row group; the platform's files have a few kilobytes.
MAX_FOOTER_BYTES = 16 * 1024 * 1024
_MAGIC = b"PAR1"


def _mismatch(reason: str, column: str | None = None) -> Failure:
    return Failure(SCHEMA_MISMATCH, column=column, reason=reason)


def read_parquet(
    path: Path,
    budget: Budget,
    *,
    batch_rows: int = DEFAULT_BATCH_ROWS,
    max_footer_bytes: int = MAX_FOOTER_BYTES,
) -> Generator[dict[str, Any]]:
    """The rows of ``path`` in order, as records. A missing file is the caller's bug: ``FileNotFoundError``."""
    if batch_rows < 1:
        raise ValueError("a batch is at least one row")
    footer = _footer_length(path, max_footer_bytes)
    budget.spend(footer)
    try:
        parquet = pq.ParquetFile(path)
    except (pa.ArrowException, ValueError):
        raise _mismatch("TYPE") from None
    try:
        names = _columns(parquet.schema_arrow)
        metadata = parquet.metadata
        for index in range(metadata.num_row_groups):
            size = metadata.row_group(index).total_byte_size
            if size < 0:
                raise _mismatch("TYPE")
            budget.spend(size)
            yield from _row_group(parquet, index, names, batch_rows)
    finally:
        parquet.close()


def _footer_length(path: Path, limit: int) -> int:
    """The length of the footer the file says it has, if the file can have one; checked against ``limit``."""
    size = path.stat().st_size
    if size < 12:
        raise _mismatch("TYPE")
    with path.open("rb") as stream:
        head = stream.read(4)
        stream.seek(size - 8)
        tail = stream.read(8)
    length = int.from_bytes(tail[:4], "little")
    if head != _MAGIC or tail[4:] != _MAGIC or length == 0 or length > size - 12:
        raise _mismatch("TYPE")
    if length > limit:
        raise Failure(LIMIT_EXCEEDED)
    return length


def _columns(schema: pa.Schema) -> list[str]:
    """The names of the columns, if the mapping can hold every one of them and no name is twice at any depth."""
    names = list(schema.names)
    if len(set(names)) != len(names):
        raise _mismatch("TYPE")
    for field in schema:
        if not _holds(field.type):
            raise _mismatch("TYPE", field.name)
    return names


def _holds(kind: pa.DataType) -> bool:
    """Whether the mapping has a form for ``kind`` and no struct below it names a field twice."""
    types = pa.types
    if (
        types.is_null(kind)
        or types.is_boolean(kind)
        or types.is_integer(kind)
        or types.is_floating(kind)
        or types.is_decimal(kind)
        or types.is_string(kind)
        or types.is_large_string(kind)
        or types.is_binary(kind)
        or types.is_large_binary(kind)
        or types.is_fixed_size_binary(kind)
        or types.is_date(kind)
        or types.is_time(kind)
        or types.is_timestamp(kind)
    ):
        return True
    if types.is_dictionary(kind):
        return _holds(kind.value_type)
    if types.is_list(kind) or types.is_large_list(kind) or types.is_fixed_size_list(kind):
        return _holds(kind.value_type)
    if types.is_map(kind):
        return _holds(kind.key_type) and _holds(kind.item_type)
    if types.is_struct(kind):
        names = [kind.field(index).name for index in range(kind.num_fields)]
        return len(set(names)) == len(names) and all(_holds(kind.field(index).type) for index in range(kind.num_fields))
    return isinstance(kind, pa.UuidType)


def _row_group(parquet: pq.ParquetFile, index: int, names: list[str], batch_rows: int) -> Iterator[dict[str, Any]]:
    try:
        for batch in parquet.iter_batches(batch_size=batch_rows, row_groups=[index]):
            columns = []
            for position, name in enumerate(names):
                try:
                    columns.append(batch.column(position).to_pylist())
                except ValueError:
                    # a nanosecond that does not fit in a microsecond: pyarrow refuses to convert it without loss
                    raise _mismatch("SCALE_OVERFLOW", name) from None
            for row in range(batch.num_rows):
                yield {name: _cell(column[row], name) for name, column in zip(names, columns, strict=True)}
    except (pa.ArrowException, OSError) as error:
        if isinstance(error, FileNotFoundError):
            raise
        raise _mismatch("TYPE") from None


def _cell(value: Any, column: str) -> Any:
    """``value`` as the mapping table has it: JSON's own types, and text or a typed cell for the rest."""
    if value is None or isinstance(value, bool | int | float | str | Decimal):
        return value
    if isinstance(value, bytes):
        return {"type": "binary", "value": base64.b64encode(value).decode("ascii")}
    if isinstance(value, datetime.date | datetime.time):
        # a datetime is a date, and its isoformat keeps the offset the mapping needs to store it as UTC
        return value.isoformat()
    if isinstance(value, uuid.UUID):
        return str(value)
    if isinstance(value, dict):
        return {key: _cell(item, column) for key, item in value.items()}
    if isinstance(value, list):
        return [
            {"key": _cell(item[0], column), "value": _cell(item[1], column)}
            if isinstance(item, tuple)
            else _cell(item, column)
            for item in value
        ]
    raise _mismatch("TYPE", column)
