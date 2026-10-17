package dev.sagatoctopusorchestrator.sdk;

/** Options for one page of a chain's runs. */
public final class ChainRunListOptions {
    private String cursor;
    private Integer limit;
    private String traceparent;

    public static ChainRunListOptions create() { return new ChainRunListOptions(); }
    /** The {@code nextCursor} of the previous page; opaque. */
    public ChainRunListOptions cursor(String value) { cursor = value; return this; }
    /** 1 to 200; the server clamps values outside that range. */
    public ChainRunListOptions limit(Integer value) { limit = value; return this; }
    public ChainRunListOptions traceparent(String value) { traceparent = value; return this; }
    public String cursor() { return cursor; }
    public Integer limit() { return limit; }
    public String traceparent() { return traceparent; }
}
