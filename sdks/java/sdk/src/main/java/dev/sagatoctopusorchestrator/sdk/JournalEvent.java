package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/** One event of a run's journal. {@code raw} is the event as the server sent it. */
public record JournalEvent(JsonNode raw) {
    /** The event type, for example {@code step.completed}; null if absent. */
    public String eventType() {
        JsonNode value = raw.path("eventType");
        return value.isTextual() ? value.textValue() : null;
    }

    /** The step the event is about; null for a run-level event. */
    public String stepId() {
        JsonNode value = raw.path("stepId");
        return value.isTextual() ? value.textValue() : null;
    }

    /** The attempt the event is about; null if absent. */
    public Integer attempt() {
        JsonNode value = raw.path("attempt");
        return value.isIntegralNumber() ? value.intValue() : null;
    }
}
