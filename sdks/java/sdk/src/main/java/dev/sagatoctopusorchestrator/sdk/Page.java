package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/**
 * One page of a list, as the server gave it. {@code nextCursor} is opaque and null on the last page; {@code raw} is
 * the answer as sent.
 *
 * @param items      the items of the page
 * @param nextCursor where the next page starts (nullable)
 * @param raw        the answer as sent
 * @param <T>        the type of the items
 */
public record Page<T>(List<T> items, String nextCursor, JsonNode raw) {
    /**
     * The debug text of a page: its items (each prints itself) and the cursor. {@code raw} is left out: it repeats every
     * item as sent, and an item may carry customer text that its own text hides (the business-id prefix of a schedule).
     */
    @Override
    public String toString() {
        return "Page[items=" + items + ", nextCursor=" + nextCursor + "]";
    }
}
