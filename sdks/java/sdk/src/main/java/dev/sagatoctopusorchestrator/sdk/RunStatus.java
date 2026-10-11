package dev.sagatoctopusorchestrator.sdk;

/** The status of a run. {@link #UNKNOWN} stands for a status a newer server added; {@link Run#statusText()} keeps it. */
public enum RunStatus {
    RUNNING,
    WAITING,
    PAUSED,
    CANCELLING,
    COMPENSATING,
    MANUAL_REVIEW,
    SUCCEEDED,
    FAILED,
    CANCELLED,
    TIMED_OUT,
    COMPENSATED,
    PARTIAL,
    COMPENSATION_FAILED,
    UNKNOWN;

    /** Whether the run has ended: {@code SUCCEEDED, FAILED, CANCELLED, TIMED_OUT, COMPENSATED, PARTIAL, COMPENSATION_FAILED}. */
    public boolean isTerminal() {
        return switch (this) {
            case SUCCEEDED, FAILED, CANCELLED, TIMED_OUT, COMPENSATED, PARTIAL, COMPENSATION_FAILED -> true;
            default -> false;
        };
    }

    static RunStatus of(String text) {
        try {
            return valueOf(text);
        } catch (IllegalArgumentException e) {
            return UNKNOWN;
        }
    }
}
