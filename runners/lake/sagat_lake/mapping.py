"""The lossless mapping of a record to the columns of a table (feature 035: FR-035-010, D035-091).

A value is stored as it is or the write is refused. Nothing is rounded, truncated, wrapped, coerced, given a zone it
did not have or stripped of the one it had. A refusal is ``ICEBERG_SCHEMA_MISMATCH`` with the column (the path of a
nested one, ``customer.address.zip``) and a reason of the closed set below. It never carries the value:
``failures.Failure`` has no free text, so a value cannot travel in one.

The table, by Iceberg type (``docs/guides/iceberg-type-mapping.md`` publishes the same text):

=================  ==========================================================================================
boolean            ``true`` or ``false``
int, long          a JSON integer, or a string of digits for a value a JSON number cannot hold in every
                   language. A fraction, text or a boolean is ``TYPE``; outside the width, ``OVERFLOW``.
float, double      a finite JSON number, and also NaN and the infinities, which only these columns can hold.
                   A finite value past the largest float32 is ``OVERFLOW`` in a float column.
decimal(p, s)      a string of digits with an optional point, or an exact JSON number. A digit that would be
                   rounded away is ``SCALE_OVERFLOW`` (trailing zeros carry nothing and are accepted); more
                   digits than the precision is ``OVERFLOW``. A float is ``TYPE``: it is already inexact.
date               ``YYYY-MM-DD``
time               ``HH:MM:SS[.f]`` with no zone: a zone is ``ZONE``
timestamp          ``YYYY-MM-DDTHH:MM:SS[.f]`` with no zone; a zone is ``ZONE`` (it would be dropped)
timestamptz        the same with ``Z`` or an offset, stored as UTC; no zone is ``ZONE`` (it would be guessed)
                   Microseconds. A digit past the sixth that is not zero is ``SCALE_OVERFLOW``.
string             a JSON string
uuid               the canonical form of 36 characters
binary, fixed(n)   the typed cell ``{"type": "binary", "value": "<padded base64>"}`` of feature 017. A bare
                   string is ``TYPE``, and so is a fixed value of another length: nothing is padded or cut.
struct             a JSON object, field by field; a key the table does not have is ``UNKNOWN_FIELD``, always
list               a JSON array, element by element
map                an array of ``{"key": k, "value": v}``, or an object when the keys are strings; the same
                   key twice is ``TYPE``
=================  ==========================================================================================

A missing optional column is null; a missing required one is ``MISSING_REQUIRED``; a null in a required one is
``NULL_IN_REQUIRED``. A field of the record with no column is ``UNKNOWN_FIELD`` unless the step ignores unknown fields,
which are then dropped before any check and counted. The mapping of the step reads a field by JSON pointer and writes a
column; a field it does not name is not written and is not unknown.

The records are read with ``json.loads(line, parse_float=Decimal)`` so that a number keeps the digits it was written
with.
"""

from __future__ import annotations

import base64
import binascii
import datetime
import math
import re
import struct
import uuid
from collections.abc import Mapping, Sequence
from decimal import Decimal, InvalidOperation, localcontext
from typing import Any

from pyiceberg.schema import Schema
from pyiceberg.types import (
    BinaryType,
    BooleanType,
    DateType,
    DecimalType,
    DoubleType,
    FixedType,
    FloatType,
    IcebergType,
    IntegerType,
    ListType,
    LongType,
    MapType,
    PrimitiveType,
    StringType,
    StructType,
    TimestampType,
    TimestamptzType,
    TimeType,
    UUIDType,
)

from .failures import Failure

SCHEMA_MISMATCH = "ICEBERG_SCHEMA_MISMATCH"
_MAX_COLUMN = 128

_INT_WIDTH = {"int": (-(2**31), 2**31 - 1), "long": (-(2**63), 2**63 - 1)}
_FLOAT32_MAX = 3.4028234663852886e38
_DIGITS = re.compile(r"-?[0-9]+")
_DECIMAL = re.compile(r"-?[0-9]+(\.[0-9]+)?")
_DATE = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}")
_TIME = re.compile(r"([0-9]{2}):([0-9]{2}):([0-9]{2})(\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})?")
_TIMESTAMP = re.compile(
    r"([0-9]{4}-[0-9]{2}-[0-9]{2})[T ]([0-9]{2}:[0-9]{2}:[0-9]{2})(\.[0-9]+)?(Z|[+-][0-9]{2}:[0-9]{2})?"
)
_UUID = re.compile(r"[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}")


