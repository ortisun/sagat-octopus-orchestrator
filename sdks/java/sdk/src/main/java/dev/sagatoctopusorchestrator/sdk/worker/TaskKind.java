package dev.sagatoctopusorchestrator.sdk.worker;

/** What a task asks of an activity (protocol 1.2). */
public enum TaskKind {
    /** An ordinary step. A task with no kind is a forward task. */
    FORWARD,
    /** Undo the effect of {@link ActivityContext#targetNode()}. */
    COMPENSATION,
    /** Find out whether the effect of {@link ActivityContext#targetNode()} happened. */
    RECONCILIATION;

    static TaskKind of(String wire) {
        if ("COMPENSATION".equals(wire)) {
            return COMPENSATION;
        }
        if ("RECONCILIATION".equals(wire)) {
            return RECONCILIATION;
        }
        return FORWARD;
    }
}
