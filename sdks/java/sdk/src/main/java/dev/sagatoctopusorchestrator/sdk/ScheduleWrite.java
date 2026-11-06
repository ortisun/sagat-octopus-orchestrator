package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import com.fasterxml.jackson.databind.node.ArrayNode;
import com.fasterxml.jackson.databind.node.JsonNodeFactory;
import com.fasterxml.jackson.databind.node.ObjectNode;
import java.time.Instant;
import java.util.ArrayList;
import java.util.List;

/**
 * What a schedule is written as ({@link Schedules#create}, {@link Schedules#update}): a name, when it acts, how it
 * behaves around its actions and what it does. A fluent, mutable holder: build one per call. An optional field left
 * unset is left out of the request, never sent as null.
 */
public final class ScheduleWrite {
    private static final JsonNodeFactory F = JsonNodeFactory.instance;

    private final String name;
    private final Spec spec;
    private final Action action;
    private Policies policies;
    private Boolean paused;
    private String note;
    private Long remainingActions;

    private ScheduleWrite(String name, Spec spec, Action action) {
        this.name = name;
        this.spec = spec;
        this.action = action;
    }

    /**
     * A schedule of that name, that acts at the times of {@code spec} by doing {@code action}.
     *
     * @param name   1 to 128 characters of a-z, 0-9 and "-", starting with a letter or digit
     * @param spec   when it acts
     * @param action what it does
     */
    public static ScheduleWrite of(String name, Spec spec, Action action) {
        return new ScheduleWrite(name, spec, action);
    }

    public ScheduleWrite policies(Policies value) {
        this.policies = value;
        return this;
    }

    /** Creates the schedule paused. */
    public ScheduleWrite paused(Boolean value) {
        this.paused = value;
        return this;
    }

    public ScheduleWrite note(String value) {
        this.note = value;
        return this;
    }

    /** How many times the schedule may still act; unset is no limit. */
    public ScheduleWrite remainingActions(Long value) {
        this.remainingActions = value;
        return this;
    }

    public String name() {
        return name;
    }

    public Spec spec() {
        return spec;
    }

    public Action action() {
        return action;
    }

    public Policies policies() {
        return policies;
    }

    public Boolean paused() {
        return paused;
    }

    public String note() {
        return note;
    }

    public Long remainingActions() {
        return remainingActions;
    }

    /** The write on the wire; the instants of the spec are sent as {@code Z} instants. */
    ObjectNode toJson() {
        if (name == null || spec == null || action == null) {
            throw SagatException.invalid("a schedule has a name, a spec and an action");
        }
        ObjectNode out = F.objectNode();
        out.put("name", name);
        out.set("spec", spec.toJson());
        if (policies != null) {
            out.set("policies", policies.toJson());
        }
        out.set("action", action.toJson());
        if (paused != null) {
            out.put("paused", paused.booleanValue());
        }
        if (note != null) {
            out.put("note", note);
        }
        if (remainingActions != null) {
            out.put("remainingActions", remainingActions.longValue());
        }
        return out;
    }

    // ---------------------------------------------------------------------------------------- the parts

    /**
     * A range of a calendar field: {@code start}, up to {@code end} ({@code start} alone when {@code end} is null),
     * every {@code step} (1 when null).
     */
    public record Range(long start, Long end, Long step) {
        /** Exactly {@code value}. */
        public static Range at(long value) {
            return new Range(value, null, null);
        }

        /** From {@code start} to {@code end}. */
        public static Range between(long start, long end) {
            return new Range(start, end, null);
        }

        ObjectNode toJson() {
            ObjectNode out = F.objectNode();
            out.put("start", start);
            if (end != null) {
                out.put("end", end.longValue());
            }
            if (step != null) {
                out.put("step", step.longValue());
            }
            return out;
        }
    }

    /** The calendar form of a schedule: a time matches when every field it names matches. */
    public static final class Calendar {
        private List<Range> second;
        private List<Range> minute;
        private List<Range> hour;
        private List<Range> dayOfMonth;
        private List<Range> month;
        private List<Range> year;
        private List<Range> dayOfWeek;
        private String comment;

        public static Calendar create() {
            return new Calendar();
        }

        public Calendar second(Range... ranges) {
            this.second = List.of(ranges);
            return this;
        }

        public Calendar minute(Range... ranges) {
            this.minute = List.of(ranges);
            return this;
        }

        public Calendar hour(Range... ranges) {
            this.hour = List.of(ranges);
            return this;
        }