def map_record(
    schema: Schema,
    record: object,
    *,
    mapping: Sequence[tuple[str, str]] = (),
    ignore_unknown: bool = False,
) -> tuple[dict[str, Any], int]:
    """The row of ``record`` by column name, and how many unknown fields were dropped.

    ``mapping`` is the step's list of (JSON pointer, column). Empty, a field of the record is a column of the same name.
    """
    if not isinstance(record, Mapping):
        raise _mismatch(None, "TYPE")
    columns = {field.name: field for field in schema.fields}
    if mapping:
        incoming: dict[str, Any] = {}
        for pointer, column in mapping:
            if column not in columns:
                raise _mismatch(column, "UNKNOWN_FIELD")
            present, value = _read_pointer(record, pointer)
            if present:
                incoming[column] = value
        dropped = 0
    else:
        unknown = sorted(name for name in record if name not in columns)
        if unknown and not ignore_unknown:
            raise _mismatch(unknown[0], "UNKNOWN_FIELD")
        incoming = {name: value for name, value in record.items() if name in columns}
        dropped = len(unknown)
    row: dict[str, Any] = {}
    for name, field in columns.items():
        if name in incoming:
            row[name] = _convert(field.field_type, incoming[name], name, field.required)
        elif field.required:
            raise _mismatch(name, "MISSING_REQUIRED")
        else:
            row[name] = None
    return row, dropped


def _mismatch(column: str | None, reason: str) -> Failure:
    if column is not None:
        column = "".join(c if c.isprintable() else "?" for c in column)[-_MAX_COLUMN:] or "?"
    return Failure(SCHEMA_MISMATCH, column=column, reason=reason)


def _read_pointer(record: Mapping[str, Any], pointer: str) -> tuple[bool, Any]:
    """The value at an RFC 6901 pointer, and whether there was one."""
    current: Any = record
    for segment in pointer.removeprefix("/").split("/"):
        name = segment.replace("~1", "/").replace("~0", "~")
        if isinstance(current, Mapping):
            if name not in current:
                return False, None
            current = current[name]
        elif isinstance(current, list) and name.isdigit() and int(name) < len(current):
            current = current[int(name)]
        else:
            return False, None
    return True, current


def _convert(kind: IcebergType, value: Any, path: str, required: bool) -> Any:
    if value is None:
        if required:
            raise _mismatch(path, "NULL_IN_REQUIRED")
        return None
    if isinstance(kind, BooleanType):
        if isinstance(value, bool):
            return value
        raise _mismatch(path, "TYPE")
    if isinstance(kind, IntegerType):
        return _integer(value, "int", path)
    if isinstance(kind, LongType):
        return _integer(value, "long", path)
    if isinstance(kind, FloatType):
        return _floating(value, narrow=True, path=path)
    if isinstance(kind, DoubleType):
        return _floating(value, narrow=False, path=path)
    if isinstance(kind, DecimalType):
        return _decimal(value, kind.precision, kind.scale, path)
    if isinstance(kind, StringType):
        return _string(value, path)
    if isinstance(kind, DateType):
        return _date(value, path)
    if isinstance(kind, TimeType):
        return _time(value, path)
    if isinstance(kind, TimestampType):
        return _timestamp(value, zoned=False, path=path)
    if isinstance(kind, TimestamptzType):
        return _timestamp(value, zoned=True, path=path)
    if isinstance(kind, UUIDType):
        return _uuid(value, path)
    if isinstance(kind, FixedType):
        return _binary(value, path, length=len(kind))
    if isinstance(kind, BinaryType):
        return _binary(value, path, length=None)
    if isinstance(kind, StructType):
        return _struct(kind, value, path)
    if isinstance(kind, ListType):
        return _list(kind, value, path)
    if isinstance(kind, MapType):
        return _map(kind, value, path)
    # A type this table does not cover is refused rather than passed through: nothing is stored that was not mapped.
    raise _mismatch(path, "TYPE")


