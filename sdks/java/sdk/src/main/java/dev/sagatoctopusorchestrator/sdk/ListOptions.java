package dev.sagatoctopusorchestrator.sdk;

/** Options of {@link SagatClient#list} and {@link SagatClient#listAll}. */
public final class ListOptions {
    private String workflowId;
    private RunStatus status;
    private Integer limit;
    private String cursor;

    public static ListOptions create() {
        return new ListOptions();
    }

    public ListOptions workflowId(String value) {
        this.workflowId = value;
        return this;
    }

    public ListOptions status(RunStatus value) {
        this.status = value;
        return this;
    }

    /** The page size. */
    public ListOptions limit(Integer value) {
        this.limit = value;
        return this;
    }

    /** The {@code nextCursor} of the previous page; ignored by {@code listAll}. */
    public ListOptions cursor(String value) {
        this.cursor = value;
        return this;
    }

    public String workflowId() {
        return workflowId;
    }

    public RunStatus status() {
        return status;
    }

    public Integer limit() {
        return limit;
    }

    public String cursor() {
        return cursor;
    }
}
