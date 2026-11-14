package dev.sagatoctopusorchestrator.sdk;

/**
 * What a schedule does when a run it started is still open at its next action. The server may add others: a policy
 * is a string, and one this SDK does not know is sent as given.
 */
public final class OverlapPolicy {
    public static final String SKIP = "SKIP";
    public static final String BUFFER_ONE = "BUFFER_ONE";
    public static final String BUFFER_ALL = "BUFFER_ALL";
    public static final String CANCEL_OTHER = "CANCEL_OTHER";
    public static final String ALLOW_ALL = "ALLOW_ALL";

    private OverlapPolicy() {}
}
