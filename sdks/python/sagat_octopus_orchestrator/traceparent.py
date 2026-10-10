"""W3C trace context (``sdks/SPEC.md`` section 4)."""

from __future__ import annotations

import re
import secrets

from .errors import SagatError

_PATTERN = re.compile(r"00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}")


def is_traceparent(value: str) -> bool:
    """True for a W3C ``traceparent`` of version 00 with a non-zero trace and parent id."""
    match = _PATTERN.fullmatch(value) if isinstance(value, str) else None
    return bool(match) and match is not None and match[1] != "0" * 32 and match[2] != "0" * 16


def check_traceparent(value: str) -> str:
    """Returns ``value``; raises ``BAD_REQUEST`` for a malformed ``traceparent``, before it is sent."""
    if not is_traceparent(value):
        raise SagatError("BAD_REQUEST", "traceparent is not a W3C trace context (00-<32 hex>-<16 hex>-<2 hex>)")
    return value


def trace_id_of(value: str | None) -> str | None:
    """The trace id of a valid ``traceparent``; ``None`` otherwise."""
    if not value or not is_traceparent(value):
        return None
    match = _PATTERN.fullmatch(value)
    return match[1] if match else None


def new_traceparent() -> str:
    """A fresh sampled trace context, for an application that has none."""
    return f"00-{secrets.token_hex(16)}-{secrets.token_hex(8)}-01"
