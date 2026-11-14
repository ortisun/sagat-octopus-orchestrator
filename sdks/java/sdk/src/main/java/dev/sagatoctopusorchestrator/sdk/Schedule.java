package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.time.Instant;

/**
 * A schedule as the server keeps it. The input of its action is customer content: it is never returned, only its
 * digest. {@code raw} is the answer as sent, so a field of a newer server is reachable. The business-id prefix of the
 * action is customer text: it is there for the caller ({@link ActionInfo#businessIdPrefix()}, {@code raw}) and is left
 * out of the debug text ({@code toString}) of the schedule.
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

    /**
     * The action as the server keeps it: without the input, with its digest. Every member is nullable.
     *
     * @param workflowId               the workflow it starts a run of
     * @param versionPolicy            {@code ACTIVE} or {@code PINNED}, as the server sent it
     * @param versionId                the pinned version
     * @param inputDigest              the digest of the input (the input itself is never returned)
     * @param nominalTimeField         the member of the input that receives the nominal time
     * @param runTimeoutMs             the timeout of each run it starts
     * @param businessIdPrefix         the prefix of the business identifier of every run it starts, which is
     *                                 {@code <prefix>-<nominal time>}; null for an action that has none. Customer text,
     *                                 exactly as the server sent it, and never in {@link #toString()}
     * @param businessIdConflictPolicy what to do when an open execution holds that identifier, as the server sent it
     *                                 (a string: one this SDK does not know is kept); null when there is no prefix
     * @param businessIdReusePolicy    what to do when a closed execution remembers it, as the server sent it; null when
     *                                 there is no prefix
     */
    public record ActionInfo(
            String workflowId,
            String versionPolicy,
            String versionId,
            String inputDigest,
            String nominalTimeField,
            Long runTimeoutMs,
            String businessIdPrefix,
            String businessIdConflictPolicy,
            String businessIdReusePolicy) {

        /** An action that has no business-id prefix. */
        public ActionInfo(
                String workflowId,
                String versionPolicy,
                String versionId,
                String inputDigest,
                String nominalTimeField,
                Long runTimeoutMs) {
            this(workflowId, versionPolicy, versionId, inputDigest, nominalTimeField, runTimeoutMs, null, null, null);
        }

        /** The prefix is customer text: it does not appear here, only whether there is one. */
        @Override
        public String toString() {
            return "ActionInfo[workflowId=" + workflowId + ", versionPolicy=" + versionPolicy + ", versionId=" + versionId
                    + ", inputDigest=" + inputDigest + ", nominalTimeField=" + nominalTimeField + ", runTimeoutMs=" + runTimeoutMs
                    + ", businessIdPrefix=" + (businessIdPrefix == null ? null : HIDDEN)
                    + ", businessIdConflictPolicy=" + businessIdConflictPolicy
                    + ", businessIdReusePolicy=" + businessIdReusePolicy + "]";
        }
    }

    private static final String HIDDEN = "<hidden>";

    /** The debug text of a schedule: every component, the prefix of its action (customer text) hidden, in {@code raw} too. */
    @Override
    public String toString() {
        return "Schedule[scheduleId=" + scheduleId + ", name=" + name + ", origin=" + origin + ", state=" + state
                + ", revision=" + revision + ", exhausted=" + exhausted + ", action=" + action + ", note=" + note
                + ", noteBy=" + noteBy + ", remainingActions=" + remainingActions + ", nextActionAt=" + nextActionAt
                + ", createdAt=" + createdAt + ", updatedAt=" + updatedAt + ", raw=" + printable(raw) + "]";
    }

    /** What {@code raw} prints as: the answer, without the business-id prefix of its action. */
    static JsonNode printable(JsonNode raw) {
        if (raw == null || !raw.path("action").isObject() || !raw.get("action").has("businessIdPrefix")) {
            return raw;
        }
        ObjectNode copy = raw.deepCopy();
        ((ObjectNode) copy.get("action")).put("businessIdPrefix", HIDDEN);
        return copy;
    }

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
                        Wire.optStr(info, "inputDigest"), Wire.optStr(info, "nominalTimeField"), Wire.optNum(info, "runTimeoutMs"),
                        Wire.optStr(info, "businessIdPrefix"), Wire.optStr(info, "businessIdConflictPolicy"),
                        Wire.optStr(info, "businessIdReusePolicy")),
                Wire.optStr(raw, "note"),
                Wire.optStr(raw, "noteBy"),
                Wire.optNum(raw, "remainingActions"),
                Wire.optInstant(raw, "nextActionAt"),
                Wire.instant(raw, "createdAt"),
                Wire.optInstant(raw, "updatedAt"),
                raw);
    }
}
