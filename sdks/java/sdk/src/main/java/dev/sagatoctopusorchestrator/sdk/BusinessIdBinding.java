package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * How a business identifier stands in a namespace.
 *
 * @param businessId    the identifier, as it was given when it was bound
 * @param executionId   the run that holds the identifier, or the initial run of the continuation chain that does
 * @param currentRunId  the run that carries the work now
 * @param open          whether the holder is still open
 * @param lastStatus    the status of the holder at its latest change (nullable)
 * @param workflowId    the workflow of the holder
 * @param boundAt       when the identifier was bound
 * @param reusableAfter when a closed holder stops being remembered (nullable)
 * @param raw           the answer as sent
 */
public record BusinessIdBinding(
        String businessId,
        String executionId,
        String currentRunId,
        boolean open,
        String lastStatus,
        String workflowId,
        Instant boundAt,
        Instant reusableAfter,
        JsonNode raw) {

    static BusinessIdBinding parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a business id binding");
        return new BusinessIdBinding(
                Wire.str(raw, "businessId"), Wire.str(raw, "executionId"), Wire.str(raw, "currentRunId"),
                raw.path("open").isBoolean() && raw.path("open").booleanValue(), Wire.optStr(raw, "lastStatus"),
                Wire.str(raw, "workflowId"), Wire.instant(raw, "boundAt"), Wire.optInstant(raw, "reusableAfter"), raw);
    }
}
