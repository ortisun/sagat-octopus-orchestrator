package dev.sagatoctopusorchestrator.sdk;

import java.util.List;
import java.util.ResourceBundle;
import java.util.concurrent.CopyOnWriteArrayList;

/** A {@link System.Logger} that keeps what it is given, for the tests that check what the SDK says. */
public final class CapturingLogger implements System.Logger {
    /** One line. */
    public record Line(Level level, String message) {}

    private final List<Line> lines = new CopyOnWriteArrayList<>();

    public List<Line> lines() {
        return lines;
    }

    /** The lines at WARNING or above whose text contains {@code text}. */
    public List<Line> warnings(String text) {
        return lines.stream().filter(l -> l.level().getSeverity() >= Level.WARNING.getSeverity() && l.message().contains(text)).toList();
    }

    @Override
    public String getName() {
        return "capturing";
    }

    @Override
    public boolean isLoggable(Level level) {
        return true;
    }

    @Override
    public void log(Level level, ResourceBundle bundle, String message, Throwable thrown) {
        lines.add(new Line(level, message));
    }

    @Override
    public void log(Level level, ResourceBundle bundle, String format, Object... params) {
        lines.add(new Line(level, format));
    }
}
