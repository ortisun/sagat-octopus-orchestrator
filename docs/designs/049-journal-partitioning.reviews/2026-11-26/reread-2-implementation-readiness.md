# Targeted re-read 2 of design 049: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 3c82957ecde6e5dd24a4138fca874b3cf9e29df8
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the task list and merge order after re-read 1's fixes, not a full review.

Re-read 1's findings are resolved. Every file named exists (`path_read.rs` and `resets.rs` are under
`crates/infra-postgres/src/execution/`).

## Major

1. Task 049-8 cannot merge in the stated order: the classification test lists every relation of kind `r` or `p` against
   a static manifest, so the partitions of Task 049-8 (and `run_payloads` at Task 049-10) fail it until Task 049-7's
   mapping, which merged after Task 049-10. Fix: the mapping merges with Task 049-8; `run_payloads` joins the
   append-only set in Task 049-10.
2. Task 049-9 leaves the shape of `journal_retirements`, the return type of `journal_bucket_retiring` and the tests of
   `ensure_journal_buckets()` to the builder, though Tasks 049-5, 049-8 and 049-9 read the state and the drop record; at
   Task 049-9 no table is partitioned, so a test against the real schema cannot fail. Fix: name the key, the state and
   its values, the drop record and the return type; tests on a disposable partitioned table; a test of both error
   mappings in Task 049-8.

## Nit

1. The static check would flag the readiness check of `execution.rs` and the strings of `privilege.rs`; count only
   literals parsed as statements, and allow the readiness check.
2. Step 3 does not call `ensure_journal_buckets()`, so `run_payloads` has no partitions in a database only migrated.
3. Tasks 049-8 and 049-10 do not name `work_budget.rs`, whose locked relations must name dated partitions by role.
4. Some per-run reads of `run_events` sit inside statements on other tables (`telemetry.rs`, `execution.rs`); they move
   whole into `journal.rs`.
5. Before Task 049-10 the reference trigger's "legacy branch only" is ambiguous for dated runs; it checks `payloads` for
   every run until then.
6. The gauge `journal_partition_months_ahead` is undefined while no table is partitioned.

Counts: Blocker 0, Major 2, Nit 6

All eight were fixed at `75f01e1f`.
