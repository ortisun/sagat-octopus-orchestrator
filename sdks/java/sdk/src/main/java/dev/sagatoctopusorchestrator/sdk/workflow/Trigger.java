package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;
import dev.sagatoctopusorchestrator.sdk.SagatException;

/** A trigger of a workflow: how a run is started other than by a person calling {@code start}. */
public final class Trigger {
    /** The kinds of trigger of {@code workflow.schema.json}. */
    public enum Kind {
        MANUAL,
        EVENT,
        WEBHOOK,
        MESSAGE,
        CRON
    }

    private final ObjectNode node = JsonNodeFactory.instance.objectNode();

    private Trigger(Kind kind) {
        node.put("kind", kind.name().toLowerCase(java.util.Locale.ROOT));
    }

    /** A trigger of any kind; set its fields with the methods below. */
    public static Trigger of(Kind kind) {
        return new Trigger(kind);
    }

    /** Somebody starts it. */
    public static Trigger manual() {
        return new Trigger(Kind.MANUAL);
    }

    /** An event of a type, from any source (narrow it with {@link #source(String)}). */
    public static Trigger event(String eventType) {
        return new Trigger(Kind.EVENT).eventType(eventType);
    }

    /** An event received by a webhook endpoint of the namespace. */
    public static Trigger webhook(String endpoint) {
        return new Trigger(Kind.WEBHOOK).endpoint(endpoint);
    }

    /** An event consumed from a topic or queue by a connector of the namespace. */
    public static Trigger message(String connector) {
        return new Trigger(Kind.MESSAGE).connector(connector);
    }

    /** A schedule: five fields, minute hour day-of-month month day-of-week. */
    public static Trigger cron(String expression) {
        return new Trigger(Kind.CRON).withCron(expression);
    }

    public Trigger eventType(String value) {
        node.put("eventType", value);
        return this;
    }

    public Trigger source(String value) {
        node.put("source", value);
        return this;
    }

    public Trigger endpoint(String value) {
        node.put("endpoint", value);
        return this;
    }

    public Trigger connector(String value) {
        node.put("connector", value);
        return this;
    }

    /** A side-effect-free condition over the event; the event starts a run only when it holds. */
    public Trigger filter(String value) {
        node.put("filter", value);
        return this;
    }

    public Trigger timezone(String value) {
        node.put("timezone", value);
        return this;
    }

    private Trigger withCron(String value) {
        node.put("cron", value);
        return this;
    }

    /** Sets any other field of the trigger. */
    public Trigger with(String name, Object value) {
        if ("kind".equals(name)) {
            throw SagatException.invalid("the kind of a trigger is fixed when it is created");
        }
        node.set(name, Json.toTree(value));
        return this;
    }

    /** Reads a trigger from its IR form. */
    public static Trigger fromJson(JsonNode json) {
        if (json == null || !json.isObject() || !json.path("kind").isTextual()) {
            throw SagatException.invalid("a trigger is an object with a kind");
        }
        Kind kind;
        try {
            kind = Kind.valueOf(json.path("kind").textValue().toUpperCase(java.util.Locale.ROOT));
        } catch (IllegalArgumentException e) {
            throw SagatException.invalid("the kind of a trigger is manual, event, webhook, message or cron");
        }
        Trigger trigger = new Trigger(kind);
        json.properties().forEach(entry -> {
            if (!"kind".equals(entry.getKey())) {
                trigger.node.set(entry.getKey(), entry.getValue().deepCopy());
            }
        });
        return trigger;
    }

    ObjectNode toJson() {
        return node.deepCopy();
    }
}
