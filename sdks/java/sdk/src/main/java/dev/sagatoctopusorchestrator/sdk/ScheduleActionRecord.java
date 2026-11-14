package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * What a schedule decided about one of its times. {@code outcome}, {@code kind} and {@code origin} are kept as the
 * strings they came as: the set of values only grows, and one this SDK does not know is never an error.
 *
 * @param nominalTime            the time the decision is about
 * @param actualTime             when the action was made (nullable)
 * @param kind                   {@code SCHEDULED}, {@code TRIGGERED}, {@code BACKFILLED}, ...
 * @param origin                 who asked for it (nullable)
 * @param revision               the revision of the schedule when it was decided
 * @param outcome                {@code STARTED}, {@code SKIPPED_OVERLAP}, {@code ALREADY_ACTED}, ...
 * @param reason                 why, for an outcome that has a reason (nullable)
 * @param runId                  the run it started (nullable)
 * @param backfillId             the backfill it belongs to (nullable)
 * @param summaryCount           how many times a summary stands for (nullable)
 * @param summaryLastNominalTime the last of them (nullable)
 * @param decidedAt              when it was decided
 * @param raw                    the answer as sent
 */
public record ScheduleActionRecord(
        Instant nominalTime,
        Instant actualTime,
        String kind,
        String origin,
        long revision,
        String outcome,
        String reason,
        String runId,
        String backfillId,
        Long summaryCount,
        Instant summaryLastNominalTime,
        Instant decidedAt,
        JsonNode raw) {

    static ScheduleActionRecord parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a schedule action");
        return new ScheduleActionRecord(
                Wire.instant(raw, "nominalTime"),
                Wire.optInstant(raw, "actualTime"),
                Wire.str(raw, "kind"),
                Wire.optStr(raw, "origin"),
                Wire.revision(raw, "revision", 1),
                Wire.str(raw, "outcome"),
                Wire.optStr(raw, "reason"),
                Wire.optStr(raw, "runId"),
                Wire.optStr(raw, "backfillId"),
                Wire.optNum(raw, "summaryCount"),
                Wire.optInstant(raw, "summaryLastNominalTime"),
                Wire.instant(raw, "decidedAt"),
                raw);
    }
}
