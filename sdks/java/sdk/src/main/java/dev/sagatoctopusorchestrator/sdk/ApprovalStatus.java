package dev.sagatoctopusorchestrator.sdk;

/** The lifecycle state of a native approval request. */
public enum ApprovalStatus {
    PENDING,
    APPROVED,
    REJECTED,
    EXPIRED,
    CANCELLED
}
