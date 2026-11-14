package dev.sagatoctopusorchestrator.sdk;

/** Options of {@link Schedules#get}: how much of the future and the past to describe. */
public final class ScheduleGetOptions {
    private Integer nextCount;
    private Integer recentCount;
    private String traceparent;

    public static ScheduleGetOptions create() {
        return new ScheduleGetOptions();
    }

    /** How many upcoming times to describe; 0 asks for none. Unset is the server's default. */
    public ScheduleGetOptions nextCount(Integer value) {
        this.nextCount = value;
        return this;
    }

    /** How many latest decisions to describe; 0 asks for none. Unset is the server's default. */
    public ScheduleGetOptions recentCount(Integer value) {
        this.recentCount = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public ScheduleGetOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    public Integer nextCount() {
        return nextCount;
    }

    public Integer recentCount() {
        return recentCount;
    }

    public String traceparent() {
        return traceparent;
    }
}
