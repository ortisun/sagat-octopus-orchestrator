package dev.sagatoctopusorchestrator.sdk;

import java.security.SecureRandom;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/** W3C {@code traceparent} helpers (version 00). */
public final class Traceparents {
    private static final Pattern PATTERN = Pattern.compile("^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$");
    private static final SecureRandom RANDOM = new SecureRandom();
    private static final char[] HEX = "0123456789abcdef".toCharArray();

    private Traceparents() {}

    /** True for a version 00 {@code traceparent} with a non-zero trace id and parent id. */
    public static boolean isValid(String value) {
        if (value == null) {
            return false;
        }
        Matcher match = PATTERN.matcher(value);
        return match.matches() && !match.group(1).equals("0".repeat(32)) && !match.group(2).equals("0".repeat(16));
    }

    /** Raises {@code BAD_REQUEST} for a malformed {@code traceparent}, before anything is sent. */
    public static String check(String value) {
        if (!isValid(value)) {
            throw SagatException.of(
                    ErrorCode.BAD_REQUEST, "traceparent is not a W3C trace context (00-<32 hex>-<16 hex>-<2 hex>)");
        }
        return value;
    }

    /** The trace id of a valid {@code traceparent}; null otherwise. */
    public static String traceIdOf(String value) {
        if (value == null) {
            return null;
        }
        Matcher match = PATTERN.matcher(value);
        return match.matches() ? match.group(1) : null;
    }

    /** A fresh sampled trace context, for an application that has none. */
    public static String create() {
        return "00-" + randomHex(16) + "-" + randomHex(8) + "-01";
    }

    private static String randomHex(int bytes) {
        byte[] data = new byte[bytes];
        RANDOM.nextBytes(data);
        char[] out = new char[bytes * 2];
        for (int i = 0; i < bytes; i++) {
            out[2 * i] = HEX[(data[i] >> 4) & 0xf];
            out[2 * i + 1] = HEX[data[i] & 0xf];
        }
        return new String(out);
    }
}