        public Calendar dayOfMonth(Range... ranges) {
            this.dayOfMonth = List.of(ranges);
            return this;
        }

        public Calendar month(Range... ranges) {
            this.month = List.of(ranges);
            return this;
        }

        public Calendar year(Range... ranges) {
            this.year = List.of(ranges);
            return this;
        }

        public Calendar dayOfWeek(Range... ranges) {
            this.dayOfWeek = List.of(ranges);
            return this;
        }

        public Calendar comment(String value) {
            this.comment = value;
            return this;
        }

        ObjectNode toJson() {
            ObjectNode out = F.objectNode();
            put(out, "second", second);
            put(out, "minute", minute);
            put(out, "hour", hour);
            put(out, "dayOfMonth", dayOfMonth);
            put(out, "month", month);
            put(out, "year", year);
            put(out, "dayOfWeek", dayOfWeek);
            if (comment != null) {
                out.put("comment", comment);
            }
            return out;
        }

        private static void put(ObjectNode out, String field, List<Range> ranges) {
            if (ranges != null) {
                ArrayNode array = out.putArray(field);
                ranges.forEach(range -> array.add(range.toJson()));
            }
        }
    }

    /** A fixed interval, optionally shifted by an offset from the epoch. */
    public record Interval(long everyMs, Long offsetMs) {
        public static Interval every(long everyMs) {
            return new Interval(everyMs, null);
        }

        ObjectNode toJson() {
            ObjectNode out = F.objectNode();
            out.put("everyMs", everyMs);
            if (offsetMs != null) {
                out.put("offsetMs", offsetMs.longValue());
            }
            return out;
        }
    }

    /**
     * What a local time that the zone skips or repeats does. {@code gap} is {@code SKIP} or {@code SHIFT_FORWARD},
     * {@code ambiguous} is {@code ONCE_EARLIER} or {@code TWICE}; null leaves the server's default.
     */
    public record Dst(String gap, String ambiguous) {
        ObjectNode toJson() {
            ObjectNode out = F.objectNode();
            if (gap != null) {
                out.put("gap", gap);
            }
            if (ambiguous != null) {
                out.put("ambiguous", ambiguous);
            }
            return out;
        }
    }

    /**
     * When a schedule acts: any of calendars, cron strings and intervals, less the exclusions, between {@code startAt}
     * and {@code endAt}, in the zone {@code timezone}. A fluent, mutable holder.
     */
    public static final class Spec {
        private final List<Calendar> calendars = new ArrayList<>();
        private final List<String> cron = new ArrayList<>();
        private final List<Interval> intervals = new ArrayList<>();
        private final List<Calendar> exclusions = new ArrayList<>();
        private Object startAt;
        private Object endAt;
        private String timezone;
        private Long jitterMs;
        private Dst dst;

        public static Spec create() {
            return new Spec();
        }

        public Spec calendar(Calendar value) {
            this.calendars.add(value);
            return this;
        }

        /** A cron string. */
        public Spec cron(String value) {
            this.cron.add(value);
            return this;
        }

        public Spec interval(Interval value) {
            this.intervals.add(value);
            return this;
        }

        /** A time that is left out. */
        public Spec exclusion(Calendar value) {
            this.exclusions.add(value);
            return this;
        }

        /** The schedule does not act before this instant. Sent in UTC, as an instant with a {@code Z}. */
        public Spec startAt(Instant value) {
            this.startAt = value;
            return this;
        }

        /**
         * The same, as RFC 3339 text with any offset. Text that is not an instant (or has no offset) is refused when the
         * schedule is written, as {@code VALIDATION_FAILED}, and nothing is sent.
         */
        public Spec startAt(String value) {
            this.startAt = value;
            return this;
        }

        /** The schedule does not act from this instant on. Sent in UTC, as an instant with a {@code Z}. */
        public Spec endAt(Instant value) {
            this.endAt = value;
            return this;
        }

        /** The same, as RFC 3339 text; see {@link #startAt(String)}. */
        public Spec endAt(String value) {
            this.endAt = value;
            return this;
        }

        public Spec timezone(String value) {
            this.timezone = value;
            return this;
        }

        public Spec jitterMs(Long value) {
            this.jitterMs = value;
            return this;
        }

        public Spec dst(Dst value) {
            this.dst = value;
            return this;
        }

