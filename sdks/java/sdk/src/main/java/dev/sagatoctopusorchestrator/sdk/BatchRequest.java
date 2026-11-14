package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;

/**
 * A batch job to create: an action over the runs a selector names. A fluent, mutable holder: build one per call. An
 * optional member left unset is left out of the request, never sent as null.
 */
public final class BatchRequest {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final BatchSelector selector;
    private final String action;
    private final String reason;
    private String signalName;
    private Object signalPayload;
    private String updateName;
    private Object updateArgs;
    private String stepId;
    private String businessId;
    private Integer maxItems;
    private Integer itemsPerSecond;

    private BatchRequest(BatchSelector selector, String action, String reason) {
        this.selector = selector;
        this.action = action;
        this.reason = reason;
    }

    /**
     * @param selector which runs
     * @param action   {@code CANCEL}, {@code SIGNAL}, {@code RERUN}, {@code RETRY_STEP}, {@code PAUSE}, {@code RESUME},
     *                 {@code MARK_MANUAL_REVIEW}, ...: the set only grows, and one this SDK does not know is sent as given
     * @param reason   why the job is made; the server judges its length
     */
    public static BatchRequest of(BatchSelector selector, String action, String reason) {
        return new BatchRequest(selector, action, reason);
    }

    /** The signal the {@code SIGNAL} action sends to each run. */
    public BatchRequest signal(String name, Object payload) {
        this.signalName = name;
        this.signalPayload = payload;
        return this;
    }

    /** The declared update the action applies to each run. */
    public BatchRequest update(String name, Object args) {
        this.updateName = name;
        this.updateArgs = args;
        return this;
    }

    /** The step the {@code RETRY_STEP} action retries. */
    public BatchRequest stepId(String value) {
        this.stepId = value;
        return this;
    }

    /** What happens to the business identifier of a rerun: {@code KEEP} or {@code DROP}. */
    public BatchRequest businessId(String value) {
        this.businessId = value;
        return this;
    }

    public BatchRequest maxItems(Integer value) {
        this.maxItems = value;
        return this;
    }

    public BatchRequest itemsPerSecond(Integer value) {
        this.itemsPerSecond = value;
        return this;
    }

    ObjectNode toJson() {
        if (selector == null || action == null || reason == null) {
            throw SagatException.invalid("a batch request has a selector, an action and a reason");
        }
        ObjectNode out = F.objectNode();
        out.set("selector", selector.toJson());
        out.put("action", action);
        if (signalName != null) {
            ObjectNode signal = out.putObject("signal");
            signal.put("name", signalName);
            signal.set("payload", signalPayload == null ? F.objectNode() : Json.toTree(signalPayload));
        }
        if (updateName != null) {
            ObjectNode update = out.putObject("update");
            update.put("name", updateName);
            if (updateArgs != null) {
                update.set("args", Json.toTree(updateArgs));
            }
        }
        if (stepId != null) {
            out.put("stepId", stepId);
        }
        if (businessId != null) {
            out.put("businessId", businessId);
        }
        out.put("reason", reason);
        if (maxItems != null) {
            out.put("maxItems", maxItems.intValue());
        }
        if (itemsPerSecond != null) {
            out.put("itemsPerSecond", itemsPerSecond.intValue());
        }
        return out;
    }
}
