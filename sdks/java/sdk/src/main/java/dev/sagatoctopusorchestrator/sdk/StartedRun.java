package dev.sagatoctopusorchestrator.sdk;

/**
 * A run as {@link SagatClient#start} answers it. {@code started} is false when an open run already held the business
 * identifier and {@link BusinessIdConflictPolicy#USE_EXISTING} was asked: the server answered 200 with that run, and
 * this call created nothing. A call that never sends a conflict policy never sees the 200.
 *
 * <p>{@link Run} is a final record, so the run is held here rather than extended.
 *
 * @param run     the run
 * @param started whether this call created the run (HTTP 202); false when it answered an existing one (HTTP 200)
 */
public record StartedRun(Run run, boolean started) {

    /** The id of the run: {@code run().runId()}. */
    public String runId() {
        return run.runId();
    }
}
