package dev.sagatoctopusorchestrator.sdk;

import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.Locale;

/** URL helpers shared by the REST client and the worker host. */
public final class Endpoints {
    private static final char[] HEX = "0123456789ABCDEF".toCharArray();

    private Endpoints() {}

    /** True for {@code localhost}, {@code 127.0.0.0/8} and {@code ::1}. */
    public static boolean isLoopback(String host) {
        if (host == null) {
            return false;
        }
        String h = host.toLowerCase(Locale.ROOT);
        return h.equals("localhost") || h.equals("[::1]") || h.equals("::1") || h.startsWith("127.");
    }

    /** Logs one warning when a plain-text URL is used outside loopback (SPEC section 1). */
    public static void warnIfPlainText(System.Logger logger, String url, String what) {
        URI uri;
        try {
            uri = URI.create(url);
        } catch (IllegalArgumentException e) {
            return;
        }
        // Only the host is named: a URL can carry a credential in its user info or its query.
        if ("http".equalsIgnoreCase(uri.getScheme()) && uri.getHost() != null && !isLoopback(uri.getHost())) {
            logger.log(System.Logger.Level.WARNING, what + " uses plain text outside loopback (" + uri.getHost() + "): use https");
        }
    }

    /** Percent-encodes a path segment or a query value like JavaScript's {@code encodeURIComponent}. */
    public static String encodeComponent(String value) {
        byte[] bytes = value.getBytes(StandardCharsets.UTF_8);
        StringBuilder out = new StringBuilder(bytes.length + 8);
        for (byte b : bytes) {
            int c = b & 0xff;
            boolean unreserved = (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= '0' && c <= '9')
                    || c == '-' || c == '_' || c == '.' || c == '!' || c == '~' || c == '*' || c == '\'' || c == '(' || c == ')';
            if (unreserved) {
                out.append((char) c);
            } else {
                out.append('%').append(HEX[c >> 4]).append(HEX[c & 0xf]);
            }
        }
        return out.toString();
    }
}
