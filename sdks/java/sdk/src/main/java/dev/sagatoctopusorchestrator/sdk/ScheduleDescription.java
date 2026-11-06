package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;
import java.util.List;
import java.util.Map;

/**
 * A schedule with its next times, its latest decisions, the runs it has open and its counters.
 *
 * @param schedule      the schedule itself
 * @param nextActions   its next times, as many as were asked for
 * @param recentActions its latest decisions, newest first, as many as were asked for
 * @param openRuns      the runs it started that are still open
 * @param bufferedCount how many actions wait for an open run to end
 * @param counters      how many decisions of each outcome it has made (the outcomes are strings: the set only grows)
 * @param raw           the answer as sent
 */
public record ScheduleDescription(
        Schedule schedule,
        List<NextAction> nextActions,
        List<ScheduleActionRecord> recentActions,
        List<String> openRuns,
        long bufferedCount,
        Map<String, Long> counters,
        JsonNode raw) {

    /** A time the schedule will act at. */
    public record NextAction(Instant nominalTime, Instant actualTime) {}

    static ScheduleDescription parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a schedule description");
        List<NextAction> next = Wire.array(raw.path("nextActions")).stream()
                .map(item -> {
                    JsonNode entry = Wire.object(item, "a next action");
                    return new NextAction(Wire.instant(entry, "nominalTime"), Wire.instant(entry, "actualTime"));
                })
                .toList();
        List<ScheduleActionRecord> recent = Wire.array(raw.path("recentActions")).stream()
                .map(ScheduleActionRecord::parse)
                .toList();
        return new ScheduleDescription(
                Schedule.parse(raw), next, recent, Wire.strings(raw.path("openRuns")),
                Wire.num(raw, "bufferedCount", 0), Wire.counters(raw.path("counters")), raw);
    }
}
