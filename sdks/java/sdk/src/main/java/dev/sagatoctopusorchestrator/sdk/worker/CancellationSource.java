package dev.sagatoctopusorchestrator.sdk.worker;

import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.time.Duration;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.TimeUnit;
import java.util.function.Consumer;

/** The writable side of a {@link CancellationSignal}: the first reason wins. */
final class CancellationSource implements CancellationSignal {
    private final CountDownLatch latch = new CountDownLatch(1);
    private final List<Consumer<CancelReason>> listeners = new ArrayList<>();
    private CancelReason reason;

    /** Cancels with a reason; false if it was cancelled already. */
    boolean cancel(CancelReason why) {
        List<Consumer<CancelReason>> toRun;
        synchronized (this) {
            if (reason != null) {
                return false;
            }
            reason = why;
            toRun = new ArrayList<>(listeners);
            listeners.clear();
        }
        latch.countDown();
        for (Consumer<CancelReason> listener : toRun) {
            run(listener, why);
        }
        return true;
    }

    private static void run(Consumer<CancelReason> listener, CancelReason why) {
        try {
            listener.accept(why);
        } catch (Throwable t) {
            // A listener that fails must not stop the others, nor the host (only the VM's own failures are rethrown).
            if (!dev.sagatoctopusorchestrator.sdk.Hooks.survivable(t)) {
                throw (VirtualMachineError) t;
            }
        }
    }

    @Override
    public boolean isCancelled() {
        return latch.getCount() == 0;
    }

    @Override
    public synchronized Optional<CancelReason> reason() {
        return Optional.ofNullable(reason);
    }

    @Override
    public boolean await(Duration timeout) throws InterruptedException {
        return latch.await(Math.max(0, timeout.toNanos()), TimeUnit.NANOSECONDS);
    }

    @Override
    public void awaitCancelled() throws InterruptedException {
        latch.await();
    }

    @Override
    public void onCancel(Consumer<CancelReason> listener) {
        CancelReason now;
        synchronized (this) {
            now = reason;
            if (now == null) {
                listeners.add(listener);
                return;
            }
        }
        run(listener, now);
    }

    @Override
    public void throwIfCancelled() {
        if (isCancelled()) {
            throw SagatException.of(ErrorCode.CANCELLED, "the activity was cancelled");
        }
    }
}
