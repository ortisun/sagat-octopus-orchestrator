package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.List;

/** A page of a run's journal. {@code nextCursor} is null on the last page; {@code raw} is the answer as sent. */
public record JournalPage(List<JournalEvent> items, String nextCursor, JsonNode raw) {}
