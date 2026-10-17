package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;
import java.util.List;

/** One approval request visible on a run, without eligibility or actor disclosure. */
public record ApprovalListItem(
        String approvalId,
        String runId,
        String nodeId,
        List<ApprovalScope> instancePath,
        ApprovalStatus status,
        Instant deadline,
        Instant settledAt,
        JsonNode raw) {}
