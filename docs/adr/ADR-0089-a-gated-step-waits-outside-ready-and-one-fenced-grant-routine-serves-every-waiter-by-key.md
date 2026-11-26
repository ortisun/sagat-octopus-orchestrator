# ADR-0089: A gated step waits outside READY, and one fenced grant routine, driven by key, serves every waiter whatever its gates

**Status:** Accepted (2026-11-13, OD-2026-11-13-19: the owner's "Aceitar o ADR-0089")
**Refined by:** [ADR-0110](ADR-0110-step-timeouts-are-deadlines-on-the-database-clock-and-retry-is-decided-per-failure-code.md) (proposed, design 045): a waiter's deadline becomes the earlier of its gates' and the step's close cutoff, and a waiter past its close cutoff is not eligible (decisions 4 and 5).
**Decided by:** agent proposal after the architecture, security and correctness reviews of 2026-11-13; accepted by the owner on 2026-11-13 (OD-2026-11-13-19)
**Refines / supersedes:** supersedes ADR-0056 decision 3 for step permits (a step permit is no longer taken
at claim) and decision 5 (grant by the waiter's shard owner, strict oldest first); refines ADR-0088
decisions 3 and 7 (one routine for both gates, the reservation rule) and ADR-0050 decision 9 (the two
flow-control capabilities are registry capabilities). The rest of ADR-0056 and ADR-0088 stands.
**Enforced by:** proposed checks of feature 022 — the poll-window test (64 permit-blocked steps of one key
and one step of another, T022-099), one readiness test per readiness source and the journal invariant
"every readiness of a gated node commits with its grant or its `queued`" in `verify_run` (T022-096), the
dual-gate starvation and timeout tests (T022-097, T022-098), the mixed-fleet activation test (T022-094),
the architecture rule that only `PgRunStore::create` inserts a gated run (T022-093) and the hot-path budget
(T022-076, T022-091). None exists yet; until they do, this rule is review only.

## Context

ADR-0056 (accepted 2026-11-13) takes a step permit in the claim transaction and leaves a step without one
`READY`. The claim's candidate query (`crates/infra-postgres/src/workers.rs`, `CLAIMABLE` and
`CLAIMABLE_PATHS`) reads, per activity, the oldest `READY` steps up to `limit.min(64)`. Sixty-four steps of
one saturated key therefore fill every poll's window and hide every step of another key of the same
activity, and each poll probes them again: the waste design 038 measured and FR-038-002 forbids. ADR-0088
(accepted the same day) already rejected granting a rate slot at claim for this reason.

The reviews found four more facts:

- Steps become `READY` in many places: `step.released` from the planner inside every `settle` of
  `crates/domain/src/execution/command.rs` (worker reports, timers, signals, approvals, updates, finished
  children), `step.retry_due` from the retry pass (`crates/infra-postgres/src/maintenance.rs`), and
  `step.retry_requested` from an operator (`crates/domain/src/execution/state_effects.rs`). A gate added to
  some callers is bypassed by the others.
- A run or step may declare both gates. Two routines (ADR-0056's grant and ADR-0088's) each checking one
  gate can grant one gate and leave the other waiting, or time out on one gate and leave a waiter row of
  the other behind.
- Strict oldest-first across shards makes every grant of a key wait for the shard that holds its head;
  with a pass every 500 ms that bounds a key to roughly two grants a second per replica.
- An engine of an older binary runs no gate at all. ADR-0050 decision 9 requires such a feature to be
  activated through the instance-capability registry (ADR-0082).

## Decision

1. **A gated step is never `READY` without its grants.** For a node that declares `concurrency` or
   `rateLimit`, the reducer MUST map `step.released`, `step.retry_due` and `step.retry_requested` to
   `WAITING` (wait kind `RATE_LIMIT` or `CONCURRENCY`), and only the grant event that completes the step's
   last declared gate MUST make it `READY`. A step permit is held from its grant until its attempt ends or
   the step leaves `READY` other than by a lease; granted, unleased steps count as holders. The claim MUST
   NOT read or write a semaphore or a window.
2. **One adapter at the persistence point.** The function that persists every transition
   (`persist`, `crates/infra-postgres/src/execution.rs`) MUST collect, after the reducer, every gate key the
   transition's events touch, lock them once (semaphores, then windows, each sorted by pool, scope, group,
   key, before usage rows) and journal for each gated step its grant or its `*.queued` in the same commit.
   A commit that leaves a gated step released with neither MUST fail `verify_run`. It MUST NOT write the
   task-ready hint for a step left waiting.
3. **One grant routine, driven by key.** A scheduler pass MUST select keys, not waiters: those with
   waiters whose next-eligible instant has come (a hint kept under the key's lock, which may be early and
   is never late), through the scheduler's per-tenant fair selection, bounded per pass. It MUST hold the
   fence of the key's home shard. For each waiter it tries, one transaction MUST lock the waiter's run
   (`SKIP LOCKED`), its `runs` row, its semaphore, its windows and usage rows in that order, check every
   gate the waiter declares, and grant all of them or none. A run waiting on two keys may be granted by
   either key's pass; the first commit wins and the other finds no waiter.
