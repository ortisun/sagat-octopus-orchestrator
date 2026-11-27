# Targeted re-read 6 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 3c82957ecde6e5dd24a4138fca874b3cf9e29df8
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A targeted re-read of the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`, at 66634fb9, 6
Majors and 11 Nits), then the fixes re-read in `reread-1-security.md` to `reread-5-security.md`, every finding of each
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 5's Nit is resolved: `journal_retire_start` refuses a transaction not at `READ COMMITTED`, in the function
itself, so a role or session default cannot get around it, matching the per-run purge's rule; Task 049-14 tests it.
The change adds no injection point, no leak in a message or an audit record, no privilege and no race.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
