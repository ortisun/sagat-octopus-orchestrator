package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;

/** A bounded snapshot projection with an explicit result-disclosure policy. */
public record QueryDefinition(JsonNode argsSchema, JsonNode resultSchema, String expression, String disclosure) {}
