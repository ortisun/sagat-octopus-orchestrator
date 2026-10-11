package dev.sagatoctopusorchestrator.sdk.worker;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import dev.sagatoctopusorchestrator.sdk.Json;

/** What a reconciler says about the step it looks up (SPEC section 6.6). */
public final class Verdict {
    /** The three answers. */
    public enum Effect {
        APPLIED("applied"),
        NOT_APPLIED("not_applied"),
        UNKNOWN("unknown");

        private final String wire;

        Effect(String wire) {
            this.wire = wire;
        }

        public String wire() {
            return wire;
        }
    }

    private final Effect effect;
    private final boolean hasOutput;
    private final JsonNode output;

    private Verdict(Effect effect, boolean hasOutput, JsonNode output) {
        this.effect = effect;
        this.hasOutput = hasOutput;
        this.output = output;
    }

    /** The effect happened; the step's output is not known. */
    public static Verdict applied() {
        return new Verdict(Effect.APPLIED, false, null);
    }

    /** The effect happened; {@code output} is what the step would have returned. */
    public static Verdict applied(Object output) {
        return new Verdict(Effect.APPLIED, true, Json.toTree(output));
    }

    /** The effect did not happen: the platform may try it again, with the same key. */
    public static Verdict notApplied() {
        return new Verdict(Effect.NOT_APPLIED, false, null);
    }

    /** Cannot tell: the decision goes to a person. */
    public static Verdict unknown() {
        return new Verdict(Effect.UNKNOWN, false, null);
    }

    public Effect effect() {
        return effect;
    }

    /** The JSON the task is completed with: exactly {@code {"effect": ..., "output": ...}}, output only for applied. */
    public JsonNode toJson() {
        ObjectNode node = JsonNodeFactory.instance.objectNode();
        node.put("effect", effect.wire());
        if (effect == Effect.APPLIED && hasOutput) {
            node.set("output", output);
        }
        return node;
    }
}
