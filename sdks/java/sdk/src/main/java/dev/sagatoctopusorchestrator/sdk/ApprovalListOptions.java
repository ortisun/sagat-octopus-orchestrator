package dev.sagatoctopusorchestrator.sdk;

/** Options for one approval page. */
public final class ApprovalListOptions {
    private String cursor;
    private Integer limit;
    private String traceparent;

    public static ApprovalListOptions create() { return new ApprovalListOptions(); }
    public ApprovalListOptions cursor(String value) { cursor = value; return this; }
    public ApprovalListOptions limit(Integer value) { limit = value; return this; }
    public ApprovalListOptions traceparent(String value) { traceparent = value; return this; }
    public String cursor() { return cursor; }
    public Integer limit() { return limit; }
    public String traceparent() { return traceparent; }
}
