# Approval confirmation of design 049: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 75f01e1f5c4941a1a6fc7c561aa5ea81524ea4b3
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last re-read of the
task list and merge order, not a full review.

Re-read 2's two Majors and six Nits are resolved.

No Blocker, no Major.

## Nit

1. No task empties the runtime-SQL rule's baseline entry for the `format!` of `telemetry.rs`, which Task 049-8 moves into
   `journal.rs` unchanged.
2. The readiness-check exception of Task 049-6 is not in Requirements 049-005 and 049-011.
3. Task 049-9 does not say which values of `journal_bucket_retiring` count as active for the `runs` trigger and the
   purge refusal, nor what a done retirement returns.
4. `ensure_journal_buckets()` partitions every table in `pg_partitioned_table`, so a later partitioned table would get
   monthly partitions; none exists today.
5. `mark_projected` in `telemetry.rs` has no run row, so naming the bucket needs a read of `runs` first, off the hot path.

Counts: Blocker 0, Major 0, Nit 5

Settled: the five are recorded as design 049's follow-up Task 049-19, added after the approval as the approval rule
allows, to be fixed in the build.
