package dev.sagatoctopusorchestrator.sdk;

/** Options that identify one durable state update. */
public final class UpdateOptions {
    private String updateKey;
    private Long expectedStateRevision;
    private String traceparent;

    public static UpdateOptions create(String updateKey) {
        return new UpdateOptions().updateKey(updateKey);
    }

    /** One to 128 printable ASCII characters, preserved across retries. */
    public UpdateOptions updateKey(String value) { this.updateKey = value; return this; }

    /** Apply only at this committed state revision. */
    public UpdateOptions expectedStateRevision(Long value) { this.expectedStateRevision = value; return this; }

    /** An explicit W3C traceparent. */
    public UpdateOptions traceparent(String value) { this.traceparent = value; return this; }

    public String updateKey() { return updateKey; }
    public Long expectedStateRevision() { return expectedStateRevision; }
    public String traceparent() { return traceparent; }
}
