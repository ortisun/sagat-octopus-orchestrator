# Stage 9, codebase-details lens, of design 045, third run (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 7675358f
Reviewer: egm-review, stage 9 (the egm-codebase-details lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author; every finding and its severity kept, with the author's answer (applied in the fourth revision).

Counts: Blocker 1, Major 9, Nit 11

## Blocker

1. **Two writers of a `READY` step are still missing** (`route_loop_successor` in `loop_progress.rs`, `finish_collection` in
   `primitives.rs`), `chain_progress.rs` is mislabelled (it inserts a continuation successor's entry step), and those paths write their
   events in the store, so the deadline's source there is undefined. *Answer: accepted*: all named; on those store-side paths the store
   computes each deadline once from `db_now` and the definition it holds and writes the same value to the event and the column, as the
   named exceptions of the static check.

## Major (each accepted)

1. The path re-check: the domain command re-checks in the path transaction; `apply_path_if` takes the kind; `due_step_deadlines`
   returns rows shaped like `LostLease`. 2. The hold is inside the store's publication, reached from `publish_through` and `prepare`.
3. The ad hoc version insert writes level 0, and the loaders pass the stored level to the codec. 4. The overlay is the domain's, not
`retry_at`'s. 5. `tests/contract/schedules-bulk-operations/unchanged.rs` lists the new properties. 6. The static check names its
allowed functions and bans interval arithmetic on the columns elsewhere. 7. The slices name the step view's reader, the lag sample and
`apps/execution-engine/src/capabilities.rs`. 8. The engine's registry tests are in `apps/execution-engine/tests/registry.rs`. 9. The
measurements go in a new suite, `perf_activity_timeouts`.

## Nit (each accepted)

1. The five places of the time-to-live and `ClaimedTask`. 2. `RUN_FOR_CLAIM` is the run lock, not a loader. 3. The two metrics reuse the
existing families. 4. The harness's `fails` behaviour is extended. 5. The six SDK copies of `worker.proto` and their scripts. 6.
`sdks/SPEC.md` §7 and the pointer in Architecture decision 0089. 7. The equality test is in
`tests/integration/schedules-bulk-operations/instances.rs`. 8. The protocol baseline tests are named. 9. The claim tests set deadlines
relative to the clock. 10. `RetryRules` replaces the activity's field. 11. The codec belongs to slice 1 only.
