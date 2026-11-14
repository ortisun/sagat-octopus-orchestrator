package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * A durable batch job. {@code action} and {@code state} are kept as the strings they came as: the set of values only
 * grows, and one this SDK does not know is never an error.
 *
 * @param batchId        the job's id
 * @param action         {@code CANCEL}, {@code SIGNAL}, {@code RERUN}, ...
 * @param state          {@code RUNNING}, {@code CANCELLING}, {@code COMPLETED}, ...
 * @param failureReason  why the job failed (nullable)
 * @param createdBy      who made it
 * @param hasPayload     whether the job carries a signal or an update
 * @param maxItems       the cap it was made with (nullable)
 * @param itemsPerSecond the rate it was made with (nullable)
 * @param counts         how many runs are in each state
 * @param createdAt      when it was made
 * @param startedAt      when it started (nullable)
 * @param finishedAt     when it ended (nullable)
 * @param expiresAt      when the server forgets it (nullable)
 * @param raw            the answer as sent
 */
public record BatchJob(
        String batchId,
        String action,
        String state,
        String failureReason,
        String createdBy,
        boolean hasPayload,
        Long maxItems,
        Long itemsPerSecond,
        Counts counts,
        Instant createdAt,
        Instant startedAt,
        Instant finishedAt,
        Instant expiresAt,
        JsonNode raw) {

    /** How many runs of the job are in each state. */
    public record Counts(long selected, long succeeded, long skipped, long failed, long cancelled, long deferred) {}

    static BatchJob parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a batch job");
        JsonNode counts = raw.path("counts");
        return new BatchJob(
                Wire.str(raw, "batchId"), Wire.str(raw, "action"), Wire.str(raw, "state"), Wire.optStr(raw, "failureReason"),
                Wire.str(raw, "createdBy"), raw.path("hasPayload").isBoolean() && raw.path("hasPayload").booleanValue(),
                Wire.optNum(raw, "maxItems"), Wire.optNum(raw, "itemsPerSecond"),
                new Counts(
                        Wire.num(counts, "selected", 0), Wire.num(counts, "succeeded", 0), Wire.num(counts, "skipped", 0),
                        Wire.num(counts, "failed", 0), Wire.num(counts, "cancelled", 0), Wire.num(counts, "deferred", 0)),
                Wire.instant(raw, "createdAt"), Wire.optInstant(raw, "startedAt"), Wire.optInstant(raw, "finishedAt"),
                Wire.optInstant(raw, "expiresAt"), raw);
    }
}
