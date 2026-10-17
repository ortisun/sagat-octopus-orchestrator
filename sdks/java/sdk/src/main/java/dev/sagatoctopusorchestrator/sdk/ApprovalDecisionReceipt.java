package dev.sagatoctopusorchestrator.sdk;

import com.fasterxml.jackson.databind.JsonNode;
import java.time.Instant;

/** The durable receipt for an approval decision. */
public record ApprovalDecisionReceipt(
        String receiptId,
        String approvalId,
        String decisionKey,
        ApprovalDecision decision,
        String reasonCode,
        String actor,
        Instant decidedAt,
        long outcomeSequence,
        boolean replayed,
        JsonNode raw) {}
