package dev.sagatoctopusorchestrator.sdk;

import java.time.Duration;

/** Options of {@link SagatClient#waitForCompletion}. */
public final class WaitOptions {
    private Duration timeout = Duration.ofSeconds(60);
    private Duration pollInterval = Duration.ofMillis(500);

    public static WaitOptions create() {
        return new WaitOptions();
    }

    /** How long to wait for a terminal status; 60 s by default. */
    public WaitOptions timeout(Duration value) {
        this.timeout = value;
        return this;
    }

    /** The wait between two reads, 500 ms by default, with 20 % jitter either way. */
    public WaitOptions pollInterval(Duration value) {
        this.pollInterval = value;
        return this;
    }

    public Duration timeout() {
        return timeout;
    }

    public Duration pollInterval() {
        return pollInterval;
    }
}
