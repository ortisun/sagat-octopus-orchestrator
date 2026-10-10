package dev.sagatoctopusorchestrator.sdk;

import java.time.Instant;

/** An event to publish to the inbox of the namespace. */
public final class EventInput {
    private final String type;
    private final Object data;
    private String eventId;
    private String subject;
    private Instant occurredAt;
    private String correlationKey;
    private Integer dedupeTtlSeconds;

    private EventInput(String type, Object data) {
        this.type = type;
        this.data = data;
    }

    /** An event of a type with a data object (a map, a record, a bean or a {@code JsonNode}). */
    public static EventInput of(String type, Object data) {
        if (type == null || type.isBlank()) {
            throw SagatException.invalid("an event needs a type");
        }
        return new EventInput(type, data);
    }

    /** Unique within the source for the deduplication window; generated per call when absent and kept by its retries. */
    public EventInput eventId(String value) {
        this.eventId = value;
        return this;
    }

    public EventInput subject(String value) {
        this.subject = value;
        return this;
    }

    public EventInput occurredAt(Instant value) {
        this.occurredAt = value;
        return this;
    }

    /** The key that correlates the event with the runs that wait for it. */
    public EventInput correlationKey(String value) {
        this.correlationKey = value;
        return this;
    }

    public EventInput dedupeTtlSeconds(Integer value) {
        this.dedupeTtlSeconds = value;
        return this;
    }

    public String type() {
        return type;
    }

    public Object data() {
        return data;
    }

    public String eventId() {
        return eventId;
    }

    public String subject() {
        return subject;
    }

    public Instant occurredAt() {
        return occurredAt;
    }

    public String correlationKey() {
        return correlationKey;
    }

    public Integer dedupeTtlSeconds() {
        return dedupeTtlSeconds;
    }
}
