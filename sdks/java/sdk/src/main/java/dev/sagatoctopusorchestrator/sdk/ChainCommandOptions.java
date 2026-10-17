package dev.sagatoctopusorchestrator.sdk;

/** Options for a command against an observed chain head. */
public final class ChainCommandOptions {
    private String expectedRunId;
    private String idempotencyKey;
    private String traceparent;

    public static ChainCommandOptions create(String expectedRunId) { return new ChainCommandOptions().expectedRunId(expectedRunId); }
    public ChainCommandOptions expectedRunId(String value) { this.expectedRunId = value; return this; }
    public ChainCommandOptions idempotencyKey(String value) { this.idempotencyKey = value; return this; }
    public ChainCommandOptions traceparent(String value) { this.traceparent = value; return this; }
    public String expectedRunId() { return expectedRunId; }
    public String idempotencyKey() { return idempotencyKey; }
    public String traceparent() { return traceparent; }
}
