package dev.sagatoctopusorchestrator.sdk;

/** The filter and page of {@link Schedules#list} and {@link Schedules#listAll}. A fluent, mutable holder. */
public final class ScheduleListFilter {
    private String cursor;
    private Integer limit;
    private String traceparent;
    private String state;
    private String origin;
    private String workflowId;
    private String name;

    public static ScheduleListFilter create() {
        return new ScheduleListFilter();
    }

    /** The {@code nextCursor} of the previous page; ignored by {@code listAll}. */
    public ScheduleListFilter cursor(String value) {
        this.cursor = value;
        return this;
    }

    /** The page size. */
    public ScheduleListFilter limit(Integer value) {
        this.limit = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public ScheduleListFilter traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    /** {@code ACTIVE} or {@code PAUSED}. */
    public ScheduleListFilter state(String value) {
        this.state = value;
        return this;
    }

    /** {@code API} or {@code TRIGGER}. */
    public ScheduleListFilter origin(String value) {
        this.origin = value;
        return this;
    }

    /** The schedules that start this workflow; a UUID. */
    public ScheduleListFilter workflowId(String value) {
        this.workflowId = value;
        return this;
    }

    /** The schedules whose name starts with this text. */
    public ScheduleListFilter name(String value) {
        this.name = value;
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

    public String state() {
        return state;
    }

    public String origin() {
        return origin;
    }

    public String workflowId() {
        return workflowId;
    }

    public String name() {
        return name;
    }
}
