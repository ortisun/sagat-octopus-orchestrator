package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/** A declared query result and the committed revisions it observed. */
public record QueryResult(String runId, long runVersion, long stateRevision, JsonNode result, JsonNode raw) {}
