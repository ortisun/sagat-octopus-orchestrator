package dev.sagatoctopusorchestrator.sdk.workflow;

/** The options of an {@code invokeChild} node. */
public final class ChildOptions {
    /** What happens to the child when the parent is cancelled. */
    public enum CancelPolicy {
        PROPAGATE,
        DETACH,
        REQUEST_CANCEL;

        String wire() {
            return name().toLowerCase(java.util.Locale.ROOT);
        }
    }

    Integer version;
    String input;
    CancelPolicy cancelPolicy;

    public static ChildOptions create() {
        return new ChildOptions();
    }

    /** Pins a published version number; the workflow's active version when absent. */
    public ChildOptions version(int value) {
        this.version = value;
        return this;
    }

    /** An expression over the run's data that yields the child's input. */
    public ChildOptions input(String value) {
        this.input = value;
        return this;
    }

    public ChildOptions cancelPolicy(CancelPolicy value) {
        this.cancelPolicy = value;
        return this;
    }
}
