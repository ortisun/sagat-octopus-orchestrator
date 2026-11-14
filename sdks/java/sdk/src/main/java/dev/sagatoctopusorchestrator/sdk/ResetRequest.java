package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;

/**
 * A reset to make: a new run from the prefix of a run that ended, up to a point. A fluent, mutable holder: build one per
 * call. An optional member left unset is left out of the request, never sent as null.
 */
public final class ResetRequest {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final long sequence;
    private final String reason;
    private String reapply;
    private String sourcePolicy;
    private String businessId;
    private Boolean acknowledgeReexecution;

    private ResetRequest(long sequence, String reason) {
        this.sequence = sequence;
        this.reason = reason;
    }

    /**
     * @param sequence the point to go back to, 1 or more (see {@link SagatClient#resetPoints})
     * @param reason   why the reset is made; the server judges its length
     */
    public static ResetRequest of(long sequence, String reason) {
        return new ResetRequest(sequence, reason);
    }

    /** What to deliver again on top of the prefix: {@code NONE}, {@code SIGNALS}, {@code UPDATES} or {@code ALL}. */
    public ResetRequest reapply(String value) {
        this.reapply = value;
        return this;
    }

    /** What to do with the source run: {@code REQUIRE_TERMINAL} or {@code CANCEL_SOURCE}. */
    public ResetRequest sourcePolicy(String value) {
        this.sourcePolicy = value;
        return this;
    }

    /** What happens to the business identifier of the source run: {@code KEEP} or {@code DROP}. */
    public ResetRequest businessId(String value) {
        this.businessId = value;
        return this;
    }

    /** Says that the caller read the plan and accepts that its steps run again. */
    public ResetRequest acknowledgeReexecution(Boolean value) {
        this.acknowledgeReexecution = value;
        return this;
    }

    public long sequence() {
        return sequence;
    }

    public String reason() {
        return reason;
    }

    public String reapply() {
        return reapply;
    }

    public String sourcePolicy() {
        return sourcePolicy;
    }

    public String businessId() {
        return businessId;
    }

    public Boolean acknowledgeReexecution() {
        return acknowledgeReexecution;
    }

    ObjectNode toJson() {
        if (sequence < 1) {
            throw SagatException.invalid("sequence is a positive integer");
        }
        if (reason == null) {
            throw SagatException.invalid("a reset has a reason");
        }
        ObjectNode out = F.objectNode();
        out.put("sequence", sequence);
        out.put("reason", reason);
        if (reapply != null) {
            out.put("reapply", reapply);
        }
        if (sourcePolicy != null) {
            out.put("sourcePolicy", sourcePolicy);
        }
        if (businessId != null) {
            out.put("businessId", businessId);
        }
        if (acknowledgeReexecution != null) {
            out.put("acknowledgeReexecution", acknowledgeReexecution.booleanValue());
        }
        return out;
    }
}
