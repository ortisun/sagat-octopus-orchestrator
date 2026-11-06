package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * A schedule as the server keeps it. The input of its action is customer content: it is never returned, only its
 * digest. {@code raw} is the answer as sent, so a field of a newer server is reachable.
 *
 * @param scheduleId       the schedule's id
 * @param name             its name
 * @param origin           where it came from ({@code API}, {@code TRIGGER}, ...: the set only grows)
 * @param state            {@code ACTIVE}, {@code PAUSED}, ...: the set only grows
 * @param revision         its revision, 1 or more; what {@code expectedRevision} names
 * @param exhausted        whether it has no action left
 * @param action           what it does
 * @param note             the note of its latest pause or resume (nullable)
 * @param noteBy           who wrote it (nullable)
 * @param remainingActions how many actions it may still make (nullable: no limit)
 * @param nextActionAt     its next time (nullable)
 * @param createdAt        when it was created
 * @param updatedAt        when it last changed (nullable)
 * @param raw              the answer as sent
 */
public record Schedule(
        String scheduleId,
        String name,
        String origin,
        String state,
        long revision,
        boolean exhausted,
        ActionInfo action,
        String note,
        String noteBy,
        Long remainingActions,
        Instant nextActionAt,
        Instant createdAt,
        Instant updatedAt,
        JsonNode raw) {

    /** The action as the server keeps it: without the input, with its digest. Every member is nullable. */
    public record ActionInfo(
            String workflowId,
            String versionPolicy,
            String versionId,
            String inputDigest,
            String nominalTimeField,
            Long runTimeoutMs) {}

    static Schedule parse(JsonNode value) {
        JsonNode raw = Wire.object(value, "a schedule");
        JsonNode info = raw.path("action");
        return new Schedule(
                Wire.str(raw, "scheduleId"),
                Wire.str(raw, "name"),
                Wire.str(raw, "origin"),
                Wire.str(raw, "state"),
                Wire.revision(raw, "revision", 1),
                raw.path("exhausted").isBoolean() && raw.path("exhausted").booleanValue(),
                new ActionInfo(
                        Wire.optStr(info, "workflowId"), Wire.optStr(info, "versionPolicy"), Wire.optStr(info, "versionId"),
                        Wire.optStr(info, "inputDigest"), Wire.optStr(info, "nominalTimeField"), Wire.optNum(info, "runTimeoutMs")),
                Wire.optStr(raw, "note"),
                Wire.optStr(raw, "noteBy"),
                Wire.optNum(raw, "remainingActions"),
                Wire.optInstant(raw, "nextActionAt"),
                Wire.instant(raw, "createdAt"),
                Wire.optInstant(raw, "updatedAt"),
                raw);
    }
}
