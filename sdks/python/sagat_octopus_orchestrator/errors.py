"""The one error an SDK raises (``sdks/SPEC.md`` section 2) and the canonical codes."""

from __future__ import annotations

from typing import Any, Literal, get_args

from ._catalog import CATALOG

ErrorCode = Literal[
    "BAD_REQUEST",
    "UNAUTHENTICATED",
    "FORBIDDEN",
    "NOT_FOUND",
    "CONFLICT",
    "VALIDATION_FAILED",
    "RATE_LIMITED",
    "QUOTA_EXCEEDED",
    "UNAVAILABLE",
    "INTERNAL",
    "STALE_ATTEMPT",
    "IDEMPOTENCY_KEY_REUSED",
    "NOT_BOUND",
    "SECRET_NOT_USABLE",
    "UNSUPPORTED_PROTOCOL",
    "TRANSPORT",
    "CANCELLED",
    "PROTOCOL",
    "PAYLOAD_TOO_LARGE",
]

ERROR_CODES: frozenset[str] = frozenset(get_args(ErrorCode))

_ENTRIES = {entry.code: entry for entry in CATALOG}
_FALLBACK_REMEDIATION = "Report the trace id to the operator."


def remediation_of(code: str) -> str:
    """What to do about a code: the fixed sentence of the catalog."""
    entry = _ENTRIES.get(code)
    return entry.remediation if entry else _FALLBACK_REMEDIATION


def retryable_by_default(code: str) -> bool:
    """The default ``retryable`` of a code, for an error the server did not classify itself."""
    entry = _ENTRIES.get(code)
    return entry.retryable if entry else False


def code_of_http_status(status: int) -> ErrorCode:
    """The code the catalog gives an HTTP status when the body carried no envelope."""
    for entry in CATALOG:
        if entry.origin == "rest" and status in entry.http:
            return entry.code  # type: ignore[return-value]  # the catalog test proves the codes are ErrorCodes
    return "INTERNAL" if status >= 500 else "BAD_REQUEST"


class SagatError(Exception):
    """A failure of a call to the platform, or of the SDK's own checks.

    ``message`` is safe to log: it never holds a payload, a token or a secret. ``remediation`` says what to
    do about ``code``. ``status`` is the HTTP status, or the gRPC status name, or ``None`` when no answer
    came; ``trace_id`` is empty when the server did not answer.
    """

    code: ErrorCode
    message: str
    status: int | str | None
    retryable: bool
    trace_id: str
    reason: str | None
    retry_after_ms: int | None
    remediation: str

    def __init__(
        self,
        code: ErrorCode,
        message: str,
        *,
        status: int | str | None = None,
        retryable: bool | None = None,
        trace_id: str = "",
        reason: str | None = None,
        retry_after_ms: int | None = None,
    ) -> None:
        super().__init__(message)
        self.code = code
        self.message = message
        self.status = status
        self.retryable = retryable_by_default(code) if retryable is None else retryable
        self.trace_id = trace_id
        self.reason = reason
        self.retry_after_ms = retry_after_ms
        self.remediation = remediation_of(code)

    def to_dict(self) -> dict[str, Any]:
        """The fields that are compared across SDKs (the message is the SDK's own words)."""
        return {
            "code": self.code,
            "status": self.status,
            "retryable": self.retryable,
            "traceId": self.trace_id,
            "reason": self.reason,
            "retryAfterMs": self.retry_after_ms,
            "remediation": self.remediation,
            "message": self.message,
        }

    def __reduce__(self) -> tuple[Any, tuple[Any, ...]]:
        return (_rebuild, (self.to_dict(),))

    def __repr__(self) -> str:
        return (
            f"SagatError(code={self.code!r}, status={self.status!r}, "
            f"retryable={self.retryable!r}, trace_id={self.trace_id!r})"
        )


def _rebuild(fields: dict[str, Any]) -> SagatError:
    return SagatError(
        fields["code"],
        fields["message"],
        status=fields["status"],
        retryable=fields["retryable"],
        trace_id=fields["traceId"],
        reason=fields["reason"],
        retry_after_ms=fields["retryAfterMs"],
    )


def invalid(message: str) -> SagatError:
    """A failure the SDK raises about its own input, before anything is sent."""
    return SagatError("VALIDATION_FAILED", message)
