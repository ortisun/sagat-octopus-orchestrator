package dev.sagatoctopusorchestrator.sdk;

/** An operation on a run ({@code POST /runs/{id}/operations}). */
public enum OperateAction {
    PAUSE,
    RESUME,
    CANCEL,
    RETRY_STEP,
    RETRY_COMPENSATION,
    SKIP_COMPENSATION,
    RESOLVE_STEP
}
