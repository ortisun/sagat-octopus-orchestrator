package dev.sagatoctopusorchestrator.sdk;

/** The canonical error codes of the SDKs (sdks/SPEC.md section 2). */
public enum ErrorCode {
    BAD_REQUEST,
    UNAUTHENTICATED,
    FORBIDDEN,
    NOT_FOUND,
    CONFLICT,
    VALIDATION_FAILED,
    RATE_LIMITED,
    QUOTA_EXCEEDED,
    UNAVAILABLE,
    INTERNAL,
    STALE_ATTEMPT,
    IDEMPOTENCY_KEY_REUSED,
    NOT_BOUND,
    SECRET_NOT_USABLE,
    UNSUPPORTED_PROTOCOL,
    TRANSPORT,
    CANCELLED,
    PROTOCOL,
    PAYLOAD_TOO_LARGE,
    ICEBERG_TABLE_NOT_FOUND,
    ICEBERG_UNSUPPORTED_TABLE,
    ICEBERG_LOCATION_NOT_ALLOWED,
    ICEBERG_SCHEMA_MISMATCH,
    ICEBERG_DUPLICATE_KEY,
    ICEBERG_SCOPE_VIOLATION,
    ICEBERG_COMMIT_CONFLICT,
    ICEBERG_REVERT_BLOCKED,
    ICEBERG_LIMIT_EXCEEDED,
    CATALOG_UNAVAILABLE,
    STORAGE_UNAVAILABLE,
    DESTINATION_KEY_CONSTRAINT_MISSING,
    DESTINATION_SCHEMA_MISMATCH,
    DESTINATION_DEDUPLICATION_NOT_CONFIRMED,
    DESTINATION_OBJECT_CONFLICT,
    DESTINATION_DELIVERY_BOUND_EXCEEDED,
    DESTINATION_SUBSCRIPTION_SHARED,
    DESTINATION_AUTH_FAILED,
    DESTINATION_UNAVAILABLE,
    DESTINATION_SUBSCRIPTION_UNUSABLE,
    DESTINATION_FAILED,
    LAKE_HOST_PROTOCOL,
    LAKE_SANDBOX_UNAVAILABLE,
    CLASSIFICATION_EGRESS_REFUSED,
    OUTCOME_UNKNOWN,
    CAPABILITY_NOT_ACTIVE,
    SIGNAL_NOT_DECLARED,
    SIGNAL_INBOX_FULL,
    HISTORY_HEADROOM,
    RUN_CLOSING,
    UPDATE_PENDING,
    UPDATE_BACKLOG,
    RUN_NOT_MUTABLE,
    QUERY_TIMEOUT,
    QUERY_COMMAND,
    QUERY_UNAVAILABLE,
    MOVE_TARGET_INVALID,
    RESULT_SCHEMA_VIOLATION,
    RUN_ENDED,
    RUN_CONTINUED,
    RUN_BUSY,
    NO_TARGET_WORKER,
    REPLAY_MISMATCH,
    ENVELOPE_MISMATCH,
    MATCHED,
    REPLAY_CHECK_ABANDONED,
    WORKFLOW_FAILED,
    NONDETERMINISM,
    CHAIN_MISMATCH,
    NONDETERMINISM_RISK,
    COMMAND_OUTSIDE_ENVELOPE,
    COMMAND_INVALID,
    VALIDATOR_COMMAND,
    WORKFLOW_TASK_FAILED,
    ACTIVATION_DEADLINE,
    WORKFLOW_TASK_TIMED_OUT,
    COMMAND_VALUE_RELEASED,
    COMMAND_VALUE_LIMIT,
    COMMAND_LIMIT,
    OUTSTANDING_OBLIGATIONS,
    UNHANDLED_EVENTS,
    CLOSING_WINDOW_COMMAND,
    HISTORY_LIMIT,
    RETAINED_BYTES_LIMIT,
    STORAGE_QUOTA_EXCEEDED,
    EXTERNAL_CONTENT_NOT_INSPECTED,
    COMMAND_BATCH_LIMIT_EXCEEDED,
    INLINE_VALUES_EXCEEDED,
    INCONCLUSIVE_REDACTED;

    /** What to do about this code: the fixed sentence of the catalog. */
    public String remediation() {
        return ErrorCatalog.entry(this).remediation();
    }

    /** Whether the same call may be made again, for an error the server did not classify itself. */
    public boolean retryableByDefault() {
        return ErrorCatalog.entry(this).retryable();
    }

    /** The code an error envelope names, if it is one of the REST codes of the catalog; null otherwise. */
    static ErrorCode fromEnvelope(String value) {
        if (value == null) {
            return null;
        }
        for (ErrorCatalog.Entry entry : ErrorCatalog.entries()) {
            if ("rest".equals(entry.origin()) && entry.code().name().equals(value)) {
                return entry.code();
            }
        }
        return null;
    }
}
