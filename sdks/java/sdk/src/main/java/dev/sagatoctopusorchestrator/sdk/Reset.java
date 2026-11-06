package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;
import java.util.List;

/**
 * A reset of a run, as the register holds it. {@code state} is kept as the string it came as: the set only grows.
 *
 * @param resetId       the reset's id
 * @param sourceRunId   the run that was reset
 * @param sequence      the point it went back to
 * @param state         {@code PENDING_SOURCE}, {@code CREATED}, ...
 * @param failureReason why it failed (nullable)
 * @param newRunId      the run it made (nullable until it is made)
 * @param reapplyReport what was applied again (each member as the server sent it)
 * @param createdAt     when the reset was made (nullable)
 * @param raw           the answer as sent
 */
public record Reset(
        String resetId,
        String sourceRunId,
        long sequence,
        String state,
        String failureReason,
        String newRunId,
        List<JsonNode> reapplyReport,
        Instant createdAt,
        JsonNode raw) {

    static Reset parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a reset");
        return new Reset(
                Wire.str(raw, "resetId"), Wire.str(raw, "sourceRunId"), Wire.revision(raw, "sequence", 1), Wire.str(raw, "state"),
                Wire.optStr(raw, "failureReason"), Wire.optStr(raw, "newRunId"), Wire.array(raw.path("reapplyReport")),
                Wire.optInstant(raw, "createdAt"), raw);
    }
}
