package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;
import java.util.Map;

/**
 * A backfill of a schedule: its actions on the nominal times of a past range that had none.
 *
 * @param backfillId the backfill's id
 * @param start      the start of the range (inclusive)
 * @param end        the end of the range (exclusive)
 * @param cursor     how far the backfill has got (nullable)
 * @param state      {@code PENDING}, {@code RUNNING}, {@code COMPLETED}, {@code CANCELLED}, ...: the set only grows
 * @param overlap    the overlap policy of its actions, as the string it came as
 * @param counts     how many of its times had each outcome
 * @param raw        the answer as sent
 */
public record ScheduleBackfill(
        String backfillId,
        Instant start,
        Instant end,
        Instant cursor,
        String state,
        String overlap,
        Map<String, Long> counts,
        JsonNode raw) {

    static ScheduleBackfill parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a backfill");
        return new ScheduleBackfill(
                Wire.str(raw, "backfillId"), Wire.instant(raw, "start"), Wire.instant(raw, "end"), Wire.optInstant(raw, "cursor"),
                Wire.str(raw, "state"), Wire.str(raw, "overlap"), Wire.counters(raw.path("counts")), raw);
    }
}
