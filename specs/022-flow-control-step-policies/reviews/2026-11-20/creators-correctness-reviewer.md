# Correctness review: T022-064 run creators

Reviewer: `correctness-reviewer` subagent, 2026-11-20, given the diff and the spec. Text as returned (condensed only in
the "checked and sound" list); resolutions are in `research.md` D022-054.

I found no Blocker. There are 3 Major findings, 9 minor and 4 nits, with one note for the owner. The release, reset and reducer paths are mostly sound: the start's decision is taken in the same transaction as the release or the `run.reset`, and the lock order is safe. The real problems are the retry loop of a deferred release and a ticked task whose stated evidence the suite cannot reproduce.

`PgRunStore::create` still refuses every gated definition unless the test switch `step_gates_for_tests` is on (D022-037), so none of this reaches production in this build.

## Checked and sound

- One transaction per decision: `Engine::run` answers `StartGates` inside the same transaction as `run.start_released` or `run.reset`.
- Lock order of the release (run row, namespace admission lock, key row) cannot form a cycle.
- Reset reservation rule: `holders + waiters < limit`, the expression admission uses; refused before anything is written.
- Planner ordering: `give_back_ended_gates` runs first, and `retire_restored` stops it from giving back the restored step permit; the point's step is rewritten to `Ended` by its own `step.routed`.
- Pause and stop against an undecided start: `start_undecided` returns `None`; `Condition::StartDue` does not release a paused or stopping run.
- Reset at the `run.started` point, a source that queued and was then granted: sound.
- Other callers: `start_run_delayed`, `start_run_gated`, `start_run_key_failed` only in `Engine::start`; `record_reset` only in `runs.rs`.
- Journal readers: the only consumer of gate event details is `event_codec::gate_of`.

## Major

### M1 — Deferred releases block the release pass for every run behind them, with no bound
`maintenance.rs:638-648`, `passes.rs:4499-4506` and `6169-6195`. `due_start_releases` selects `ORDER BY start_delayed_until LIMIT $batch`; a deferral writes nothing, so a deferred run stays first in line on every pass. With `batch = B` deferred runs of a full key, a later ungated delayed run of the same tenant is never released until the key drains. A deferred run also waits outside the queue's own timeout. Test: `batch = 2`, defer two releases, add a delayed run of another definition, one `tick()` must give `starts_released == 1` (0 today). Fix: a `start_release_retry_at` with backoff the candidate query filters on and orders by; do not move `start_delayed_until`.

### M2 — `concurrency_acquisitions_total{scope="run",outcome="refused_full"}` counts a deferred run once per pass
`readiness_gate.rs:940`, `956-958`, `1031`, `1381-1384`. The release reuses `start_refused`, so a deferred release adds one each pass; a reset refused `CONCURRENCY_PERMIT_UNAVAILABLE` is not counted at all. Fix: call `start_refused`'s counter only for `StartDecision::Admission`; count a deferral under its own label; count a reset refusal once.

### M3 — The ticked T022-064 claims a hand mutation the suite cannot show
"the restored returns persisted" cannot be caught: a reset point's sequence is its `step.completed`, and the step's `concurrency.released` comes after it, so none of the reset tests restores a prefix that contains a return. Test: `a → b → c` with step-level `concurrency` on `a`, reset at `b`'s completion. Rerun the mutation and correct the evidence.

## minor

- **m1** The reservation rule for a reset is not tested: the pending-reset test grants `behind` before the completer runs, so entry 0 is refused by a holder, not by the waiter. Test: complete before the tick.
- **m2** `reset_admission` is not stored; stored state equals replay only because no commit ends while it is true. Fix: refuse a commit that ends with it set, or leave it out of the comparison.
- **m3** `runs.flow_control_gates` is not rewritten by `run.reset` alone (`run_gates_changed` looks only at run-scope `Gate` events): the `Err(())` key path would leave the source's `Held`. Fix: include `RunEvent::RunReset`.
- **m4** A delayed gated start skips the binary-capability check at creation: without a key ring the release fails every pass. Fix: probe the ring in the delayed branch, or defer `BinaryCapabilityMissing`.
- **m5** Rolling upgrade: a pre-T022-064 scheduler with flow control releases a delayed gated run without a permit; an old reader rejects `restored: true` gate rows. Fix: version the capability.
- **m6** A transient refusal at pending-reset completion (`BinaryCapabilityMissing`, a live-keys ceiling) fails the entry permanently (`ADMISSION_REFUSED`) after its source was cancelled.
- **m7** The 409 mapping is untested (`crates/api-http/tests/flow_control_errors.rs:3` says it lands with T022-064).
- **m8** 018's `journal-and-events.md:58` ("Step permits are never open at an eligible point") and 022's `journal-events.md:64` disagree with the build, which offers the gated step's own `step.completed` as a point while its permit is still `Held` and retires it at `run.reset`.
- **m9** A deferral is invisible: the run shows neither holding nor waiting.

## nit

- `quotas.rs:214-215`: `ensure_room_for_run`'s doc line moved to `lock_run_admission`.
- `scripts/check-architecture.py:375` and `research.md` near line 633 still say `start_run_delayed` refuses a gated definition.
- 018 plan row 17 puts the semaphore before the re-emitted prefix; the build writes the prefix first (harmless today).
- `Report.starts_released` also counts a release that ended the run `FAILED`/`EXPRESSION_ERROR`.

## Note for the owner (not a defect of the diff)

With `CANCEL_SOURCE` on a key at `limit: 1` that has any waiter, the source is cancelled at request time and the completion always fails `CONCURRENCY_PERMIT_UNAVAILABLE` under the reservation rule. 018 row 18 allows this, but users get a cancelled run and no replacement. A pre-check at request time may be worth an owner decision.
