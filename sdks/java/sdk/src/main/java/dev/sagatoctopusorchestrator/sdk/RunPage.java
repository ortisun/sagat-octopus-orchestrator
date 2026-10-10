package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/**
 * A page of runs, newest first, as the server gave them. {@code nextCursor} is null on the last page; {@code raw} is
 * the answer as sent.
 */
public record RunPage(List<Run> items, String nextCursor, JsonNode raw) {}
