package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * A run, as the server describes it. {@code raw} is the answer as the server sent it, so a field added by a newer
 * server is reachable. Components marked nullable are null when the server sent none.
 *
 * @param runId         the run's id
 * @param workflowId    the workflow's id
 * @param versionId     the id of the workflow version the run executes
 * @param status        the status; {@link RunStatus#UNKNOWN} for one this SDK does not know
 * @param statusText    the status exactly as the server wrote it
 * @param version       number of journal events applied; usable as {@code expectedRunVersion}
 * @param paused        whether the run is paused
 * @param startedBy     the identity that started it (nullable)
 * @param createdAt     when it was created (UTC)
 * @param updatedAt     when it last changed (nullable)
 * @param finishedAt    when it ended (nullable)
 * @param deadline      the instant at which it times out (nullable)
 * @param traceId       the run's trace id (nullable)
 * @param correlationId the caller's correlation id (nullable)
 * @param depth         0 for a top-level run, the nesting depth for a child
 * @param parent        the parent run and node of a child run (nullable)
 * @param chain         the run's segment in its continuation chain (nullable: a run outside any chain has none). A
 *                      segment that is {@code SUCCEEDED} with completion kind {@code CONTINUED} ended into its
 *                      successor: it is not the process outcome, which is the chain's final outcome
 * @param raw           the answer as sent
 */
public record Run(
        String runId,
        String workflowId,
        String versionId,
        RunStatus status,
        String statusText,
        long version,
        boolean paused,
        String startedBy,
        Instant createdAt,
        Instant updatedAt,
        Instant finishedAt,
        Instant deadline,
        String traceId,
        String correlationId,
        int depth,
        Parent parent,
        ChainSegmentOfRun chain,
        JsonNode raw) {

    /** The parent of a child run. */
    public record Parent(String runId, String node, String cancelPolicy) {}

    /**
     * Where a run sits in its continuation chain.
     *
     * @param chainId        the chain, which is its first run's id
     * @param segmentIndex   0-based position of the run in the chain
     * @param completionKind {@code CONTINUED} or {@code FINAL}; null while the segment is open
     * @param successorRunId the next segment when the run was continued (nullable)
     */
    public record ChainSegmentOfRun(String chainId, int segmentIndex, String completionKind, String successorRunId) {}

    /** Whether the run has ended. */
    public boolean isTerminal() {
        return status.isTerminal();
    }
}
