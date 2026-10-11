package dev.sagatoctopusorchestrator.sdk.worker;

/** Why the cancellation signal of an activity was cancelled. */
public enum CancelReason {
    /** The run was cancelled (the server said so on a heartbeat). */
    RUN_CANCELLED("run-cancelled"),
    /** The lease is gone: the activity's result will be dropped. */
    LEASE_LOST("lease-lost"),
    /** The host is stopping and the task did not finish in time. */
    SHUTDOWN("shutdown");

    private final String wire;

    CancelReason(String wire) {
        this.wire = wire;
    }

    /** The reason as SPEC section 6.4 writes it: {@code run-cancelled}, {@code lease-lost} or {@code shutdown}. */
    public String wire() {
        return wire;
    }
}
