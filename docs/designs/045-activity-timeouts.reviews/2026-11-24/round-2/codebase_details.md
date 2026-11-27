# Stage 9 (Implementation Readiness Test), codebase-details lens, of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 5033df9d
Reviewer: egm-review, stage 9 (the egm-codebase-details lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author; every finding and its severity kept, with the author's answer.

Counts: Blocker 2, Major 13, Nit 10

## Blocker

1. **"The three statements that write a step" is false**: body steps are inserted `READY` by `insert_ready_activity`
   (`crates/infra-postgres/src/loop_progress.rs`), collection admission (`primitives.rs`) and chained collections
   (`chain_progress.rs`), and the admission events (`loop.iteration_admitted`, `collection.iteration_admitted`) make body steps
   claimable. *Answer: accepted*: Technical Plan 2 lists every statement that makes a step claimable, and the admission events carry
   the deadlines; columns are added to existing statements, so still no statement is added.
2. **`due_start_releases` is not a precedent for excluding paused runs in the query.** *Answer: accepted*: the precedents are
   `stopped_runs` (`NOT run.paused`) and `claimable_query!`; and the pause now writes the claimable rows' deadlines as null (security
   round 2, Major 5), so paused rows are not in the index at all.

## Major

3. **Who computes `attempt_deadline_at`.** *Answer: accepted*, as for the implementation-readiness Blocker: from the event for a
   version whose level is above 0, in SQL as today otherwise.
4. **Two contract files named do not exist.** *Answer: accepted*: the feature's own `specs/045-activity-timeouts/contracts/`; the
   journal events are listed beside `specs/018-schedules-bulk-operations/contracts/journal-and-events.md` and
   `specs/002-durable-execution/data-model.md`.
5. **`sdks/error-catalog.json` is the wrong home for failure codes.** *Answer: accepted*: dropped.
6. **The reset rule targets a case that cannot occur** (a reset point is not offered while a step is open, `ResetBlock::OpenWork`).
   *Answer: accepted*: no reset rule is needed; the new run's steps are released fresh; Requirement 045-020 keeps only the operator's
   retry, and the reset-registry test asserts the copied deadline fields of finished steps are inert.
7. **The new events' shapes are not given.** *Answer: accepted*: `run.paused {pausedAtUs}` and `run.resumed {shiftUs}`, optional,
   with the `StartDelayed {until_us}` precedent; `step.failed` from `READY` and `RETRY_WAIT` keeps `fence: None` and gains
   `generation`; both reducers (root and scoped) are named.
8. **Two clearing events do not exist** (`step.reconciliation_requested`; an operator's hold). *Answer: accepted*: the events are
   `reconciliation.requested` and `step.needs_review` (an operator's hold is a review).
9. **The pass's port and re-check are not named; the lost-lease carrier has no cause.** *Answer: accepted*: `MaintenanceStore::due_step_deadlines`,
   `Condition::StepDeadlinePassed`, `LostLease.deadline` and `Expiry::AttemptTimeout {deadline}`.
10. **Slice 2 is not buildable alone.** *Answer: accepted*: each server slice changes every caller it breaks.
11. **Protocol 1.7 is a missing dependency** (the server offers 1.6). *Answer: accepted*: named.
12. **The index SQL contradicts the risk mitigation.** *Answer: accepted*: `IF NOT EXISTS` with the `indisvalid` check of
    `20261017000000_scheduler_shards.up.sql`, in the new migration.
13. **The REST views are not named.** *Answer: accepted*: `RuntimeNode` gains the deadlines, `RuntimeGraph` the server time.
14. **The publication gate and the replay floor have no precedent or specification.** *Answer: accepted*: specified (the version's
    level, the hold function's return, the replacement of `platform_requirement_remove`).
15. **`RetryPolicy` is `Copy`; where the timeouts live.** *Answer: accepted*: `RetryRules` beside it; `StepTimeouts` on `Node`.

## Nit (each accepted)

16. `upsert_path_instance` is in `execution/path.rs`, `RUN_FOR_CLAIM` in `execution.rs`; the compensation-only branches of
    `claimable_query!` need no change.
17. The candidate query uses `clock_timestamp()`: kept, and the parity test fixes the clock.
18. `[[test]]` entries in `tests/Cargo.toml`: named.
19. No code-to-sentence map exists in the Studio: Task 045-14 creates it.
20. `sdks/SPEC.md` has two sections numbered 10 and lacks the 2,000 ms bound on this branch: cited as protocol 1.8's
    `stream-completion.md`.
21. The `byErrorCode` delays lack the 86,400,000 maxima: added.
22. Reuse `retry_at` with the base as defaults: named.
23. How `min_protocol_minor` relates to `DATABASE_PROTOCOL_MINOR` and the run-level `required_worker_protocol_minor`: said.
24. Design 043's capability work as the template: named.
25. The lease time-to-live is set in five places: all named.
