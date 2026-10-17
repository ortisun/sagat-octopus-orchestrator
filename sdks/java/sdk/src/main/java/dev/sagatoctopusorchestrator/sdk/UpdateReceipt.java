package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/** The durable result of an applied or replayed declared update. */
public record UpdateReceipt(
        String runId,
        String operation,
        String updateKey,
        String status,
        JsonNode result,
        long beforeStateRevision,
        long afterStateRevision,
        long runVersion,
        boolean replayed,
        JsonNode raw) {}
