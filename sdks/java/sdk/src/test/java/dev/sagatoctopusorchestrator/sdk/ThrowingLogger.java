package dev.sagatoctopusorchestrator.sdk;

import java.util.ResourceBundle;
import java.util.function.Supplier;

/** A {@link System.Logger} whose sink is down: it throws on WARNING and above once armed. */
public final class ThrowingLogger implements System.Logger {
    private volatile boolean armed;
    private final Supplier<Throwable> failure;

    public ThrowingLogger() {
        this(() -> new IllegalStateException("log sink down"));
    }

    public ThrowingLogger(Supplier<Throwable> failure) {
        this.failure = failure;
    }

    public ThrowingLogger armed(boolean value) {
        this.armed = value;
        return this;
    }

    @Override
    public String getName() {
        return "throwing";
    }

    @Override
    public boolean isLoggable(Level level) {
        return true;
    }

    private void maybeThrow(Level level) {
        if (armed && level.getSeverity() >= Level.WARNING.getSeverity()) {
            Throwable t = failure.get();
            if (t instanceof RuntimeException r) {
                throw r;
            }
            if (t instanceof Error e) {
                throw e;
            }
            throw new IllegalStateException(t);
        }
    }

    @Override
    public void log(Level level, ResourceBundle bundle, String msg, Throwable thrown) {
        maybeThrow(level);
    }

    @Override
    public void log(Level level, ResourceBundle bundle, String format, Object... params) {
        maybeThrow(level);
    }
}
