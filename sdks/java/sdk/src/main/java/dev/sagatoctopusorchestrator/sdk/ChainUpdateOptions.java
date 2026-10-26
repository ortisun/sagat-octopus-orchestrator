package dev.sagatoctopusorchestrator.sdk;

/** Options that identify one chain-scoped durable state update. */
public final class ChainUpdateOptions {
    private String updateKey;
    private String expectedRunId;
    private Long expectedStateRevision;
    private String traceparent;

    public static ChainUpdateOptions create(String updateKey) {
        return new ChainUpdateOptions().updateKey(updateKey);
    }

    public ChainUpdateOptions updateKey(String value) { this.updateKey = value; return this; }
    public ChainUpdateOptions expectedRunId(String value) { this.expectedRunId = value; return this; }
    public ChainUpdateOptions expectedStateRevision(Long value) { this.expectedStateRevision = value; return this; }
    public ChainUpdateOptions traceparent(String value) { this.traceparent = value; return this; }

    public String updateKey() { return updateKey; }
    public String expectedRunId() { return expectedRunId; }
    public Long expectedStateRevision() { return expectedStateRevision; }
    public String traceparent() { return traceparent; }
}
