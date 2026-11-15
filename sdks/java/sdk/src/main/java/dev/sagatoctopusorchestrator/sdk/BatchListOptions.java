package dev.sagatoctopusorchestrator.sdk;

/** The filter and page of {@link BatchOperations#list} and {@link BatchOperations#listItems}. A fluent, mutable holder. */
public final class BatchListOptions {
    private String cursor;
    private Integer limit;
    private String traceparent;
    private String state;

    public static BatchListOptions create() {
        return new BatchListOptions();
    }

    /** The {@code nextCursor} of the previous page. */
    public BatchListOptions cursor(String value) {
        this.cursor = value;
        return this;
    }

    /** The page size. */
    public BatchListOptions limit(Integer value) {
        this.limit = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public BatchListOptions traceparent(String value) {
        this.traceparent = value;
        return this;
    }

    /** Only the jobs (or the items) in this state; the set of states only grows. */
    public BatchListOptions state(String value) {
        this.state = value;
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
}
