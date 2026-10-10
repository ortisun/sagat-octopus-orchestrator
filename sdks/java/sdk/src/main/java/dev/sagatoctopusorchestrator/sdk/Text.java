package dev.sagatoctopusorchestrator.sdk;

/** Text that leaves the SDK as an error or a report is bounded and always valid Unicode. */
public final class Text {
    private Text() {}

    /**
     * At most {@code maxCodePoints} Unicode code points of {@code text}: never cut inside a surrogate pair, and a lone
     * surrogate (which UTF-8 cannot carry) becomes U+FFFD, so that what is sent is always valid UTF-8 (SPEC section 6.5).
     * {@code null} is the empty string.
     */
    public static String cut(String text, int maxCodePoints) {
        if (text == null || text.isEmpty()) {
            return "";
        }
        StringBuilder out = null;
        int kept = 0;
        int i = 0;
        int n = text.length();
        while (i < n && kept < maxCodePoints) {
            char c = text.charAt(i);
            if (Character.isHighSurrogate(c) && i + 1 < n && Character.isLowSurrogate(text.charAt(i + 1))) {
                if (out != null) {
                    out.append(c).append(text.charAt(i + 1));
                }
                i += 2;
            } else if (Character.isSurrogate(c)) {
                if (out == null) {
                    out = new StringBuilder(Math.min(n, maxCodePoints * 2)).append(text, 0, i);
                }
                out.append('�');
                i++;
            } else {
                if (out != null) {
                    out.append(c);
                }
                i++;
            }
            kept++;
        }
        if (out != null) {
            return out.toString();
        }
        return i == n ? text : text.substring(0, i);
    }
}
