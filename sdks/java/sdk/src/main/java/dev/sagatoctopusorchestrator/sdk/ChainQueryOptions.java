package dev.sagatoctopusorchestrator.sdk;

/** Options for resolving and querying a continuation chain head. */
public final class ChainQueryOptions {
    private String expectedRunId;
    private String traceparent;

    public static ChainQueryOptions create() {
        return new ChainQueryOptions();
    }

    /** Requires this run to remain the current head while the query is evaluated. */
    public ChainQueryOptions expectedRunId(String value) {
        this.expectedRunId = value;
        return this;
    }

    /** Sets an explicit W3C traceparent. */
    public ChainQueryOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    public String expectedRunId() {
        return expectedRunId;
    }

    public String traceparent() {
        return traceparent;
    }
}
