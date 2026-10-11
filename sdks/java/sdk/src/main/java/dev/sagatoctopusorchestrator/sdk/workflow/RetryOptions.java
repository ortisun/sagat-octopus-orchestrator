package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;

/** The retry policy of a node, as {@code workflow.schema.json} names it. Fields that are not set are left out of the document. */
public final class RetryOptions {
    /** How the wait between attempts grows. */
    public enum Strategy {
        FIXED,
        EXPONENTIAL;

        String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    private Integer maxAttempts;
    private Long initialDelayMs;
    private Long maxDelayMs;
    private Strategy strategy;
    private Boolean jitter;

    public static RetryOptions create() {
        return new RetryOptions();
    }

    public RetryOptions maxAttempts(int value) {
        this.maxAttempts = value;
        return this;
    }

    public RetryOptions initialDelayMs(long value) {
        this.initialDelayMs = value;
        return this;
    }

    public RetryOptions maxDelayMs(long value) {
        this.maxDelayMs = value;
        return this;
    }

    public RetryOptions strategy(Strategy value) {
        this.strategy = value;
        return this;
    }

    public RetryOptions jitter(boolean value) {
        this.jitter = value;
        return this;
    }

    ObjectNode toJson() {
        ObjectNode node = JsonNodeFactory.instance.objectNode();
        if (maxAttempts != null) {
            node.put("maxAttempts", maxAttempts.intValue());
        }
        if (initialDelayMs != null) {
            node.put("initialDelayMs", initialDelayMs.longValue());
        }
        if (maxDelayMs != null) {
            node.put("maxDelayMs", maxDelayMs.longValue());
        }
        if (strategy != null) {
            node.put("strategy", strategy.wire());
        }
        if (jitter != null) {
            node.put("jitter", jitter.booleanValue());
        }
        return node;
    }
}
