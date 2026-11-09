"""The Iceberg write step: an append of records to a table through the isolated Sagat lake runner.

Only a scoped logical connection name and revision appear in a workflow: no catalog, bucket, credential or secret.
The mode is ``append``; overwrite and upsert are not built yet. Shape validation only: the catalog, access, modes and
classification belong to the server.
"""

from __future__ import annotations

import json
import re
import unicodedata
from typing import Any, TypedDict, cast

from ._json import canonical_json
from .database import valid_connection_ref
from .errors import SagatError, invalid


class IcebergTable(TypedDict):
    """A table of an Iceberg catalog: a namespace of 1 to 8 levels and a name."""

    namespace: list[str]
    name: str


class _Inline(TypedDict):
    inline: str


class _Blobs(TypedDict):
    blobs: str


class _DatabaseResult(TypedDict):
    databaseResult: str


IcebergRecords = _Inline | _Blobs | _DatabaseResult
"""Where the records come from: exactly one of an expression that evaluates to a list of objects (``inline``), one
that evaluates to 1 to 32 blob descriptors (``blobs``), or the id of an earlier root-level database step
(``databaseResult``)."""


class IcebergMapping(TypedDict):
    """A field of a record (a JSON pointer, ``/customer/name``) and the column it is written to."""

    field: str
    column: str


class IcebergLimits(TypedDict, total=False):
    maxRecordBytes: int
    maxRecords: int
    timeoutMs: int


_LIMITS = {"maxRecordBytes": (1024, 2147483648), "maxRecords": (1, 100000000), "timeoutMs": (1000, 3600000)}
_OPTIONS = {
    "connectionRef",
    "table",
    "ref",
    "records",
    "mapping",
    "ignoreUnknownFields",
    "limits",
    "compensation",
    "onError",
}


def _plain(value: Any, limit: int) -> bool:
    """1 to ``limit`` bytes of UTF-8 without a control character: the platform's plain text."""
    return (
        isinstance(value, str)
        and len(value) > 0
        and len(value.encode("utf-8")) <= limit
        and not any(unicodedata.category(c) == "Cc" for c in value)
    )


def _part(value: Any) -> bool:
    """A level or a name of a table: plain, and not a path component of its own."""
    return _plain(value, 255) and value not in (".", "..")


def _pointer(value: Any) -> bool:
    """A JSON pointer into a record whose escapes are ``~0`` and ``~1``, of at most 16 segments."""
    if not _plain(value, 512) or not value.startswith("/"):
        return False
    segments = value[1:].split("/")
    return len(segments) <= 16 and all(segments) and re.search(r"~(?![01])", value) is None


def _bounded(value: Any, low: int, high: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and low <= value <= high


def iceberg_write_node(node_id: str, options: dict[str, Any]) -> dict[str, Any]:
    """Validate the authoring shape and capture a snapshot without exposing rejected values in errors."""
    try:
        if not set(options) <= _OPTIONS or not valid_connection_ref(options["connectionRef"]):
            raise ValueError
        table = options["table"]
        if (
            not isinstance(table, dict)
            or set(table) != {"namespace", "name"}
            or not isinstance(table["namespace"], list)
            or not 1 <= len(table["namespace"]) <= 8
            or not all(map(_part, table["namespace"]))
            or not _part(table["name"])
        ):
            raise ValueError
        if "ref" in options and not _plain(options["ref"], 255):
            raise ValueError
        records = options["records"]
        if not isinstance(records, dict) or len(records) != 1:
            raise ValueError
        source, value = next(iter(records.items()))
        if source in ("inline", "blobs"):
            # an expression of 1 to 1024 UTF-8 bytes, as everywhere else: the schema says 4096, the engine 1024
            if not isinstance(value, str) or not 1 <= len(value.encode("utf-8")) <= 1024:
                raise ValueError
        elif source != "databaseResult" or not _plain(value, 64):
            raise ValueError
        if "mapping" in options:
            mapping = options["mapping"]
            if not isinstance(mapping, list) or len(mapping) > 1024:
                raise ValueError
            fields: set[str] = set()
            columns: set[str] = set()
            for pair in mapping:
                if not isinstance(pair, dict) or set(pair) != {"field", "column"}:
                    raise ValueError
                if not _pointer(pair["field"]) or not _plain(pair["column"], 255):
                    raise ValueError
                # a field or a column twice is two rows of the mapping that disagree about one thing
                if pair["field"] in fields or pair["column"] in columns:
                    raise ValueError
                fields.add(pair["field"])
                columns.add(pair["column"])
        if "ignoreUnknownFields" in options and not isinstance(options["ignoreUnknownFields"], bool):
            raise ValueError
        if "limits" in options:
            limits = options["limits"]
            if not isinstance(limits, dict) or any(
                k not in _LIMITS or not _bounded(v, *_LIMITS[k]) for k, v in limits.items()
            ):
                raise ValueError
        if "compensation" in options and options["compensation"] != "revert":
            raise ValueError
        if "onError" in options and (not isinstance(options["onError"], str) or not options["onError"]):
            raise ValueError
        return cast(
            dict[str, Any],
            json.loads(canonical_json({"id": node_id, "type": "icebergWrite", "mode": "append", **options})),
        )
    except (ValueError, TypeError, KeyError, OverflowError, SagatError):
        raise invalid("invalid iceberg write") from None


__all__ = [
    "IcebergLimits",
    "IcebergMapping",
    "IcebergRecords",
    "IcebergTable",
    "iceberg_write_node",
]
