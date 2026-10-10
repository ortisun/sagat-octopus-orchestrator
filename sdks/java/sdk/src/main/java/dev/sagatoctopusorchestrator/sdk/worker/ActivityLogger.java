package dev.sagatoctopusorchestrator.sdk.worker;

/**
 * A logger an activity can use; every line is tagged with the task id. It writes to the host's
 * {@link System.Logger}. Never log a payload, a token or a secret.
 */
public interface ActivityLogger {
    void log(System.Logger.Level level, String message);

    default void debug(String message) {
        log(System.Logger.Level.DEBUG, message);
    }

    default void info(String message) {
        log(System.Logger.Level.INFO, message);
    }

    default void warn(String message) {
        log(System.Logger.Level.WARNING, message);
    }

    default void error(String message) {
        log(System.Logger.Level.ERROR, message);
    }
}
