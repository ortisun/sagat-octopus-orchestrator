"""Small readers and checks shared by the client modules.

Ids that become path segments, the idempotency key, and the typed reads of an answer. An answer that does not have
the shape the contract promises is a ``PROTOCOL`` error, never a guess (``sdks/SPEC.md`` section 10).
"""

from __future__ import annotations

import re
import uuid
from collections.abc import Mapping
from datetime import datetime
from typing import Any

from ._json import parse_instant
from .errors import SagatError, invalid

UUID = re.compile(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", re.IGNORECASE)
MAX_I64 = (1 << 63) - 1
_KEY = re.compile(r"[\x21-\x7e]{8,160}")


def idempotency_key_of(given: str | None) -> str:
    """The key of one call that changes something: the caller's, or a fresh one made once per call.

    It is 8 to 160 printable ASCII characters. The caller keeps it for every retry of the call.
    """
    key = given if given is not None else str(uuid.uuid4())
    if not isinstance(key, str) or not _KEY.fullmatch(key):
        raise invalid("an idempotency key is 8 to 160 printable ASCII characters")
    return key


def run_id_of(value: str) -> str:
    """A run id is a UUID: nothing else is ever put in the path of a call about a run."""
    if not isinstance(value, str) or not UUID.fullmatch(value):
        raise invalid("a run id is a UUID")
    return value


def namespace_id_of(value: str) -> str:
    if not isinstance(value, str) or not UUID.fullmatch(value):
        raise invalid("a namespace id is a UUID")
    return value.lower()


def id_of(value: str, what: str) -> str:
    """An id of a schedule, a backfill, a batch job or a reset: a UUID, since it becomes a path segment."""
    if not isinstance(value, str) or not UUID.fullmatch(value):
        raise invalid(f"{what} is a UUID")
    return value.lower()


def positive_int(value: int, what: str, minimum: int = 1) -> int:
    """An integer of at least ``minimum`` that fits the wire's 64 bits (a ``bool`` is not one)."""
    if not isinstance(value, int) or isinstance(value, bool) or not minimum <= value <= MAX_I64:
        raise invalid(f"{what} is an integer of at least {minimum}")
    return value


def object_of(value: Any, what: str) -> Mapping[str, Any]:
    if not isinstance(value, dict):
        raise SagatError("PROTOCOL", f"{what} is not an object")
    return value


def str_of(raw: Mapping[str, Any], key: str) -> str:
    value = raw.get(key)
    if not isinstance(value, str):
        raise SagatError("PROTOCOL", f"{key} is missing from the answer")
    return value


def opt_str(raw: Mapping[str, Any], key: str) -> str | None:
    value = raw.get(key)
    return value if isinstance(value, str) else None


def int_of(raw: Mapping[str, Any], key: str, fallback: int) -> int:
    value = raw.get(key)
    return value if isinstance(value, int) and not isinstance(value, bool) else fallback


def opt_number(raw: Mapping[str, Any], key: str) -> int | float | None:
    """A JSON number as it came (an ``int`` stays an ``int``), or ``None`` when it is absent or not a number."""
    value = raw.get(key)
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def revision(raw: Mapping[str, Any], key: str, minimum: int) -> int:
    value = raw.get(key)
    if not isinstance(value, int) or isinstance(value, bool) or not minimum <= value <= MAX_I64:
        raise SagatError("PROTOCOL", f"{key} is invalid in the answer")
    return value


def instant_of(raw: Mapping[str, Any], key: str) -> datetime:
    value = parse_instant(raw.get(key))
    if value is None:
        raise SagatError("PROTOCOL", f"{key} is missing from the answer")
    return value
