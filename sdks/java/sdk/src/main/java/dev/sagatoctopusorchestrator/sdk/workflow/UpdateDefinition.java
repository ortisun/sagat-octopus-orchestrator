package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;

/** An atomic declarative validator, state reducer and result projection. */
public record UpdateDefinition(
        JsonNode argsSchema,
        String validate,
        String state,
        String result,
        JsonNode resultSchema,
        String disclosure) {}