def _integer(value: Any, width: str, path: str) -> int:
    if isinstance(value, bool):
        raise _mismatch(path, "TYPE")
    if isinstance(value, int):
        number = value
    elif isinstance(value, Decimal):
        if not value.is_finite() or value != value.to_integral_value():
            raise _mismatch(path, "TYPE")
        number = int(value)
    elif isinstance(value, float):
        if not value.is_integer() or abs(value) > 2**53:
            raise _mismatch(path, "TYPE")
        number = int(value)
    elif isinstance(value, str) and _DIGITS.fullmatch(value):
        number = int(value)
    else:
        raise _mismatch(path, "TYPE")
    low, high = _INT_WIDTH[width]
    if not low <= number <= high:
        raise _mismatch(path, "OVERFLOW")
    return number


def _floating(value: Any, *, narrow: bool, path: str) -> float:
    if isinstance(value, bool) or not isinstance(value, int | float | Decimal):
        raise _mismatch(path, "TYPE")
    try:
        number = float(value)
    except OverflowError:
        raise _mismatch(path, "OVERFLOW") from None
    if math.isinf(number) and not isinstance(value, float):
        raise _mismatch(path, "OVERFLOW")
    if narrow and math.isfinite(number):
        try:
            struct.pack("f", number)
        except OverflowError:
            raise _mismatch(path, "OVERFLOW") from None
        if abs(number) > _FLOAT32_MAX:
            raise _mismatch(path, "OVERFLOW")
    return number


def _decimal(value: Any, precision: int, scale: int, path: str) -> Decimal:
    if isinstance(value, bool):
        raise _mismatch(path, "TYPE")
    if isinstance(value, int):
        number = Decimal(value)
    elif isinstance(value, Decimal):
        number = value
    elif isinstance(value, str) and _DECIMAL.fullmatch(value):
        number = Decimal(value)
    else:
        raise _mismatch(path, "TYPE")
    if not number.is_finite():
        raise _mismatch(path, "TYPE")
    with localcontext() as context:
        context.prec = 200
        try:
            rounded = number.quantize(Decimal(1).scaleb(-scale))
        except InvalidOperation:
            raise _mismatch(path, "OVERFLOW") from None
        if rounded != number:
            raise _mismatch(path, "SCALE_OVERFLOW")
    if len(rounded.as_tuple().digits) > precision:
        raise _mismatch(path, "OVERFLOW")
    return rounded


def _string(value: Any, path: str) -> str:
    if not isinstance(value, str):
        raise _mismatch(path, "TYPE")
    try:
        value.encode("utf-8")
    except UnicodeEncodeError:
        raise _mismatch(path, "TYPE") from None
    return value


def _date(value: Any, path: str) -> datetime.date:
    if not isinstance(value, str) or not _DATE.fullmatch(value):
        raise _mismatch(path, "TYPE")
    try:
        return datetime.date.fromisoformat(value)
    except ValueError:
        raise _mismatch(path, "TYPE") from None


def _microseconds(fraction: str | None, path: str) -> int:
    """The microseconds of a fraction (``.123``), refusing a digit past the sixth that is not zero."""
    if not fraction:
        return 0
    digits = fraction[1:]
    if digits[6:].strip("0"):
        raise _mismatch(path, "SCALE_OVERFLOW")
    return int(digits[:6].ljust(6, "0"))


def _offset(text: str, path: str) -> datetime.timezone:
    if text == "Z":
        return datetime.UTC
    sign = -1 if text[0] == "-" else 1
    hours, minutes = int(text[1:3]), int(text[4:6])
    if hours > 23 or minutes > 59:
        raise _mismatch(path, "TYPE")
    return datetime.timezone(sign * datetime.timedelta(hours=hours, minutes=minutes))


