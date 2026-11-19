# Security re-read of design 041 on the owner's approval path

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 46c42bbbfc2f2c1cc918970086990fb590020ce4
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a fresh `security-reviewer`, asked for a short targeted re-read of the changes of resolution.md section 12 (OD-2026-11-16-7), given its own `security-approval-3.md`; no shell. Recorded condensed.

Third-run Major 1 (the PII grant against feature 016's default) fixed: the grant on every read, consistently stated and tested (T041-11, T041-13, T041-29). Third-run Nits 1, 2, 5 and 8 closed.

## Blocker

None.

## Major

None. Checked: the ingest's unlocked authority reads (the window is the batches in flight at a revocation, accepted by the owner); the marker's own transaction; grants and the one new definer function; what a caller can learn.

## Nit

1. The order of the grant check against `stream` is not stated (a 403 against a 404 could tell whether a run has a log).
2. T041-29 does not test the listing's `logTruncated`.
3. The lock-free assertion of T7 names only the secrets lock functions, not `hold_worker_binding` or `sagat_lock_memberships`.
4. ADR-0085's Consequences still says "in classified namespaces".

Counts: Blocker 0, Major 0, Nit 4
