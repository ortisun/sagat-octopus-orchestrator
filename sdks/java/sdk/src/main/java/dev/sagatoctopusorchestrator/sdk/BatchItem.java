package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * What a batch job did to one run. {@code state} and {@code reason} are kept as the strings they came as: the set of
 * values only grows, and one this SDK does not know is never an error.
 *
 * @param ordinal   the position of the run among the job's runs, 0 or more
 * @param runId     the run
 * @param state     {@code SUCCEEDED}, {@code SKIPPED}, {@code FAILED}, ...
 * @param reason    why, for a state that has a reason, such as {@code ALREADY_TERMINAL} (nullable)
 * @param errorCode the canonical code of a failure (nullable)
 * @param attempts  how many times the job tried the run
 * @param newRunId  the run a {@code RERUN} started (nullable)
 * @param raw       the answer as sent
 */
public record BatchItem(
        long ordinal, String runId, String state, String reason, String errorCode, long attempts, String newRunId, JsonNode raw) {

    static BatchItem parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a batch item");
        return new BatchItem(
                Wire.revision(raw, "ordinal", 0), Wire.str(raw, "runId"), Wire.str(raw, "state"), Wire.optStr(raw, "reason"),
                Wire.optStr(raw, "errorCode"), Wire.num(raw, "attempts", 0), Wire.optStr(raw, "newRunId"), raw);
    }
}
