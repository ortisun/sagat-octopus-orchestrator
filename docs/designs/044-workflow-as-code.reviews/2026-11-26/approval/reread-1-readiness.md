# Targeted re-read of design 044 for approval: egm implementation readiness, first pass (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: f7ddcbbe2521bcb85bc6265e94dffd895fb4dded
Reviewer: egm-implementation-readiness, implementation-readiness standard, the text changed since round 11 only
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 11's Major 1 is
closed in substance; Nits 2 to 8 closed (2 mostly). Every finding below was fixed in the next commit.

## Major

1. Held deferral blobs and the tenant's count: the count query counts every uncommitted blob; state the rule.
2. The uncounted-expiry rule has no counter column and no stored delay.
3. A replay-check item claimed and never read never expires; the extension and cap are unstated.

## Nit

4. The alert's runbook must be a section of `docs/runbooks/alerts.md`; missing files.
5. The claim budget lacks its rows written.
6. The chain receipt route should be under `/namespaces/{namespaceId}/chains/{chainId}`.
7. `run.cancel_requested` is a journal event, not an AsyncAPI message; name its contract home.
8. "Keep the source's gated entries" should say equal; the republished-envelope scenario.
9. The children-entry refusal and the union classification have no implementing task.

Counts: Blocker 0, Major 3, Nit 6
