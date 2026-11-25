package dev.sagatoctopusorchestrator.sdk.worker;

import java.time.Duration;
import java.util.function.BooleanSupplier;

/**
 * The barrier of an activity under test: the handler holds its task until what the test claims has happened (three
 * heartbeats, a beat after the token provider recovered) instead of sleeping for a time it hopes is long enough
 * (AGENTS.md Rule 18). It does not fail by itself and gives up before the wait of the test does, so the assertion of
 * the test stays the one that says what is missing.
 */
final class Barrier {
    private static final Duration LIMIT = Duration.ofSeconds(5);

    private Barrier() {}

    static boolean reached(BooleanSupplier condition) {
        long end = System.nanoTime() + LIMIT.toNanos();
        while (!condition.getAsBoolean()) {
            if (System.nanoTime() > end) {
                return false;
            }
            try {
                Thread.sleep(5);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
                return false;
            }
        }
        return true;
    }
}
