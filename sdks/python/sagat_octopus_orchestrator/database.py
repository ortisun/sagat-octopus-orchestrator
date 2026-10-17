"""The managed database wire contract. Values and bindings are explicit; server policy is authoritative."""

from __future__ import annotations

import json
import math
import re
from typing import Any, Literal, TypedDict, cast

from ._json import canonical_json
from .errors import SagatError, invalid

DatabaseType = Literal["null", "boolean", "integer", "decimal", "float", "text", "timestamp", "date", "binary", "json"]


class _OptionalRevision(TypedDict, total=False):
    revision: int


class ConnectionRef(_OptionalRevision):
    """A scoped logical name, optionally fixed to an immutable revision."""

    name: str


class DatabaseValue(TypedDict):
    """Integer/decimal values are exact strings, binary is base64, timestamp/date are text."""

    type: DatabaseType
    value: Any


class _LiteralParameter(TypedDict):
    literal: DatabaseValue


class _ExpressionParameter(TypedDict):
    type: DatabaseType
    expression: str


DatabaseParameter = _LiteralParameter | _ExpressionParameter


class DatabaseLimits(TypedDict, total=False):
    """Optional tighter limits on the execution budget, using canonical IR field names."""

    maxStatementBytes: int
    maxParameters: int
    maxParameterBytes: int
    maxRows: int
    maxResultBytes: int
    timeoutMs: int
    connectTimeoutMs: int
    poolSize: int
    idleTimeoutMs: int


_TYPES = {"null", "boolean", "integer", "decimal", "float", "text", "timestamp", "date", "binary", "json"}
_CEILINGS = {
    "maxStatementBytes": 65536,
    "maxParameters": 256,
    "maxParameterBytes": 262144,
    "maxRows": 10000,
    "maxResultBytes": 4194304,
    "timeoutMs": 30000,
    "connectTimeoutMs": 10000,
    "poolSize": 16,
    "idleTimeoutMs": 60000,
}


def _integer(value: Any, maximum: int) -> bool:
    return isinstance(value, int) and not isinstance(value, bool) and 1 <= value <= maximum


def _parameter(p: Any) -> bool:
    if not isinstance(p, dict):
        return False
    if "literal" not in p:
        return (
            p.keys() == {"type", "expression"}
            and isinstance(p["type"], str)
            and p["type"] in _TYPES
            and isinstance(p["expression"], str)
            and bool(p["expression"])
        )
    v = p["literal"]
    if p.keys() != {"literal"} or not isinstance(v, dict) or v.keys() != {"type", "value"}:
        return False
    kind, value = v["type"], v["value"]
    if kind == "null":
        return value is None
    if kind == "boolean":
        return isinstance(value, bool)
    if kind == "float":
        return isinstance(value, (int, float)) and not isinstance(value, bool) and math.isfinite(value)
    if kind == "integer":
        return isinstance(value, str) and len(value) <= 40 and re.fullmatch(r"-?[0-9]+", value) is not None
    if kind == "decimal":
        return (
            isinstance(value, str) and len(value) <= 16384 and re.fullmatch(r"-?[0-9]+(?:\.[0-9]+)?", value) is not None
        )
    if kind == "text":
        return isinstance(value, str) and "\0" not in value
    if kind in ("timestamp", "date", "binary"):
        return isinstance(value, str)
    return bool(kind == "json")  # canonical_json validates the JSON; SQL conversion remains server-side.


def database_node(node_id: str, options: dict[str, Any]) -> dict[str, Any]:
    """Validate authoring shape and capture a snapshot without exposing rejected values in errors."""
    try:
        ref = options["connectionRef"]
        if (
            not isinstance(ref, dict)
            or not ref.keys() <= {"name", "revision"}
            or not isinstance(ref.get("name"), str)
            or not re.fullmatch(r"[a-z][a-z0-9-]{1,61}[a-z0-9]", ref["name"])
            or "--" in ref["name"]
            or ("revision" in ref and not _integer(ref["revision"], 2147483647))
        ):
            raise ValueError
        statement = options["statement"]
        if (
            options["operation"] not in ("query", "execute")
            or not isinstance(statement, str)
            or not statement.strip()
            or "\0" in statement
            or len(statement.encode("utf-8")) > 65536
        ):
            raise ValueError
        parameters = options.get("parameters", [])
        if not isinstance(parameters, list) or len(parameters) > 256 or not all(map(_parameter, parameters)):
            raise ValueError
        limits = options.get("limits", {})
        if not isinstance(limits, dict) or any(not _integer(v, _CEILINGS.get(k, 0)) for k, v in limits.items()):
            raise ValueError
        if "timeoutMs" in options and not _integer(options["timeoutMs"], 60000):
            raise ValueError
        if "onError" in options and (not isinstance(options["onError"], str) or not options["onError"]):
            raise ValueError
        if len(canonical_json(parameters).encode("utf-8")) > 262144:
            raise ValueError
        return cast(dict[str, Any], json.loads(canonical_json({"id": node_id, "type": "database", **options})))
    except (ValueError, TypeError, OverflowError, SagatError):
        raise invalid("invalid managed database operation") from None
