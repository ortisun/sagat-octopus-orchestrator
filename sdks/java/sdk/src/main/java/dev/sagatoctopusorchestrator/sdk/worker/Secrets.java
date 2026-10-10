package dev.sagatoctopusorchestrator.sdk.worker;

import dev.sagatoctopusorchestrator.sdk.ErrorCode;
import dev.sagatoctopusorchestrator.sdk.SagatException;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;

/**
 * The secrets a step declares, resolved for one attempt (SPEC section 6.7): a read-only map of name to bytes. The
 * values are credentials. Never log them, never put them in an error; the host overwrites them when the handler
 * returns.
 */
public final class Secrets {
    private final Map<String, byte[]> values = new LinkedHashMap<>();
    private boolean dropped;

    Secrets(Map<String, byte[]> values) {
        this.values.putAll(values);
    }

    /** The names of the secrets the step declares. */
    public synchronized Set<String> names() {
        return Set.copyOf(values.keySet());
    }

    /** The bytes of a secret (a copy). */
    public synchronized byte[] get(String name) {
        byte[] value = values.get(name);
        if (value == null) {
            throw SagatException.of(ErrorCode.NOT_FOUND, dropped
                    ? "the secrets of this attempt were dropped"
                    : "the step declares no secret named " + name);
        }
        return value.clone();
    }

    /** A secret as UTF-8 text. */
    public String text(String name) {
        return new String(get(name), StandardCharsets.UTF_8);
    }

    /** Overwrites the values; called by the host when the handler returns. */
    synchronized void drop() {
        dropped = true;
        values.values().forEach(value -> Arrays.fill(value, (byte) 0));
        values.clear();
    }

    @Override
    public String toString() {
        return "Secrets[names=" + values.keySet() + "]";
    }
}
