# Security review of design 041, the second approval run

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fefcfafbb40d153ebd3e462c7ba2d94a7c0d9286
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a fresh `security-reviewer`, given the committed design, its companions, the ADRs, the constitution, the owner decisions and its own `security-approval.md`; no shell. Recorded condensed.

Previous findings 1 to 3: closed.

## Blocker

None.

## Major

None.

## Nit

1. `UPDATE` on `step_log_streams` lets a writer undo a delete or change retention; add a trigger that refuses changes to identity columns and `retention_days` and any removal or final timestamp going back to null.
2. The two `SECURITY DEFINER` accessors are not stated as revoked from `PUBLIC`.
3. The server scrub does not say it covers the exception, every key and every string value.
4. A revoked worker's `AUTHORITY` answer leaves no audit record.
5. `stored_through_sequence` can tell a revoked caller about the stream; send it only with `CONTINUE`, `FINAL_ONLY` and `FINAL_RECORDED`.
6. A live tail keeps sending for up to 30 s after a PII grant is revoked; owner acceptance or a per-poll unlocked grant check.
7. A filtered list read rescans the whole run on every poll.
8. Lines are served after `expires_at` until the purge marks the stream.
9. Redaction with the whole process's secret set confirms a guessed secret; register it as a gap.
10. (Suspicion) a constraint failure can put line text in PostgreSQL's own log (`DETAIL: Failing row contains`).

Counts: Blocker 0, Major 0, Nit 10
