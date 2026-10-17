package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;

/**
 * The answer to an operation on a run (cancel, pause, resume, rerun, ...).
 *
 * @param operationId the id of the operation
 * @param accepted    whether the server accepted it
 * @param runStatus   the run's status after it (nullable)
 * @param runVersion  the run's version after it (nullable)
 * @param newRunId    for {@link OperateAction#RERUN_AS_NEW}, the run that was started, which names its origin in
 *                    {@link Run#rerunOf()}; null for any other action
 * @param raw         the answer as sent
 */
public record OperationResult(
        String operationId, boolean accepted, String runStatus, Long runVersion, String newRunId, JsonNode raw) {

    /** An answer that starts no run. */
    public OperationResult(String operationId, boolean accepted, String runStatus, Long runVersion, JsonNode raw) {
        this(operationId, accepted, runStatus, runVersion, null, raw);
    }
}
