# Targeted re-read 9 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 1f07592aee8297f7da940a2decd6bf5b89d1699d
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`, at 66634fb9, 6
Majors and 11 Nits), then the fixes re-read in `reread-1-security.md` to `reread-8-security.md`, every finding of each
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 8's Nit is resolved: Task 049-14 tests both isolation refusals (`journal_retire_start` called directly as
`sagat_retirer`, and the dated root of an ended month) at `REPEATABLE READ` and at `SERIALIZABLE`; both rules are written
as "refuse anything but `READ COMMITTED`", and `READ UNCOMMITTED` runs as `READ COMMITTED` in PostgreSQL. The change
only adds test cases and opens no tenant, privilege, data-exposure or race risk.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
