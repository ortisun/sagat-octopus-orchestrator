package dev.sagatoctopusorchestrator.sdk;

/** Options that identify one safely retryable approval decision. */
public final class ApprovalDecisionOptions {
    private String decisionKey;
    private String reasonCode;
    private String traceparent;

    public static ApprovalDecisionOptions create(String decisionKey) {
        return new ApprovalDecisionOptions().decisionKey(decisionKey);
    }

    /** One to 128 printable ASCII characters, reused only for the same decision. */
    public ApprovalDecisionOptions decisionKey(String value) {
        this.decisionKey = value;
        return this;
    }

    /** An optional machine-readable reason code. */
    public ApprovalDecisionOptions reasonCode(String value) {
        this.reasonCode = value;
        return this;
    }

    /** An explicit W3C traceparent. */
    public ApprovalDecisionOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    public String decisionKey() { return decisionKey; }
    public String reasonCode() { return reasonCode; }
    public String traceparent() { return traceparent; }
}
