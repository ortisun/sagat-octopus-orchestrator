package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.SagatException;

/**
 * A rate limit (feature 022): at most {@code limit} grants of one key in any window of {@code per}, strictly (there is no
 * burst). At the workflow level a run is granted at its start and the key reads {@code input}; on a step a grant counts
 * from its lease. Members that are not set are left out of the document. The policy is read when it is given to a step or a
 * workflow. The builder only assembles it: the server checks every bound, and the ceiling of each window, at publication.
 */
public final class RateLimitPolicy {
    private final int limit;
    private final RatePeriod per;
    private String key;
    private String group;
    private Long queueTimeoutMs;

    private RateLimitPolicy(int limit, RatePeriod per) {
        this.limit = limit;
        this.per = per;
    }

    /** At most {@code limit} grants of one key in a window of {@code per} (1 to 10000; at most 100 a second and 6000 a minute). */
    public static RateLimitPolicy of(int limit, RatePeriod per) {
        if (per == null) {
            throw SagatException.invalid("rateLimit per is second, minute or hour");
        }
        return new RateLimitPolicy(limit, per);
    }

    /** An expression whose value is the key; without one, the whole group is one key. */
    public RateLimitPolicy key(String value) {
        this.key = value;
        return this;
    }

    /** Shared by the definitions of a namespace that name it; without one, derived from the workflow (and node). */
    public RateLimitPolicy group(String value) {
        this.group = value;
        return this;
    }

    /** How long a run or a step waits for a slot (1 ms to 1 day; 10 minutes by default). */
    public RateLimitPolicy queueTimeoutMs(long value) {
        this.queueTimeoutMs = value;
        return this;
    }

    ObjectNode toJson() {
        ObjectNode node = JsonNodeFactory.instance.objectNode();
        node.put("limit", limit);
        node.put("per", per.wire());
        return ConcurrencyPolicy.members(node, key, group, queueTimeoutMs);
    }
}
