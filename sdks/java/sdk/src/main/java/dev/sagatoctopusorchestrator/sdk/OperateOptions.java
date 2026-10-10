package dev.sagatoctopusorchestrator.sdk;

/** Options of {@link SagatClient#operate} and {@link SagatClient#cancel}. */
public final class OperateOptions {
    /** The outcome an operator gives to a step ({@code RESOLVE_STEP}). */
    public enum Outcome {
        SUCCEEDED,
        FAILED
    }

    private String idempotencyKey;
    private String traceparent;
    private String stepId;
    private Outcome outcome;
    private String errorCode;
    private Long expectedRunVersion;

    public static OperateOptions create() {
        return new OperateOptions();
    }

    public OperateOptions idempotencyKey(String value) {
        this.idempotencyKey = value;
        return this;
    }

    public OperateOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    /** The step an operation is about (retry, skip, resolve). */
    public OperateOptions stepId(String value) {
        this.stepId = value;
        return this;
    }

    public OperateOptions outcome(Outcome value) {
        this.outcome = value;
        return this;
    }

    public OperateOptions errorCode(String value) {
        this.errorCode = value;
        return this;
    }

    /** Refuses the operation when the run has moved on: the {@code version} of a run you read. */
    public OperateOptions expectedRunVersion(Long value) {
        this.expectedRunVersion = value;
        return this;
    }

    public String idempotencyKey() {
        return idempotencyKey;
    }

    public String traceparent() {
        return traceparent;
    }

    public String stepId() {
        return stepId;
    }

    public Outcome outcome() {
        return outcome;
    }

    public String errorCode() {
        return errorCode;
    }

    public Long expectedRunVersion() {
        return expectedRunVersion;
    }
}
