package dev.sagatoctopusorchestrator.sdk;

import java.util.function.Consumer;

/**
 * Calls into the code of the caller and of the runtime's observers (a logger, a metrics callback, a trace hook) from
 * places that must not die of it: the poll loop, a heartbeat, a task in flight (SPEC section 10). Internal to the SDK.
 */
public final class Hooks {
    private Hooks() {}

    /**
     * Whether a failure is one the SDK survives. Everything is, a {@link StackOverflowError} included (a logger or a
     * callback that recurses must not end the poll loop), except {@link OutOfMemoryError}: with no memory left nothing
     * the SDK would do next is trustworthy, so it is rethrown.
     */
    public static boolean survivable(Throwable t) {
        return !(t instanceof OutOfMemoryError);
    }

    /** Rethrows a failure that is not survivable (an {@link OutOfMemoryError}); returns for any other. */
    public static void rethrowIfFatal(Throwable t) {
        if (t instanceof OutOfMemoryError oom) {
            throw oom;
        }
    }

    /** Writes a line to the logger of the caller; a logger that throws is ignored. */
    public static void log(System.Logger logger, System.Logger.Level level, String message) {
        try {
            logger.log(level, message);
        } catch (Throwable t) {
            rethrowIfFatal(t);
        }
    }

    /** Hands a value to a callback of the caller; a callback that throws is ignored (a metrics hook must not stop the host). */
    public static <T> void call(Consumer<T> callback, T value) {
        if (callback == null) {
            return;
        }
        try {
            callback.accept(value);
        } catch (Throwable t) {
            rethrowIfFatal(t);
        }
    }
}
