# Targeted re-read 8 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 1f07592aee8297f7da940a2decd6bf5b89d1699d
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since 3c82957e, not a full review. The reviewer read files only and ran nothing.

Re-read 7's three Nits are resolved. The snapshot reasoning of the late dated root holds in both orders (each statement
of a `VOLATILE` function at `READ COMMITTED` takes a fresh snapshot), and its lock adds no deadlock; the merge order
matches the dependencies.

## Major

1. No task measures the hot-path budget on a dated run: at Task 049-8 the store cannot yet create one, Task 049-10 does
   not touch the budget, and no task references Success criterion 049-001; "the current month" would also flake at a
   month's end. Fix: Task 049-5 switches the budget to dated runs, naming the partition of the run's own bucket; Task
   049-10 adds `run_payloads` and the trigger lookups and references the criterion.

## Nit

1. The isolation tests may pass for the wrong reason (the 24-hour rule refuses an older month with the same constraint),
   and a retirement-active refusal is mapped to an internal error though a retry would succeed. Fix: own constraint
   names, the test inside the 24-hour window with an accepted `READ COMMITTED` case, a retryable mapping.
2. Requirement 049-014 still says "one function" of `journal.rs`; say which moved functions check `purged_at`.
3. The static check no longer covers migration function bodies or the `pg_proc` test of Requirement 049-011.
4. The function audit fails open for a function left off the list; compare `proconfig` as a set.
5. Step 3 does not say the regenerated trigger's clones are enabled always on existing partitions.

Counts: Blocker 0, Major 1, Nit 5

All six were fixed at `b4c70e38`.
