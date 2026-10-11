package dev.sagatoctopusorchestrator.sdk.worker;

import java.util.Objects;

/**
 * How an activity fails on purpose (SPEC section 6.5). {@code code} is a stable machine code ({@code [A-Z0-9_]{1,64}},
 * anything else is reported as {@code WORKER_ERROR}); {@code message} is recorded with the step, so it must not carry
 * a payload or a secret. Any other exception an activity raises is reported as {@code ACTIVITY_ERROR}, retryable,
 * with only the exception's type name as message.
 */
public class ActivityFailure extends RuntimeException {
    private static final long serialVersionUID = 1L;

    private final String code;
    private final boolean retryable;

    public ActivityFailure(String code, String message, boolean retryable) {
        super(message == null ? "" : message);
        this.code = Objects.requireNonNull(code, "code");
        this.retryable = retryable;
    }

    /** The step may be tried again as its node's retry policy says. */
    public static ActivityFailure retryable(String code, String message) {
        return new ActivityFailure(code, message, true);
    }

    /** The step is failed for good: the policy goes to its error branch or its saga compensates. */
    public static ActivityFailure permanent(String code, String message) {
        return new ActivityFailure(code, message, false);
    }

    /**
     * The activity sent a request and cannot tell whether it was applied (a timeout after connecting, an answer that
     * never arrived). The platform then reconciles the step, when it has a {@code reconcileWith}, instead of retrying
     * it blindly. Code {@code OUTCOME_UNKNOWN}, not retryable.
     */
    public static ActivityFailure outcomeUnknown(String message) {
        return new ActivityFailure("OUTCOME_UNKNOWN", message, false);
    }

    /** The machine code of the failure. */
    public String code() {
        return code;
    }

    /** Whether the server's retry policy may try the step again. */
    public boolean isRetryable() {
        return retryable;
    }
}
