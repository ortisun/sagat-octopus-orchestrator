package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;

/**
 * A concurrency gate (feature 022): at most {@code limit} holders of one key. At the workflow level a run holds a permit
 * from its start to its end and the key reads {@code input}; on a step an attempt holds it and the key reads the run data
 * when the step becomes ready. Members that are not set are left out of the document. The policy is read when it is given
 * to a step or a workflow. The builder only assembles it: the server checks every bound at publication.
 */
public final class ConcurrencyPolicy {
    private final int limit;
    private String key;
    private String group;
    private Long queueTimeoutMs;

    private ConcurrencyPolicy(int limit) {
        this.limit = limit;
    }

    /** At most {@code limit} holders of one key (1 to 10000). */
    public static ConcurrencyPolicy of(int limit) {
        return new ConcurrencyPolicy(limit);
    }

    /** An expression whose value is the key; without one, the whole group is one key. */
    public ConcurrencyPolicy key(String value) {
        this.key = value;
        return this;
    }

    /** Shared by the definitions of a namespace that name it; without one, derived from the workflow (and node). */
    public ConcurrencyPolicy group(String value) {
        this.group = value;
        return this;
    }

    /** How long a run or a ready step waits for a permit (1 ms to 1 day; 10 minutes by default). */
    public ConcurrencyPolicy queueTimeoutMs(long value) {
        this.queueTimeoutMs = value;
        return this;
    }

    ObjectNode toJson() {
        ObjectNode node = JsonNodeFactory.instance.objectNode();
        node.put("limit", limit);
        return members(node, key, group, queueTimeoutMs);
    }

    /** The optional members a concurrency gate and a rate limit share. */
    static ObjectNode members(ObjectNode node, String key, String group, Long queueTimeoutMs) {
        if (key != null) {
            node.put("key", key);
        }
        if (group != null) {
            node.put("group", group);
        }
        if (queueTimeoutMs != null) {
            node.put("queueTimeoutMs", queueTimeoutMs.longValue());
        }
        return node;
    }
}
