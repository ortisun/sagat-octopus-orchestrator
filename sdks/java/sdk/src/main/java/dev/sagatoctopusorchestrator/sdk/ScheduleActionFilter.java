package dev.sagatoctopusorchestrator.sdk;

/** The filter and page of {@link Schedules#listActions}. A fluent, mutable holder. */
public final class ScheduleActionFilter {
    private String cursor;
    private Integer limit;
    private String traceparent;
    private String outcome;

    public static ScheduleActionFilter create() {
        return new ScheduleActionFilter();
    }

    /** The {@code nextCursor} of the previous page. */
    public ScheduleActionFilter cursor(String value) {
        this.cursor = value;
        return this;
    }

    /** The page size. */
    public ScheduleActionFilter limit(Integer value) {
        this.limit = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public ScheduleActionFilter traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    /** Only the decisions with this outcome ({@code STARTED}, {@code SKIPPED_OVERLAP}, ...). */
    public ScheduleActionFilter outcome(String value) {
        this.outcome = value;
        return this;
    }

    public String cursor() {
        return cursor;
    }

    public Integer limit() {
        return limit;
    }

    public String traceparent() {
        return traceparent;
    }

    public String outcome() {
        return outcome;
    }
}
