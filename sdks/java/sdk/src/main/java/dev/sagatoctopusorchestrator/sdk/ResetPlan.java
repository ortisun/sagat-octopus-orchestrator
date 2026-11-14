package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/**
 * What a reset to a point would restore and run again. Asking for it changes nothing.
 *
 * @param sequence                the point
 * @param restored                the steps whose results the new run takes over
 * @param reexecuted              the steps the new run runs again
 * @param reapply                 what would be applied again on top (each member as the server sent it)
 * @param requiresAcknowledgement whether the reset needs {@code acknowledgeReexecution}
 * @param prefixEvents            how many journal events the new run starts from
 * @param raw                     the answer as sent
 */
public record ResetPlan(
        long sequence,
        List<Restored> restored,
        List<Reexecuted> reexecuted,
        List<JsonNode> reapply,
        boolean requiresAcknowledgement,
        long prefixEvents,
        JsonNode raw) {

    /** A step the new run takes over. */
    public record Restored(String node, String kind) {}

    /**
     * A step the new run runs again.
     *
     * @param node              the step
     * @param completedInSource whether it had completed in the source run
     * @param markers           what the plan flags about it ({@code irreversible}, ...)
     */
    public record Reexecuted(String node, boolean completedInSource, List<String> markers) {}

    static ResetPlan parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a reset plan");
        List<Restored> restored = Wire.array(raw.path("restored")).stream()
                .map(item -> {
                    JsonNode r = Wire.object(item, "a restored step");
                    return new Restored(Wire.str(r, "node"), Wire.str(r, "kind"));
                })
                .toList();
        List<Reexecuted> reexecuted = Wire.array(raw.path("reexecuted")).stream()
                .map(item -> {
                    JsonNode r = Wire.object(item, "a re-executed step");
                    return new Reexecuted(
                            Wire.str(r, "node"), r.path("completedInSource").isBoolean() && r.path("completedInSource").booleanValue(),
                            Wire.strings(r.path("markers")));
                })
                .toList();
        return new ResetPlan(
                Wire.revision(raw, "sequence", 1), restored, reexecuted, Wire.array(raw.path("reapply")),
                raw.path("requiresAcknowledgement").isBoolean() && raw.path("requiresAcknowledgement").booleanValue(),
                Wire.revision(raw, "prefixEvents", 1), raw);
    }
}
