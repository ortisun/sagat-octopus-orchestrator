package dev.sagatoctopusorchestrator.sdk.workflow;

/** The options of a {@code saga} node. */
public final class SagaOptions {
    /** What happens when a compensation fails for good. */
    public enum OnCompensationFailure {
        MANUAL_REVIEW,
        FAIL;

        String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    /** What happens to the steps of the saga when the run is cancelled. */
    public enum CancelPolicy {
        COMPENSATE_COMPLETED,
        ABORT_NO_COMPENSATION,
        WAIT_IN_FLIGHT_THEN_COMPENSATE;

        String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    OnCompensationFailure onCompensationFailure;
    CancelPolicy cancelPolicy;

    public static SagaOptions create() {
        return new SagaOptions();
    }

    public SagaOptions onCompensationFailure(OnCompensationFailure value) {
        this.onCompensationFailure = value;
        return this;
    }

    public SagaOptions cancelPolicy(CancelPolicy value) {
        this.cancelPolicy = value;
        return this;
    }
}
