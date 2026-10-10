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
    PAYLOAD_TOO_LARGE;

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
