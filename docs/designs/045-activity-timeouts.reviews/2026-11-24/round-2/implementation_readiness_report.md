# Stage 9 (Implementation Readiness Test), implementation-readiness lens, of design 045 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 5033df9d
Reviewer: egm-review, stage 9 (the egm-implementation-readiness lens, dispatched directly)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author; every finding and its severity kept, with the author's answer.

Counts: Blocker 1, Major 12, Nit 9

## Blocker

1. **Who computes the attempt deadline is contradicted**: the store computes `attempt_deadline_at` today, `step.leased` carries no
   deadline, yet the design says the domain computes every deadline. *Answer: accepted*: for a version whose level is above 0,
   `step.leased` carries `attemptDeadlineAt` and `attemptDeadlineCause` (optional, truncated to microseconds), the store writes them,
   and the lost-lease pass reads the cause from the column; for other versions the store computes the attempt deadline as today. The
   lease expiry stays the store's (it is renewed, not a deadline of this design). The static check covers `start_deadline_at`,
   `close_deadline_at`, `close_cutoff_at`, `attempt_deadline_cause` and, for level-above-0 versions, `attempt_deadline_at`.

## Major

1. **The slices cannot each build; the crate columns are wrong.** *Answer: accepted*: each server slice is one pull request that
   changes every caller of a signature it changes; the table lists the crates each touches.
2. **Where the deadlines live in domain state is unspecified.** *Answer: accepted*: `StepState` gains `start_deadline`,
   `close_deadline`, `close_cutoff` and `min_protocol_minor`; `StepState::restore` takes them; the root load, `RUN_FOR_CLAIM` and the
   path snapshot reader select the columns; a lease command returns `ExecutionError::NotClaimable` with a closed reason, and the
   claim filters with the same predicate first, so it never aborts the run's other leases.
3. **The shape of the domain types is open** (`RetryPolicy` is `Copy` with `const` values). *Answer: accepted*: a new type,
   `RetryRules {base: RetryPolicy, final_codes, overrides}`, beside `RetryPolicy`, which stays as it is; `RetryRules::for_code`
   returns `CodeRule::Final` or `CodeRule::Policy(RetryPolicy)`; the timeouts are `StepTimeouts` on `Node`, beside `timeout`.
4. **The publication gate is in the wrong crate and in two places.** *Answer: accepted*: one task (Task 045-5) after the migration:
   a domain predicate (`WorkflowDefinition::activity_timeouts_level`), a port method on the publication store
   (`hold_requirement(capability) -> bool`) and the check in `crates/application/src/definitions.rs` where `publish` runs; `infra-ir`
   only parses.
5. **The replay-floor mechanism is undefined.** *Answer: accepted*: the version's level (security round 2, Major 1) and a new
   migration that replaces `platform_requirement_remove`.
6. **The pause shift cannot be built** (`run.paused` and `run.resumed` carry no time). *Answer: accepted*: `run.paused` gains an
   optional `pausedAt` and `run.resumed` an optional `shiftUs`, written only for runs of a level-above-0 version; `RunState` keeps
   `paused_at`; the resume statement shifts the run's claimable rows, root and path, in one statement.
7. **No task implements the reset rule.** *Answer: accepted*: Task 045-4 adds `ResetRule::RecomputeDeadlines` for `step.released`,
   `step.retry_due` and `step.retry_requested`, from the reset's `db_now`.
8. **Unmerged lease-withdrawal work is a missing dependency.** *Answer: accepted*: the withdrawal parts of Tasks 045-7 and 045-16 are
   built by whichever of this design and pull request #260 (and #264 for provisional leases) lands second.
9. **How the delay enters the fingerprint is unspecified.** *Answer: accepted*: its bytes are appended only when the field is
   present (a separator byte and the decimal digits), so a report from an older host keeps its fingerprint.
10. **A design decision is deferred to a measurement** (the 10,000 or 18,000 ms minimum). *Answer: accepted*: decided now, 18,000 ms
    (Decision 045-024), so that every lease can be provisional and none loses delivery acknowledgement's protection.
11. **The conformance scenarios cannot be written from the plan.** *Answer: accepted*: the harness gains the controls
    `elapseStartDeadline` and `elapseCloseDeadline` (test-only, as `expireLease`) and the worker behaviours `stopHeartbeat` and
    `failWith {code, nextRetryDelayMs}`; `activity-timeouts` runs at each SDK's accepted minor below 1.9, without needing 1.8.
12. **Two targets of Task 045-1 do not exist** (`contracts/errors.md` of feature 001, an event list). *Answer: accepted*: the
    feature's own contracts, `specs/045-activity-timeouts/contracts/errors.md` and `journal-events.md`; `sdks/error-catalog.json` is
    dropped (it lists SDK errors, not failure codes).

## Nit (each accepted)

1. The new suites' `[[test]]` entries, durations and `main.rs`; the fault tests of Task 045-8 sit in its own slice; the scale fixture
   is the one of `tests/performance/` that Task 045-17 names.
2. The pass's wiring: its port method, its `Condition` variant, its report field, its registration in the tick and its lag gauge.
3. The capabilities replace the three closed `CHECK` lists, named as design 043 did, and add `Capability` variants in
   `crates/application/src/instances.rs`; the handler capability moves to the protocol task.
4. Protocol 1.7's server and host work is a listed dependency of the protocol slices.
5. `attemptDeadlineCause` in REST is `startToClose` or `scheduleToClose`; the Studio's two warnings have their wording.
6. Requirement 045-012's test is in Task 045-16's body.
7. The cross-reference to feature 022's matching task for the case where 022 lands second.
8. Cross-field validation: the domain's constructors own the rules and their closed reasons; `infra-ir` maps them to pointers; the
   boundary tests add 16 and 17 entries and the empty lists.
9. The candidate query keeps the clock function it uses today, and the parity test fixes the clock.
