package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/**
 * One segment as the paged list of a chain's runs gives it: lineage and lifecycle, never a payload.
 *
 * @param index            0-based position in the chain
 * @param runId            the segment's run
 * @param status           the run's status as the server wrote it
 * @param completionKind   {@code CONTINUED} or {@code FINAL}; null while the segment is open
 * @param predecessorRunId the previous segment (nullable)
 * @param successorRunId   the next segment (nullable)
 * @param createdAt        when the run was created (UTC)
 * @param finishedAt       when it ended (nullable)
 * @param raw              the item as sent
 */
public record ChainRun(
        int index, String runId, String status, String completionKind, String predecessorRunId, String successorRunId,
        Instant createdAt, Instant finishedAt, JsonNode raw) {}