def _time(value: Any, path: str) -> datetime.time:
    match = _TIME.fullmatch(value) if isinstance(value, str) else None
    if match is None:
        raise _mismatch(path, "TYPE")
    hour, minute, second, fraction, zone = match.groups()
    if zone is not None:
        raise _mismatch(path, "ZONE")
    micro = _microseconds(fraction, path)
    try:
        return datetime.time(int(hour), int(minute), int(second), micro)
    except ValueError:
        raise _mismatch(path, "TYPE") from None


def _timestamp(value: Any, *, zoned: bool, path: str) -> datetime.datetime:
    match = _TIMESTAMP.fullmatch(value) if isinstance(value, str) else None
    if match is None:
        raise _mismatch(path, "TYPE")
    day, clock, fraction, zone = match.groups()
    if zoned != (zone is not None):
        # no zone into a zoned column would guess one; a zone into a column without one would drop it
        raise _mismatch(path, "ZONE")
    micro = _microseconds(fraction, path)
    try:
        moment = datetime.datetime.fromisoformat(f"{day}T{clock}").replace(microsecond=micro)
    except ValueError:
        raise _mismatch(path, "TYPE") from None
    if zone is None:
        return moment
    try:
        return moment.replace(tzinfo=_offset(zone, path)).astimezone(datetime.UTC)
    except OverflowError:
        raise _mismatch(path, "OVERFLOW") from None


def _uuid(value: Any, path: str) -> uuid.UUID:
    if not isinstance(value, str) or not _UUID.fullmatch(value):
        raise _mismatch(path, "TYPE")
    return uuid.UUID(value)


def _binary(value: Any, path: str, *, length: int | None) -> bytes:
    if not isinstance(value, Mapping) or set(value) != {"type", "value"} or value["type"] != "binary":
        raise _mismatch(path, "TYPE")
    encoded = value["value"]
    if not isinstance(encoded, str):
        raise _mismatch(path, "TYPE")
    try:
        data = base64.b64decode(encoded, validate=True)
    except (binascii.Error, ValueError):
        raise _mismatch(path, "TYPE") from None
    if length is not None and len(data) != length:
        raise _mismatch(path, "TYPE")
    return data


def _struct(kind: StructType, value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, Mapping):
        raise _mismatch(path, "TYPE")
    fields = {field.name: field for field in kind.fields}
    for name in sorted(value):
        if name not in fields:
            # A nested field the table does not have is always refused: ignoring unknown fields is for the record.
            raise _mismatch(f"{path}.{name}", "UNKNOWN_FIELD")
    out: dict[str, Any] = {}
    for name, field in fields.items():
        inner = f"{path}.{name}"
        if name in value:
            out[name] = _convert(field.field_type, value[name], inner, field.required)
        elif field.required:
            raise _mismatch(inner, "MISSING_REQUIRED")
        else:
            out[name] = None
    return out


def _list(kind: ListType, value: Any, path: str) -> list[Any]:
    if not isinstance(value, list):
        raise _mismatch(path, "TYPE")
    return [_convert(kind.element_type, item, f"{path}[]", kind.element_required) for item in value]


def _map(kind: MapType, value: Any, path: str) -> list[tuple[Any, Any]]:
    # A struct, a list or a map is a legal key in the format and an unhashable one in Python, so the same key twice
    # could not be looked for. Keys are primitive.
    if not isinstance(kind.key_type, PrimitiveType):
        raise _mismatch(path, "TYPE")
    if isinstance(value, Mapping):
        if not isinstance(kind.key_type, StringType):
            raise _mismatch(path, "TYPE")
        pairs = list(value.items())
    elif isinstance(value, list) and all(isinstance(p, Mapping) and set(p) == {"key", "value"} for p in value):
        pairs = [(pair["key"], pair["value"]) for pair in value]
    else:
        raise _mismatch(path, "TYPE")
    seen: set[Any] = set()
    out: list[tuple[Any, Any]] = []
    for key, item in pairs:
        stored_key = _convert(kind.key_type, key, f"{path}{{}}", True)
        if stored_key in seen:
            raise _mismatch(path, "TYPE")
        seen.add(stored_key)
        out.append((stored_key, _convert(kind.value_type, item, f"{path}{{}}", kind.value_required)))
    return out
