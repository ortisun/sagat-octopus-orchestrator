package dev.sagatoctopusorchestrator.sdk.workflow;

import com.fasterxml.jackson.databind.JsonNode;
import java.util.Map;

/** Bounded business state and its named side-effect-free operations. */
public record StateDefinition(
        JsonNode schema,
        String initial,
        Map<String, QueryDefinition> queries,
        Map<String, UpdateDefinition> updates) {}
