package dev.sagatoctopusorchestrator.sdk.workflow;

import java.time.Duration;

/** Finite lifetime shared by every run in a continuation chain. */
public record ChainPolicy(int maxSegments, Duration timeout) {
    /** Creates the required chain limits. */
    public static ChainPolicy of(int maxSegments, Duration timeout) {
        return new ChainPolicy(maxSegments, timeout);
    }
}