        ObjectNode toJson() {
            ObjectNode out = F.objectNode();
            if (!calendars.isEmpty()) {
                ArrayNode array = out.putArray("calendars");
                calendars.forEach(calendar -> array.add(calendar.toJson()));
            }
            if (!cron.isEmpty()) {
                ArrayNode array = out.putArray("cron");
                cron.forEach(array::add);
            }
            if (!intervals.isEmpty()) {
                ArrayNode array = out.putArray("intervals");
                intervals.forEach(interval -> array.add(interval.toJson()));
            }
            if (!exclusions.isEmpty()) {
                ArrayNode array = out.putArray("exclusions");
                exclusions.forEach(calendar -> array.add(calendar.toJson()));
            }
            if (startAt != null) {
                out.put("startAt", Wire.whenGiven(startAt, "startAt"));
            }
            if (endAt != null) {
                out.put("endAt", Wire.whenGiven(endAt, "endAt"));
            }
            if (timezone != null) {
                out.put("timezone", timezone);
            }
            if (jitterMs != null) {
                out.put("jitterMs", jitterMs.longValue());
            }
            if (dst != null) {
                out.set("dst", dst.toJson());
            }
            return out;
        }
    }

    /** How a schedule behaves around its actions. A fluent, mutable holder; an unset field is the server's default. */
    public static final class Policies {
        private String overlap;
        private Long catchupWindowMs;
        private String catchupMode;
        private Boolean pauseOnFailure;
        private Integer bufferLimit;

        public static Policies create() {
            return new Policies();
        }

        /** What to do when a run the schedule started is still open at its next action; see {@link OverlapPolicy}. */
        public Policies overlap(String value) {
            this.overlap = value;
            return this;
        }

        /** How far back a missed time is still acted on (0: none). */
        public Policies catchupWindowMs(Long value) {
            this.catchupWindowMs = value;
            return this;
        }

        /** {@code ONE} or {@code ALL}. */
        public Policies catchupMode(String value) {
            this.catchupMode = value;
            return this;
        }

        public Policies pauseOnFailure(Boolean value) {
            this.pauseOnFailure = value;
            return this;
        }

        public Policies bufferLimit(Integer value) {
            this.bufferLimit = value;
            return this;
        }

        ObjectNode toJson() {
            ObjectNode out = F.objectNode();
            if (overlap != null) {
                out.put("overlap", overlap);
            }
            if (catchupWindowMs != null) {
                out.put("catchupWindowMs", catchupWindowMs.longValue());
            }
            if (catchupMode != null) {
                out.put("catchupMode", catchupMode);
            }
            if (pauseOnFailure != null) {
                out.put("pauseOnFailure", pauseOnFailure.booleanValue());
            }
            if (bufferLimit != null) {
                out.put("bufferLimit", bufferLimit.intValue());
            }
            return out;
        }
    }

    /** What a schedule does at each of its times: it starts a run of a workflow. A fluent, mutable holder. */
    public static final class Action {
        private final String workflowId;
        private final Object input;
        private String versionPolicy;
        private String versionId;
        private String nominalTimeField;
        private Long runTimeoutMs;

        private Action(String workflowId, Object input) {
            this.workflowId = workflowId;
            this.input = input;
        }

        /**
         * Starts a run of {@code workflowId}, sent as given.
         *
         * @param workflowId the workflow
         * @param input      the input of every run, at most 64 KiB; null is an empty object. The server never returns it:
         *                   only its digest
         */
        public static Action of(String workflowId, Object input) {
            return new Action(workflowId, input);
        }

        /** {@code ACTIVE} or {@code PINNED} (with {@link #versionId(String)}). */
        public Action versionPolicy(String value) {
            this.versionPolicy = value;
            return this;
        }

        public Action versionId(String value) {
            this.versionId = value;
            return this;
        }

        /** The member of the input that receives the nominal time of the action. */
        public Action nominalTimeField(String value) {
            this.nominalTimeField = value;
            return this;
        }

        public Action runTimeoutMs(Long value) {
            this.runTimeoutMs = value;
            return this;
        }

        ObjectNode toJson() {
            if (workflowId == null) {
                throw SagatException.invalid("the action of a schedule names a workflow");
            }
            ObjectNode out = F.objectNode();
            out.put("workflowId", workflowId);
            if (versionPolicy != null) {
                out.put("versionPolicy", versionPolicy);
            }
            if (versionId != null) {
                out.put("versionId", versionId);
            }
            JsonNode value = input == null ? F.objectNode() : Json.toTree(input);
            out.set("input", value);
            if (nominalTimeField != null) {
                out.put("nominalTimeField", nominalTimeField);
            }
            if (runTimeoutMs != null) {
                out.put("runTimeoutMs", runTimeoutMs.longValue());
            }
            return out;
        }
    }
}
