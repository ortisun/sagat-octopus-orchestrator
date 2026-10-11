package dev.sagatoctopusorchestrator.sdk.workflow;

import java.util.ArrayList;
import java.util.List;

/** The options of an activity step, as {@code workflow.schema.json} names them. Options that are not set are left out of the document. */
public final class StepOptions {
    RetryOptions retry;
    Long timeoutMs;
    String compensateWith;
    String reconcileWith;
    String idempotencyTemplate;
    List<String> secrets;
    RetryOptions compensationRetry;
    Long compensationTimeoutMs;
    Boolean irreversible;
    Boolean highRisk;
    String onError;

    public static StepOptions create() {
        return new StepOptions();
    }

    public StepOptions retry(RetryOptions value) {
        this.retry = value;
        return this;
    }

    public StepOptions timeoutMs(long value) {
        this.timeoutMs = value;
        return this;
    }

    /** The activity that undoes this step; it must be idempotent. */
    public StepOptions compensateWith(String value) {
        this.compensateWith = value;
        return this;
    }

    /** The activity that finds out whether this step happened: a reconciler. */
    public StepOptions reconcileWith(String value) {
        this.reconcileWith = value;
        return this;
    }

    public StepOptions idempotencyTemplate(String value) {
        this.idempotencyTemplate = value;
        return this;
    }

    /** Names of the secrets the worker may ask for while it holds a lease on this step. */
    public StepOptions secrets(List<String> value) {
        this.secrets = value == null ? null : new ArrayList<>(value);
        return this;
    }

    public StepOptions compensationRetry(RetryOptions value) {
        this.compensationRetry = value;
        return this;
    }

    public StepOptions compensationTimeoutMs(long value) {
        this.compensationTimeoutMs = value;
        return this;
    }

    /** The effect cannot be undone once it has happened. Cannot be combined with {@code compensateWith}. */
    public StepOptions irreversible(boolean value) {
        this.irreversible = value;
        return this;
    }

    /** An attempt whose outcome is unknown is never retried or compensated blindly. */
    public StepOptions highRisk(boolean value) {
        this.highRisk = value;
        return this;
    }

    /** The id of the node to go to when the step fails for good. */
    public StepOptions onError(String value) {
        this.onError = value;
        return this;
    }
}
