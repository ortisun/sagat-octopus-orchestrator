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
    "CAPABILITY_NOT_ACTIVE",
    "SIGNAL_NOT_DECLARED",
    "SIGNAL_INBOX_FULL",
    "HISTORY_HEADROOM",
    "RUN_CLOSING",
    "UPDATE_PENDING",
    "UPDATE_BACKLOG",
    "RUN_NOT_MUTABLE",
    "QUERY_TIMEOUT",
    "QUERY_COMMAND",
    "QUERY_UNAVAILABLE",
    "MOVE_TARGET_INVALID",
    "RESULT_SCHEMA_VIOLATION",
    "RUN_ENDED",
    "RUN_CONTINUED",
    "RUN_BUSY",
    "NO_TARGET_WORKER",
    "REPLAY_MISMATCH",
    "ENVELOPE_MISMATCH",
    "MATCHED",
    "REPLAY_CHECK_ABANDONED",
    "WORKFLOW_FAILED",
    "STALE_ATTEMPT",
    "IDEMPOTENCY_KEY_REUSED",
    "NOT_BOUND",
    "SECRET_NOT_USABLE",
    "UNSUPPORTED_PROTOCOL",
    "TRANSPORT",
    "CANCELLED",
    "PROTOCOL",
    "PAYLOAD_TOO_LARGE",
    "COMMAND_BATCH_LIMIT_EXCEEDED",
    "INLINE_VALUES_EXCEEDED",
    "INCONCLUSIVE_REDACTED",
    "ICEBERG_TABLE_NOT_FOUND",
    "ICEBERG_UNSUPPORTED_TABLE",
    "ICEBERG_LOCATION_NOT_ALLOWED",
    "ICEBERG_SCHEMA_MISMATCH",
    "ICEBERG_DUPLICATE_KEY",
    "ICEBERG_SCOPE_VIOLATION",
    "ICEBERG_COMMIT_CONFLICT",
    "ICEBERG_REVERT_BLOCKED",
    "ICEBERG_LIMIT_EXCEEDED",
    "CATALOG_UNAVAILABLE",
    "STORAGE_UNAVAILABLE",
    "DESTINATION_KEY_CONSTRAINT_MISSING",
    "DESTINATION_SCHEMA_MISMATCH",
    "DESTINATION_DEDUPLICATION_NOT_CONFIRMED",
    "DESTINATION_OBJECT_CONFLICT",
    "DESTINATION_DELIVERY_BOUND_EXCEEDED",
    "DESTINATION_SUBSCRIPTION_SHARED",
    "DESTINATION_AUTH_FAILED",
    "DESTINATION_UNAVAILABLE",
    "DESTINATION_SUBSCRIPTION_UNUSABLE",
    "DESTINATION_FAILED",
    "LAKE_HOST_PROTOCOL",
    "LAKE_SANDBOX_UNAVAILABLE",
    "CLASSIFICATION_EGRESS_REFUSED",
    "OUTCOME_UNKNOWN",
    "NONDETERMINISM",
    "CHAIN_MISMATCH",
    "NONDETERMINISM_RISK",
    "COMMAND_OUTSIDE_ENVELOPE",
    "COMMAND_INVALID",
    "VALIDATOR_COMMAND",
    "WORKFLOW_TASK_FAILED",
    "ACTIVATION_DEADLINE",
    "WORKFLOW_TASK_TIMED_OUT",
    "COMMAND_VALUE_RELEASED",
    "COMMAND_VALUE_LIMIT",
    "COMMAND_LIMIT",
    "OUTSTANDING_OBLIGATIONS",
    "UNHANDLED_EVENTS",
    "CLOSING_WINDOW_COMMAND",
    "HISTORY_LIMIT",
    "RETAINED_BYTES_LIMIT",
    "STORAGE_QUOTA_EXCEEDED",
    "EXTERNAL_CONTENT_NOT_INSPECTED",
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
    field: str | None
    rule: str | None

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
        field: str | None = None,
        rule: str | None = None,
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
        # design 043: ``details.field`` (a JSON pointer whose segments are names the schema declares and array indices,
        # never a value) and ``details.rule`` (the closed keyword that failed), if the server sent them
        self.field = field
        self.rule = rule

    def to_dict(self) -> dict[str, Any]:
        """The fields that are compared across SDKs (the message is the SDK's own words)."""
        fields: dict[str, Any] = {
            "code": self.code,
            "status": self.status,
            "retryable": self.retryable,
            "traceId": self.trace_id,
            "reason": self.reason,
            "retryAfterMs": self.retry_after_ms,
            "remediation": self.remediation,
            "message": self.message,
        }
        # only when the server sent them, so the errors of every other refusal read as they always have
        if self.field is not None:
            fields["field"] = self.field
        if self.rule is not None:
            fields["rule"] = self.rule
        return fields

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
        field=fields.get("field"),
        rule=fields.get("rule"),
    )


def invalid(message: str) -> SagatError:
    """A failure the SDK raises about its own input, before anything is sent."""
    return SagatError("VALIDATION_FAILED", message)
