package dev.sagatoctopusorchestrator.sdk;

/** Options of a call that lists a page: where to start, how many, and a trace context. */
public final class PageOptions {
    private String cursor;
    private Integer limit;
    private String traceparent;

    public static PageOptions create() {
        return new PageOptions();
    }

    /** The {@code nextCursor} of the previous page. */
    public PageOptions cursor(String value) {
        this.cursor = value;
        return this;
    }

    /** The page size. */
    public PageOptions limit(Integer value) {
        this.limit = value;
        return this;
    }

    /** A W3C {@code traceparent}; wins over the ambient trace context. */
    public PageOptions traceparent(String value) {
        this.traceparent = value;
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
}
