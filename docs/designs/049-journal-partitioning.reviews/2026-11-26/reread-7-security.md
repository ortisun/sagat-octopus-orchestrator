# Targeted re-read 7 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 75f01e1f5c4941a1a6fc7c561aa5ea81524ea4b3
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 3c82957e, not a full review. The reviewer read files only and ran nothing.

Checked with no finding: the gauge (no labels, no tenant data), the error mapping, `journal_bucket_retiring` returning the
state only, the `proconfig` audit, step 3's call of `ensure_journal_buckets()` (no default privileges, so new partitions
get no grants), the readiness check allowed by name, Tasks 049-7 to 049-9, the handshake grant to `sagat_retirer`.

No Blocker, no Major.

## Nit

1. The static check counts only literals parsed as statements, so SQL assembled at run time (`push_str`, a query
   builder) that names `run_events` outside `journal.rs` would skip the `purged_at` check and could serve an erased run's
   rows awaiting the drop. None exists today. Fix: also count any literal naming a journal table outside its owning file,
   or forbid assembled SQL in the crate.
2. (Suspicion.) The `runs` trigger's ended-month branch relies on `READ COMMITTED` without enforcing it; at a role's
   `REPEATABLE READ` default it could insert a dated root into a month being retired, which step 2 or 3 then stops on.
   Fix: refuse any other isolation in that branch, off the hot path, with a test.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at `a033fc72`.
