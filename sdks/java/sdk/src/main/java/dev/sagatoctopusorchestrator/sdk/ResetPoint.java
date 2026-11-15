package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * A point of a run that ended that a reset can go back to. {@code kind} is kept as the string it came as: the set only
 * grows.
 *
 * @param sequence   the sequence of the journal event, 1 or more; what {@code reset} names
 * @param kind       {@code RUN_STARTED}, {@code STEP_COMPLETED}, ...
 * @param node       the node the event is about (nullable)
 * @param occurredAt when the event happened
 * @param raw        the answer as sent
 */
public record ResetPoint(long sequence, String kind, String node, Instant occurredAt, JsonNode raw) {

    static ResetPoint parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a reset point");
        return new ResetPoint(
                Wire.revision(raw, "sequence", 1), Wire.str(raw, "kind"), Wire.optStr(raw, "node"),
                Wire.instant(raw, "occurredAt"), raw);
    }
}
