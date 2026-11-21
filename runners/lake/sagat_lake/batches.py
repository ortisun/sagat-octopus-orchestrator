"""The mapped rows of an attempt as Arrow record batches (feature 035: T035-057, D035-033, D035-098).

``mapping.map_record`` gives a row of Python values by column; PyIceberg writes Arrow. The one thing between them that
is not a plain conversion is the uuid: Arrow stores it as an extension type it cannot build from Python values, but it
builds the 16 bytes the type is stored as from a ``uuid.UUID``. So the rows are built against a *storage* schema, where
every uuid (in a struct, a list or a map included) is a fixed binary of 16, and the batch is then cast to the table's
own Arrow schema, which carries the uuid type and the field ids PyIceberg checks. A table written this way and scanned
back gives the values that went in (``tests/test_batches.py`` does it for a row of every Iceberg type, with the uuid in
every position).

The batches are bounded in rows and in bytes. The bytes are not the size of the rows (Python does not say) but what the
reader has spent on them: ``meter`` returns the total the reader has read so far, and a batch is cut when it has taken
``max_bytes`` of it. A row past the limit is a batch of one.
"""

from __future__ import annotations

from collections.abc import Callable, Iterable, Iterator, Mapping
from typing import Any

import pyarrow as pa  # type: ignore[import-untyped]
from pyiceberg.io.pyarrow import schema_to_pyarrow
from pyiceberg.schema import Schema
from pyiceberg.types import IcebergType, ListType, MapType, StructType

from .failures import Failure
from .mapping import SCHEMA_MISMATCH

DEFAULT_ROWS = 1_024
DEFAULT_BYTES = 32 * 1024 * 1024


def to_batches(
    schema: Schema,
    rows: Iterable[Mapping[str, Any]],
    *,
    meter: Callable[[], int],
    max_rows: int = DEFAULT_ROWS,
    max_bytes: int = DEFAULT_BYTES,
) -> Iterator[pa.RecordBatch]:
    """``rows`` (as ``map_record`` gives them) in record batches of the Arrow schema of ``schema``, in order.

    Rows are taken as they are needed: a batch is built when it is full, and the next row is not read before.
    """
    if max_rows < 1 or max_bytes < 1:
        raise ValueError("a batch limit is at least one")
    target = schema_to_pyarrow(schema)
    staging = pa.schema([_storage_field(field) for field in target])
    pending: list[Mapping[str, Any]] = []
    started = meter()
    for row in rows:
        for field in schema.fields:
            _check(field.field_type, row[field.name], field.name)
        pending.append(row)
        if len(pending) >= max_rows or meter() - started >= max_bytes:
            yield from _batches(pending, staging, target)
            pending = []
            started = meter()
    if pending:
        yield from _batches(pending, staging, target)


def _batches(rows: list[Mapping[str, Any]], staging: pa.Schema, target: pa.Schema) -> list[pa.RecordBatch]:
    staged = pa.RecordBatch.from_pylist(rows, schema=staging)
    return list(pa.Table.from_batches([staged]).cast(target).to_batches())


def _check(kind: IcebergType, value: Any, path: str) -> None:
    """Refuses a null that PyIceberg 0.12 would write as something else, wherever it sits in the row.

    A null for a list of structs or a map with struct values is refused: PyIceberg rebuilds such an array without its
    validity (``ArrowProjectionVisitor.list`` and ``.map``), so the null would be written as an empty list or an empty
    map (G035-046). The refusal is wider than what was measured (a null that sits deeper survived), because a refusal
    that is too wide costs a failed step and one that is too narrow costs a value.
    """
    if value is None:
        if _loses_a_null(kind):
            raise Failure(SCHEMA_MISMATCH, column=path[-128:], reason="TYPE")
    elif isinstance(kind, StructType):
        for field in kind.fields:
            _check(field.field_type, value.get(field.name), f"{path}.{field.name}")
    elif isinstance(kind, ListType):
        for item in value:
            _check(kind.element_type, item, f"{path}[]")
    elif isinstance(kind, MapType):
        for _key, item in value:
            _check(kind.value_type, item, f"{path}{{}}")


def _loses_a_null(kind: IcebergType) -> bool:
    return (isinstance(kind, ListType) and isinstance(kind.element_type, StructType)) or (
        isinstance(kind, MapType) and isinstance(kind.value_type, StructType)
    )


def _storage_field(field: pa.Field) -> pa.Field:
    return pa.field(field.name, _storage_type(field.type), field.nullable)


def _storage_type(kind: pa.DataType) -> pa.DataType:
    """``kind`` with the uuid extension type replaced by the fixed binary it is stored as."""
    if isinstance(kind, pa.UuidType):
        return pa.binary(16)
    if pa.types.is_struct(kind):
        return pa.struct([_storage_field(kind.field(index)) for index in range(kind.num_fields)])
    if pa.types.is_large_list(kind):
        return pa.large_list(_storage_field(kind.value_field))
    if pa.types.is_map(kind):
        return pa.map_(_storage_field(kind.key_field), _storage_field(kind.item_field))
    return kind
