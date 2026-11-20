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
    OUTCOME_UNKNOWN;

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
