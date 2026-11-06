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
public record Page<T>(List<T> items, String nextCursor, JsonNode raw) {}
