package dev.sagatoctopusorchestrator.sdk;

/** Options common to the calls that write: an idempotency key and a trace context. */
public final class CallOptions {
    private String idempotencyKey;
    private String traceparent;

    public static CallOptions create() {
        return new CallOptions();
    }

    /** 8 to 160 printable ASCII characters; generated per call when absent and reused by its retries. */
    public CallOptions idempotencyKey(String value) {
        this.idempotencyKey = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public CallOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    public String idempotencyKey() {
        return idempotencyKey;
    }

    public String traceparent() {
        return traceparent;
    }
}
