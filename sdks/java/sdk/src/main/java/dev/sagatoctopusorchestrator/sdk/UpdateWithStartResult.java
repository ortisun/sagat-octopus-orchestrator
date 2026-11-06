package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * The answer to {@link SagatClient#updateWithStart}. A refusal of the validator or a stale revision is an answer
 * ({@link Update#outcome()} is {@code REJECTED}, the run was started and there is no receipt), not an error.
 *
 * @param runId   the run that holds the business identifier now
 * @param started whether this request created the run
 * @param update  what became of the update
 * @param raw     the answer as sent
 */
public record UpdateWithStartResult(String runId, boolean started, Update update, JsonNode raw) {

    /**
     * What became of the update.
     *
     * @param outcome {@code APPLIED}, {@code REJECTED}, ...: the set only grows
     * @param receipt the durable receipt of an applied (or replayed) update; null for a rejected one
     * @param reason  why it was rejected (nullable)
     */
    public record Update(String outcome, Receipt receipt, String reason) {}

    /**
     * The durable receipt of an update; {@code replayed} says that it is the answer to an update the same key had
     * already applied.
     */
    public record Receipt(
            String runId,
            String operation,
            String updateKey,
            String status,
            JsonNode result,
            long beforeStateRevision,
            long afterStateRevision,
            long runVersion,
            boolean replayed,
            JsonNode raw) {}

    static UpdateWithStartResult parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "an update-with-start result");
        JsonNode update = Wire.object(raw.get("update"), "the update of the result");
        String outcome = Wire.str(update, "outcome");
        Receipt receipt = null;
        JsonNode given = update.get("receipt");
        if (given != null && !given.isNull()) {
            JsonNode r = Wire.object(given, "an update receipt");
            receipt = new Receipt(
                    Wire.str(r, "runId"), Wire.str(r, "operation"), Wire.str(r, "updateKey"), Wire.str(r, "status"),
                    r.get("result"), Wire.revision(r, "beforeStateRevision", 0), Wire.revision(r, "afterStateRevision", 0),
                    Wire.revision(r, "runVersion", 0), r.path("replayed").isBoolean() && r.path("replayed").booleanValue(), r);
        }
        return new UpdateWithStartResult(
                Wire.str(raw, "runId"), raw.path("started").isBoolean() && raw.path("started").booleanValue(),
                new Update(outcome, receipt, Wire.optStr(update, "reason")), raw);
    }
}
