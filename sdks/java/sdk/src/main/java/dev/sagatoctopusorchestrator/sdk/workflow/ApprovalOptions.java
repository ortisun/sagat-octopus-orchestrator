package dev.sagatoctopusorchestrator.sdk.workflow;

import java.time.Duration;

/** Policy, deadline and routes of a native approval. */
public record ApprovalOptions(
        ApprovalPolicy policy,
        Duration timeout,
        String onApprove,
        String onReject,
        String onExpire) {}
