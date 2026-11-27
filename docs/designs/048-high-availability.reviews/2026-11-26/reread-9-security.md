# Targeted re-read 9 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 6cdd2c007a29eaa7bca980cc4d1ef9e5deef0fa4
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 4edac72b, not a full review. The reviewer read files only and ran nothing.

Checked with no finding: the outcome rule, tickets and the superseded set, `WriteTx` with its savepoints and drop rule,
`ReadTx`, the fresh-database admission, the alerts, the drills and the probe's include, metric labels, the grants of
`server_incarnation` and `registry_first_confirmation`.

No Blocker, no Major.

## Nit

1. `command_await_durability()` is in neither the database objects paragraph nor Task 048-10, so nobody is named to run
   it.
2. Nothing stops `DbHandle::new(pool, Arc::new(InertHealth))` from staying in `apps/` after Task 048-10.
3. Nothing confines the passive observer's profile, which reads a standby, to feature 032's watchers.
4. (Suspicion.) A binary older than design 048 never confirms; ADR-0130 should state what still stops it in an
   initialized cell.
5. (Suspicion.) A failed region lookup must give the constant answer, not a reason.

Counts: Blocker 0, Major 0, Nit 5

All five were fixed at `287c7bdb`.
