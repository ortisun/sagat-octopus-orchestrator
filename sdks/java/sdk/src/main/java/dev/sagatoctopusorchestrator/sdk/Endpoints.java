package dev.sagatoctopusorchestrator.sdk;

import java.net.URI;
import java.nio.charset.StandardCharsets;
import java.util.Locale;

/** URL helpers shared by the REST client and the worker host. */
public final class Endpoints {
    private static final char[] HEX = "0123456789ABCDEF".toCharArray();

    private Endpoints() {}

    /**
     * True for {@code localhost}, an IPv4 literal in 127.0.0.0/8 and the IPv6 literal {@code ::1}. A name is not an
     * address: {@code 127.attacker.example} and {@code 127.0.0.1.nip.io} are not loopback.
     */
    public static boolean isLoopback(String host) {
        if (host == null || host.isEmpty()) {
            return false;
        }
        if (host.equalsIgnoreCase("localhost")) {
            return true;
        }
        String h = host.startsWith("[") && host.endsWith("]") ? host.substring(1, host.length() - 1) : host;
        if (h.indexOf(':') >= 0) {
            return isIpv6Loopback(h);
        }
        return isIpv4Loopback(h);
    }

    private static boolean isIpv4Loopback(String h) {
        String[] parts = h.split("\\.", -1);
        if (parts.length != 4) {
            return false;
        }
        int first = -1;
        for (int i = 0; i < 4; i++) {
            String part = parts[i];
            if (part.isEmpty() || part.length() > 3 || (part.length() > 1 && part.charAt(0) == '0')) {
                return false;
            }
            int value = 0;
            for (int k = 0; k < part.length(); k++) {
                char c = part.charAt(k);
                if (c < '0' || c > '9') {
                    return false;
                }
                value = value * 10 + (c - '0');
            }
            if (value > 255) {
                return false;
            }
            if (i == 0) {
                first = value;
            }
        }
        return first == 127;
    }

    private static boolean isIpv6Loopback(String h) {
        // Only the forms of ::1 itself: eight groups of hex digits, or the abbreviation. No name resolution happens here.
        if (h.equals("::1")) {
            return true;
        }
        String[] groups = h.split(":", -1);
        if (groups.length != 8) {
            return false;
        }
        for (int i = 0; i < 8; i++) {
            String g = groups[i];
            if (g.isEmpty() || g.length() > 4) {
                return false;
            }
            for (int k = 0; k < g.length(); k++) {
                if (Character.digit(g.charAt(k), 16) < 0) {
                    return false;
                }
            }
            int value = Integer.parseInt(g, 16);
            if (i < 7 ? value != 0 : value != 1) {
                return false;
            }
        }
        return true;
    }

    /** The host of a URL, for a message or a string form: nothing else of it (it can carry a credential). */
    public static String hostOf(String url) {
        try {
            String host = URI.create(url).getHost();
            return host == null ? "?" : host;
        } catch (IllegalArgumentException e) {
            return "?";
        }
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
            Hooks.log(logger, System.Logger.Level.WARNING, what + " uses plain text outside loopback (" + uri.getHost() + "): use https");
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
