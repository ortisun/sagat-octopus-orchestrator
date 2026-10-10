package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * The answer to an operation on a run (cancel, pause, resume, ...).
 *
 * @param operationId the id of the operation
 * @param accepted    whether the server accepted it
 * @param runStatus   the run's status after it (nullable)
 * @param runVersion  the run's version after it (nullable)
 * @param raw         the answer as sent
 */
public record OperationResult(String operationId, boolean accepted, String runStatus, Long runVersion, JsonNode raw) {}
