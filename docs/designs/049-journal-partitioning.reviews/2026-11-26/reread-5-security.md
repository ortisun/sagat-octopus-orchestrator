# Targeted re-read 5 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 2245250b429508ae1c7c6f43d7a538e3e8401426
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 44d6a0b3, not a full review. The reviewer read files only and ran nothing.

Re-read 4's Nit is resolved. Checked with no finding: the lock before the check, `TimeZone` and `DateStyle` pinned, the
lock key's explicit format computed in SQL everywhere, the trigger functions owner-only, the merge order (until Task
049-13 a process creates legacy roots, which fails closed), design 053's heartbeat returning platform data only.

No Blocker, no Major.

## Nit

1. `journal_retire_start` does not fix its isolation level: at `REPEATABLE READ` (a role default) the check's snapshot
   precedes the lock wait, and the check can accept what it should refuse; steps 2 and 3 still catch the run, so nothing
   is exposed or lost. Fix: refuse unless `READ COMMITTED`, with a test.

Counts: Blocker 0, Major 0, Nit 1

Fixed at `3c82957e`: `journal_retire_start` refuses a transaction not at `READ COMMITTED` (Requirement 049-009 and the
Retirement section's step 1), with the test in Task 049-14. A stray mark in design 053 was removed with it.
