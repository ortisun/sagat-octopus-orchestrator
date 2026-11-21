# Architecture review: T022-064 run-creator conformance

Reviewer: `architecture-reviewer` subagent, 2026-11-20, given the diff and the spec. Text as returned (condensed only in
the source list and the closing tables); resolutions are in `research.md` D022-054.

**Verdict: 1 Blocker, 6 Major, 6 minor, 4 nit.** The core design fits the existing pattern: a planner question answered by the engine, one adapter function, and fresh decisions written in the same transaction. The Blocker is in how a refused release is retried.

## Blocker

### B1. A refused release is retried every pass with no backoff and no bound, and it blocks other delayed starts
`passes.rs:4499-4506` and `:589`; `maintenance.rs:638-648` (`ORDER BY start_delayed_until LIMIT $1`, batch 200); `readiness_gate.rs:887-889` (the namespace admission lock on every attempt); `readiness_gate.rs:1381-1384` (`refused_full` counted on every attempt). A deferred run keeps its `start_delayed_until` and stays at the head of the due query: once 200 or more deferred runs exist in a replica's shards, no other delayed start is released; every pass costs up to 200 transactions that take the namespace admission lock; `refused_full` rises once per run per pass; a WARN per run per pass; the run waits past its delay with no deadline, outside the bounded queue, unmeasured. Correction: a durable retry instant (additive migration), set on `Deferred` in a second transaction under the run lock with 018's backoff (5 s doubling to 5 min); the due query filters and orders on `coalesce(retry, start_delayed_until)`; count a deferral once per run; a gauge or lag for deferred runs; a test with batch 1 (an older run deferred on full key A, a newer on key B released on the next pass); decide whether a deferral has a bound (owner's or an ADR's).

## Major

- **M1. One of the three layers protecting FR-022-059 was removed without a replacement or a record.** `start_run_delayed` and `record_reset` no longer refuse a gated definition; D022-049 and `check-architecture.py:372-375` still say they do. Safety now depends on planner ordering; `Gates::holds_entry` is false while the declared gates are undecided, so the reducer would accept `step.released` before any run-level decision, contrary to ADR-0094's principle. Correction: the reducer fails closed (an undecided declared start gate holds the entry); update D022-049 and the script comment; a row in `architecture-principles.md`; extend the script so `Resolution::StartGates {` and `StartDecision::` are constructed only in `Engine::start` and `decide_start_gates`.
- **M2. G022-012 grows.** `start_run_gated` and `resolve_start_gates` validate the same rule; `start_run_key_failed` and the `Err(())` branch end the run the same way; `Evaluation::StartGates { queue: bool }` is mapped lossily to `StartDecision::{Release, Reset}`; `RunGateKeys` now carries policy and its manual `Debug` omits it. Stronger correction: `StartCause { Admission, Release, Reset }` carried by the question, admission through the same planner question, delete `start_run_gated`, `start_run_key_failed`, `refuse_gated_start`; minimum: one shared validator, a one-to-one cause, G022-012 updated.
- **M3. `refused_reset` is in the closed vocabulary but never emitted** (`ConcurrencyOutcome::RefusedReset`; plan metrics table). Record it before returning `PermitUnavailable`, with a test.
- **M4. The 409 `CONCURRENCY_PERMIT_UNAVAILABLE` drifts from 018's contract**: `tests/contract/schedules-bulk-operations/errors.rs:66` still lists it in `NOT_YET`; no remediation sentence ("Retry the reset when a holder of the key has finished."); no API test. Move it to `RAISED`, add the sentence and a test (409, `CONFLICT`, not retryable, `details.reason`).
- **M5. ADR coverage for the new release outcome.** D022-052 item 2 adds a third outcome to ADR-0056 decision 3 (refused, retried without limit outside the bounded queue): it belongs in an ADR; ADR-0094 decision 4 does not mention `StartGates`; 018 `plan.md` row 12's lock order no longer describes the code.
- **M6. Update-with-Start on a queued run departs from an owner-accepted decision** (D022-050 item 6 says hold the signal or update); the diff records it as "not affected" in G022-010 and pins it with a test. Add a decision entry and the owner's confirmation, or keep item 6 and mark the Update case pending under T022-109.

## minor

- **m1** The `Deferred` arm is generic in `apply_if`; restrict it to `start_release`. `docs/runbooks/execution-engine.md:127` does not list `deferred`.
- **m2** "A reset never queues" is enforced only in `resolve_start_gates` and the adapter; the reducer accepts a run-level `concurrency.queued` after `run.reset` on replay.
- **m3** Older readers of gate events reject `restored: true`; confirm no released binary decodes gate events (022 is unreleased), or put the reset of a gated definition behind the capability.
- **m4** A transient error fails a pending reset for good (`BINARY_CAPABILITY_MISSING` mapped to `ADMISSION_REFUSED`).
- **m5** Deadlock freedom of the release's new lock is argued, not tested beyond the release-against-start race.
- **m6** Rule 10: `docs/runbooks/run-reset.md` (no `CONCURRENCY_PERMIT_UNAVAILABLE` row; "six closed reasons"; says a reset takes no permit) and 018 `tasks.md` T018-053 not updated.

## nit

- n1 `quotas.rs:214-226`: `ensure_room_for_run` lost its doc line.
- n2 The adapter's `decide_start_gates` doc mentions neither Release nor Reset.
- n3 `RunState.reset_admission` is derived, not restored, and `RunState` derives `PartialEq`.
- n4 The WARN per deferral should be `debug` once B1 adds a counter.

## Answers

- Layering: clean (no I/O or framework in the domain; `StartDecision` in the application).
- `StartGates`/`StartDecision` fit the ADR-0094 pattern; no second adapter path; a second domain path and a lossy mapping (M2).
- No accepted ADR contradicted outright; ADR-0050 decision 7's slot order kept; the release outcome needs an ADR or a note (M5).
- Compatibility: the reset completion reason is an addition to a closed set; `scheduler_actions_total{outcome="deferred"}` is new; the 409 drifts (M4); the `restored` marker is safe only if 022 is unreleased (m3); B1's fix needs an additive migration.
- Scope: the only 022 branch is this worktree's; no duplicated work.
