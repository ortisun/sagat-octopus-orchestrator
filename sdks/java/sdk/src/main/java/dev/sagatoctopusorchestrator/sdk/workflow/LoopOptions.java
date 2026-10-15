package dev.sagatoctopusorchestrator.sdk.workflow;

import java.time.Duration;

/** Required bounds and optional routing of a sequential workflow loop. */
public record LoopOptions(int maxIterations, Duration timeout, Duration delay, String onError) {
    /** Creates loop bounds without an inter-iteration delay or error route. */
    public static LoopOptions of(int maxIterations, Duration timeout) {
        return new LoopOptions(maxIterations, timeout, null, null);
    }

    /** Returns these bounds with a durable delay between iterations that are still needed. */
    public LoopOptions withDelay(Duration delay) {
        return new LoopOptions(maxIterations, timeout, delay, onError);
    }

    /** Returns these bounds with a node to follow when the loop fails. */
    public LoopOptions withOnError(String onError) {
        return new LoopOptions(maxIterations, timeout, delay, onError);
    }
}
