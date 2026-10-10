package dev.sagatoctopusorchestrator.sdk.worker;

import java.time.Duration;
import java.util.Optional;
import java.util.function.Consumer;

/**
 * The cancellation token of an activity. It is cancelled, once, with the reason {@code run-cancelled} (the run was
 * cancelled), {@code lease-lost} (the lease is gone) or {@code shutdown} (the host is stopping). The host never
 * interrupts the activity's thread: the activity decides what to do, and whatever it returns or raises is reported
 * (unless the lease was lost).
 */
public interface CancellationSignal {
    /** True once the signal was cancelled. */
    boolean isCancelled();

    /** Why it was cancelled; empty while it was not. */
    Optional<CancelReason> reason();

    /**
     * Waits until the signal is cancelled or the time passes, whichever comes first: the cancellable sleep.
     *
     * @return true when the signal was cancelled
     */
    boolean await(Duration timeout) throws InterruptedException;

    /** Waits until the signal is cancelled. */
    void awaitCancelled() throws InterruptedException;

    /** Runs the listener when the signal is cancelled, at once on the caller's thread if it already was. */
    void onCancel(Consumer<CancelReason> listener);

    /** Raises a {@code CANCELLED} {@link dev.sagatoctopusorchestrator.sdk.SagatException} if the signal was cancelled. */
    void throwIfCancelled();
}
