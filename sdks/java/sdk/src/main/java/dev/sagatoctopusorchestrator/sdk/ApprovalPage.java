package dev.sagatoctopusorchestrator.sdk;

import java.util.List;

/** One bounded approval page in immutable opening order. */
public record ApprovalPage(List<ApprovalListItem> items, String nextCursor) {}
