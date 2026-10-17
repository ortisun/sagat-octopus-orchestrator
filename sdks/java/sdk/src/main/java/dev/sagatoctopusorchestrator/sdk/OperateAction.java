package dev.sagatoctopusorchestrator.sdk;

/** An operation on a run ({@code POST /runs/{id}/operations}). */
public enum OperateAction {
    PAUSE,
    RESUME,
    CANCEL,
    RETRY_STEP,
    RETRY_COMPENSATION,
    SKIP_COMPENSATION,
    RESOLVE_STEP,
    /**
     * Starts a new run of the same version, with the same input and correlation id, from a run that ended (needs
     * {@code run:operate} and {@code run:start}; a run that has not ended is refused with {@code CONFLICT}).
     */
    RERUN_AS_NEW
}
