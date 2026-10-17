package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import dev.sagatoctopusorchestrator.sdk.Json;

/** The options of a {@code waitForEvent} node. */
public final class WaitForEventOptions {
    final String eventType;
    final String correlationExpr;
    final long timeoutMs;
    String onTimeout;
    Long lookbackMs;
    JsonNode outputSchema;

    private WaitForEventOptions(String eventType, String correlationExpr, long timeoutMs) {
        this.eventType = eventType;
        this.correlationExpr = correlationExpr;
        this.timeoutMs = timeoutMs;
    }

    /**
     * @param eventType       the type of event that wakes the wait
     * @param correlationExpr an expression over the run's data that yields the correlation key
     * @param timeoutMs       how long to wait, at least 1
     */
    public static WaitForEventOptions of(String eventType, String correlationExpr, long timeoutMs) {
        return new WaitForEventOptions(eventType, correlationExpr, timeoutMs);
    }

    /** The id of the node to go to when the wait times out. */
    public WaitForEventOptions onTimeout(String value) {
        this.onTimeout = value;
        return this;
    }

    /** An event that arrived up to this long before the wait started also wakes it (once). */
    public WaitForEventOptions lookbackMs(long value) {
        this.lookbackMs = value;
        return this;
    }

    /** The JSON Schema of the event payload carried by the satisfied wait. */
    public WaitForEventOptions outputSchema(Object value) {
        JsonNode node = Json.toTree(value);
        if (!node.isObject()) {
            throw dev.sagatoctopusorchestrator.sdk.SagatException.invalid("outputSchema is a JSON object");
        }
        this.outputSchema = node.deepCopy();
        return this;
    }
}
