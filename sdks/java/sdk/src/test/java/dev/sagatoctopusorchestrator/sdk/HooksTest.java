package dev.sagatoctopusorchestrator.sdk;

import static org.junit.jupiter.api.Assertions.assertDoesNotThrow;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.ArrayList;
import java.util.List;
import org.junit.jupiter.api.Test;

/** What the SDK survives when the code of the caller or a runtime observer fails (SPEC section 10). */
class HooksTest {
    private static int depth(int n) {
        return depth(n + 1) + 1;
    }

    @Test
    void onlyOutOfMemoryIsFatalEverythingElseIsSurvived() {
        assertFalse(Hooks.survivable(new OutOfMemoryError()));
        for (Throwable survivable : new Throwable[] {
            new StackOverflowError(), new InternalError("x"), new UnknownError(), new AssertionError("x"), new LinkageError("x"), new NoClassDefFoundError("x"),
            new IllegalStateException("x"), new Exception("x")}) {
            assertTrue(Hooks.survivable(survivable), survivable.getClass().getSimpleName());
        }
        assertThrows(OutOfMemoryError.class, () -> Hooks.rethrowIfFatal(new OutOfMemoryError("heap")));
        assertDoesNotThrow(() -> Hooks.rethrowIfFatal(new StackOverflowError()));
    }

    @Test
    void aLoggerOrACallbackThatOverflowsTheStackIsSurvived() {
        System.Logger deep = new System.Logger() {
            @Override
            public String getName() {
                return "deep";
            }

            @Override
            public boolean isLoggable(Level level) {
                return true;
            }

            @Override
            public void log(Level level, java.util.ResourceBundle bundle, String msg, Throwable thrown) {
                depth(0);
            }

            @Override
            public void log(Level level, java.util.ResourceBundle bundle, String format, Object... params) {
                depth(0);
            }
        };
        assertDoesNotThrow(() -> Hooks.log(deep, System.Logger.Level.WARNING, "x"), "a real StackOverflowError from the sink");
        List<String> seen = new ArrayList<>();
        assertDoesNotThrow(() -> Hooks.call(value -> depth(0), "x"));
        Hooks.call(seen::add, "after");
        assertEquals(List.of("after"), seen, "and the next call works");
    }

    @Test
    void anOutOfMemoryErrorOfAHookIsNotSwallowed() {
        assertThrows(OutOfMemoryError.class, () -> Hooks.call(value -> {
            throw new OutOfMemoryError("heap");
        }, "x"));
        assertThrows(OutOfMemoryError.class, () -> Hooks.log(new ThrowingLogger(() -> new OutOfMemoryError("heap")).armed(true), System.Logger.Level.WARNING, "x"));
    }
}
