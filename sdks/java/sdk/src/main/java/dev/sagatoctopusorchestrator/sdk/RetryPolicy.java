package dev.sagatoctopusorchestrator.sdk;

import java.time.Duration;
import java.util.Objects;
import java.util.function.DoubleSupplier;

/**
 * How many times, and how patiently, a call that is safe to repeat is repeated (SPEC section 5): exponential
 * backoff with full jitter, {@code min(maxDelay, initialDelay * 2^n)} scaled by a random factor in [0.5, 1.0].
 *
 * @param maxAttempts  attempts in all, the first included
 * @param initialDelay the wait after the first failed attempt, before jitter
 * @param maxDelay     the longest wait, before jitter
 */
public record RetryPolicy(int maxAttempts, Duration initialDelay, Duration maxDelay) {
    /** 4 attempts, 200 ms, 5 s. */
    public static final RetryPolicy DEFAULT = new RetryPolicy(4, Duration.ofMillis(200), Duration.ofSeconds(5));

    public RetryPolicy {
        Objects.requireNonNull(initialDelay, "initialDelay");
        Objects.requireNonNull(maxDelay, "maxDelay");
        if (maxAttempts < 1) {
            throw new IllegalArgumentException("maxAttempts is at least 1");
        }
    }

    public RetryPolicy withMaxAttempts(int value) {
        return new RetryPolicy(value, initialDelay, maxDelay);
    }

    public RetryPolicy withInitialDelay(Duration value) {
        return new RetryPolicy(maxAttempts, value, maxDelay);
    }

    public RetryPolicy withMaxDelay(Duration value) {
        return new RetryPolicy(maxAttempts, initialDelay, value);
    }

    /** The wait after the attempt {@code n} (from 0) failed: exponential, capped, jittered. */
    public long backoffMillis(int n, DoubleSupplier random) {
        double ceiling = Math.min((double) maxDelay.toMillis(), initialDelay.toMillis() * Math.pow(2, Math.min(n, 30)));
        return Math.round(ceiling * (0.5 + random.getAsDouble() * 0.5));
    }

    /**
     * The wait before the next attempt: the backoff, or what the server asked for if that is longer, which is never
     * more than {@link SagatException#MAX_SERVER_WAIT_MS} (a minute).
     */
    public long delayAfterMillis(int n, SagatException error, DoubleSupplier random) {
        return delayAfterMillis(n, error, SagatException.MAX_SERVER_WAIT_MS, random);
    }

    /** {@link #delayAfterMillis(int, SagatException, DoubleSupplier)} with a lower cap for what the server asks for. */
    public long delayAfterMillis(int n, SagatException error, long maxServerWaitMillis, DoubleSupplier random) {
        long asked = Math.min(error.retryAfterMs().orElse(0L), maxServerWaitMillis);
        return Math.max(backoffMillis(n, random), asked);
    }

    /** {@link #backoffMillis(int, DoubleSupplier)} with the thread-local random. */
    public long backoffMillis(int n) {
        return backoffMillis(n, () -> java.util.concurrent.ThreadLocalRandom.current().nextDouble());
    }
}
