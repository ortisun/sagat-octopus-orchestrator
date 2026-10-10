package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * Run in a child JVM with a small heap by the tests: what the SDK makes of a number with a hostile exponent. Prints
 * {@code encode=<length or error code> canonical=<length or error code>}.
 */
public final class SmallHeapProbe {
    private SmallHeapProbe() {}

    private static String describe(java.util.function.Supplier<String> write) {
        try {
            String text = write.get();
            return text.length() + ":" + (text.length() <= 80 ? text : text.substring(0, 80));
        } catch (SagatException e) {
            return "refused:" + e.code();
        }
    }

    public static void main(String[] args) {
        JsonNode node = Json.parse("{\"x\":" + args[0] + "}");
        System.out.println("encode=" + describe(() -> Json.encode(node)));
        System.out.println("canonical=" + describe(() -> Json.canonical(node)));
    }
}