4. **The reservation rule replaces strict oldest-first.** On each gate a waiter at position `p` (eligible
   waiters of that key ahead of it) MUST be granted only when the key's counted holders or grants plus `p`
   are below its own limit. Eligible means not paused, not past its deadline, of a live run, and admitted by
   its own limit. An older waiter refused only by its other gate therefore keeps room reserved; an older
   waiter refused by its own lower limit does not block (ADR-0056's skip-ahead stands). With feature 019,
   positions follow (priority, enqueue instant).
   *Amended 2026-11-24, delegate (OD-2026-11-24-4; feature 022 decisions D022-073, D022-076 items 3 and 9):* a
   narrowing, not a departure. Strict order returns at admission: a new arrival that may wait (a start, a step) is
   granted at once only when its key has no eligible waiter and its counted holders or grants are below its limit, and
   otherwise queues behind them. The grant routine keeps the reservation rule above and tries a key's waiters in
   admission order, counting as of the pass's selection of the key, and grants no younger waiter whose limit and window
   are not looser than one the same pass refused for lack of room (decision D022-078 item 2). A start that may not
   queue (a reset, a pending reset) is decided by the reservation rule: granted when its key's counted holders or
   grants plus the eligible waiters are below its limit, refused otherwise. Every limit is the lower of the waiter's
   declaration and the namespace's current ceiling.
   *Amended 2026-11-25, delegate (OD-2026-11-24-4; feature 022 decisions D022-088, D022-121 and D022-122):* a waiter
   the grant routine cannot decide now ends its key's pass, and every younger waiter of the key waits for the next
   pass, so that no younger waiter is granted before an older one the window may have room for. Two cases are "cannot
   decide now": its run is held by another transaction (`SKIP LOCKED` found it locked), and its grant failed for a
   reason that clears on its own (the store unavailable: a pool or statement timeout, a serialization failure, a
   deadlock). Any other failure of a waiter's grant (a version this build cannot load, an execution error, a row a
   constraint refuses) would end the pass on every pass, so it is skipped: counted `failed`, logged with its run, and
   the pass goes on; the waiter stays queued until its deadline. A held signal whose delivery fails is dropped with its
   audit record (`DELIVERY_FAILED`) and the start is granted without it. A waiter's transaction that does not commit is
   rolled back, and the rollback awaited, before the next waiter is tried, so that a second waiter of the same run
   finds the run free.
5. **One timeout routine.** A waiter has one deadline, the earlier of its gates'. At it, one transaction
   MUST journal the `wait_timed_out` of the gate that refuses it then (`rate_limit` first when both do) and
   the `dequeued` of the other. A grant MUST skip a waiter past its deadline. A waiter of a terminal run is
   an invariant breach: alerted, skipped, never counted as ahead.
6. **Registry activation.** `concurrencyKeys` and `rateLimits` MUST be registry capabilities required of
   control-plane and engine instances; until an operator activation, their use is refused
   (`FLOW_CONTROL_ACTIVATION_PENDING`). The current keyed-digest version MUST be a database row read in each
   deciding transaction.
   *Amended 2026-11-26, delegate (OD-2026-11-24-4; feature 022 decisions D022-136 and D022-137, task T022-116):* the
   operator activation is the generic `control-plane capability-activate flow-control-concurrency-v1` (and
   `flow-control-rate-v1`) of ADR-0098 decision 4, which keeps every check of the feature's own command (now removed)
   and refuses while no digest key version is `ACTIVE`.

## Considered options

1. **Gated steps wait outside `READY`, one key-driven grant routine with reservations** — chosen. The claim
   is untouched for every step; one place enforces the gate for every readiness source; dual gates cannot
   half-grant; each key's grants proceed without the head's shard.
2. **Keep the permit at claim and filter the claim query** — rejected: a semaphore probe per candidate in
   the hottest statement, and the window still fills with steps whose key is full.
3. **Guard each readiness caller** — rejected: the next caller added forgets it; the reducer cannot.
4. **Two routines, one per gate, with a lock order between them** — rejected: half-grants and orphaned
   waiter rows on timeout.
5. **Strict oldest-first, per waiter shard** (ADR-0056 decision 5 as accepted) — rejected for throughput
   and because a dual-gated head blocks its key whichever gate refuses it.
6. **Drop node-level `concurrency` from 0.3.0** — kept as the fallback the owner may pick (O1169 of
   feature 022), not chosen: rate limits need the same machinery anyway.

## Consequences

- Good: no poll ever probes a step it cannot lease; no readiness path can bypass a gate; a waiter is granted
  or timed out once whatever its gates; a busy key is granted at its limit regardless of shard placement;
  an older binary cannot run ungated.
- Bad: a gated step that becomes ready on a busy key waits for a pass (up to one interval) even when a
  permit frees at once; pausing a run touches its gated steps; the reservation rule can leave capacity
  unused while a dual-gated head waits for its other gate (bounded by its deadline); one more pass.
- Harder: every future readiness event must be added to decision 1's mapping; every future gate joins the
  one routine.
- Compatibility: no run uses a gate yet; additive events and wait kinds; the worker protocol is unchanged.
  Rollback disables the capabilities for new use while the routines drain.

## Evidence

None yet; the tests named under "Enforced by" are required before acceptance of the slice's
implementation, plus the benchmark of feature 022 SC-022-029 (three engines, one key at 100 a second).
Spec: `specs/022-flow-control-step-policies/spec.md` FR-022-019, FR-022-044, FR-022-046, FR-022-051..053;
research: D022-019..023, O1169; reviews: `specs/022-flow-control-step-policies/reviews/2026-11-13/`.

Note (2026-11-19, OD-2026-11-19-2 and OD-2026-11-19-3): decision 2 is refined by ADR-0094 (the readiness gate decides in
an evaluation round the planner asks for, before `persist`), and decision 6's "the current keyed-digest version" reads
"the versions in force" under ADR-0093. The rest stands.
