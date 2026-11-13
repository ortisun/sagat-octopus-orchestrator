# ADR-0056: Concurrency keys are durable permits taken after the run lock, and a queued run is granted oldest first by its own shard owner

**Status:** Proposed
**Decided by:** agent proposal, pending owner acceptance (2026-10-26; revised after architecture review 2026-10-27)
**Refines / supersedes:** refines ADR-0020 (sharded, fenced scheduler) and ADR-0027 (lock order and
quotas); builds on ADR-0002, ADR-0012 and ADR-0030 (chain-before-run order). Supersedes none.
**Enforced by:** proposed checks of feature 022 — the fault suite `tests/fault/flow-control-step-policies`
(timeout-versus-grant, crash at release and grant, duplicate grant across shards), the journal/semaphore
consistency assertion added to `verify_run`, lock-order barrier tests per path (admission, grant, timeout,
terminal, claim against attempt end), and a run-creator test that fails when a creator bypasses the
permit admission port. None exists yet; until they do, this rule is review only.

## Context

Feature 022 (`specs/022-flow-control-step-policies/spec.md`, US02) lets a definition declare that at most
N runs, or N attempts of a step, may be active per business key, with work beyond the limit waiting
durably and giving up after a timeout. Four facts constrain the design:

- A run has no `READY` state to wait in: once admitted, its first steps are ready. A queued run must be
  admitted (so the start is durable and idempotent) yet hold no work.
- The platform already orders locks carefully: chain before run (ADR-0030), run before shared aggregate
  rows, entity rows before quota usage rows (ADR-0027), and claim transactions take run, drain fence,
  session and sorted activity-quota locks (`docs/architecture/execution-semantics.md`). ADR-0037,
  ADR-0038 and ADR-0039 each fixed a deadlock found between two such orders.
- Releasing a permit happens in the holder's terminal transaction, which holds the holder's run lock.
  Handing the permit to a waiter in that transaction would also need the waiter's run lock: two run locks
  in an arbitrary order, which the project explicitly avoids for unrelated runs.
- Runs are created by many writers: the start API, event and cron triggers, child nodes, reruns, the 017
  ad hoc path, 022 standalone, test and handler-dispatch admission, and the 018 schedule, batch, reset and
  Signal/Update-with-Start writers. The invariant of decision 1 holds only if every one of them takes the
  same admission path. Feature 018's reset copies a journal prefix that can contain a source's
  `concurrency.acquired` (architecture review, 018 B1).

Unknown: the contention cost of one hot key with hundreds of waiters (G1167); it must be measured before a
ceiling above the proposed defaults is published.

## Decision

1. **A permit is a row.** Holders, waiters and per-key counters are PostgreSQL rows keyed by tenant,
   namespace, pool (ordinary or draft-test), permit scope (run or step), group and a keyed digest of the
   key value. They MUST change only in the run transactions below, never in memory or in NATS. The number
   of holders of a key MUST equal the open `concurrency.acquired` events (without a matching
   `concurrency.released`, and ignoring rows a reset re-emitted with 018's `restored: true` marker, i.e.
   sequences up to `runs.reset_sequence`) in the journals of its runs;
   `verify_run` and the fault-suite consistency check MUST assert it. Draft test trees use the draft-test
   pool and never share a row with ordinary runs.
2. **Semaphores follow the path's entity locks; only listed transition rows and usage rows come after.**
   Every transaction that touches a semaphore MUST already hold (or be creating) the run whose permit
   changes and MUST have written that run's `runs` row first (so the `run_active_counts` trigger rows
   precede the semaphore). It MUST lock semaphore rows in `(pool, scope, group, key_digest)` order, after
   every entity lock the path takes before it acquires, releases or grants a permit. After a semaphore it
   MAY lock only (a) the transition rows this ADR lists for that path — on the terminal path the namespace
   failure-handler policy row (`FOR SHARE`) — and (b) quota, rate and usage rows, which always come last
   (ADR-0050 slot 8b, ADR-0027 deferred content flush). It MUST NOT lock another existing run, an aggregate
   count or an activity-quota row after a semaphore. The failure-handler policy row and the cache scope and
   entry rows of feature 022 are the transition rows that ADR-0050 decision 7 lets each ADR list for
   transitions on an existing run; cache rows precede the semaphore when one transaction both completes a
   step from the cache and releases a permit. The concrete orders are:
   - admission: the merged admission order of ADR-0050 decision 7 (coordinator decision C4) — slots 1–5
     (entry, request receipt, run-creating aggregates and existing runs acted on, namespace fences and
     catalog heads, secrets) → slot 6 new run rows (the `runs` insert, whose `run_active_counts` trigger
     rows travel with it) → **slot 7: these semaphores, by (pool, scope, group, key)** → slot 8 bookkeeping
     and usage rows last;
   - grant: waiter shard fence → waiter run → `runs` update → semaphore → usage;
   - queue timeout, cancellation and run deadline of a waiter: run → `runs` update → semaphore → usage;
   - terminal transaction of a holder: run → `runs` update → (cache scope → cache entry, when the same
     transaction completes a step from the cache) → semaphore → namespace failure-handler policy row
     (`FOR SHARE`) → usage;
   - claim: run → drain fence → worker session → sorted activity-quota rows → sorted semaphores → usage;
   - attempt end (completion, failure, cancellation, lease expiry): run → step → semaphore → usage.
3. **Every run creator acquires at admission.** Run admission MUST either acquire a run permit or enqueue
   the new run in its admission transaction, through one application port that every run creator calls:
   the start API, event and cron triggers, child nodes (including `onFailure` children), reruns, 017 ad hoc
   and probe admission, 022 standalone, draft-test and handler-dispatch admission, and 018 schedule firing,
   batch reruns, Signal-with-Start and Update-with-Start. A run created with a start delay (018) evaluates
   its key and acquires or enqueues when the delay elapses, in that timer transaction. A **reset** run
   (018) MUST acquire a fresh permit in its reset admission transaction and MUST NOT queue: when no permit
   is free the reset is refused (`CONCURRENCY_PERMIT_UNAVAILABLE`, 018's contract); `concurrency.*` events
   copied from the source prefix are history (`REMATERIALIZE` in 018's reset-rule registry, as declared in
   `specs/022-flow-control-step-policies/contracts/journal-events.md`). A queued run is
   `WAITING` with wait kind `CONCURRENCY`, has a timer at its queue deadline and no ready task. A step claim
   MUST acquire a step permit in the claim transaction after the activity-quota locks, or leave the step
   `READY` without spending an attempt.
4. **Release never grants.** A holder's terminal transaction (run permit) or attempt-ending transaction
   (step permit) MUST release the permit and MAY write a hint; it MUST NOT lock or modify a waiter run.
5. **Grant by the waiter's shard owner, without head-of-line blocking.** A run permit MUST be granted by the
   scheduler replica that holds the waiter's shard lease, inside a transaction fenced by that lease
   (ADR-0020): lock the waiter run, then the semaphore; verify that the waiter is still queued and not
   paused, that its own declared limit admits one more holder, and that no older unpaused waiter of the key
   that its own limit would admit is queued; then acquire, journal, create the run's first ready tasks and
   outbox, and remove the waiter row. An older waiter whose own (lower) limit is reached does not block a
   younger waiter whose limit admits it; it waits until holders fall below its limit or its queue timeout.
   When feature 019 is present, "oldest" becomes the order (019 priority 1 first, enqueue time ascending)
   among the waiters this rule admits (coordinator decision C19). A grant whose fence, capacity or ordering
   check fails changes nothing.
6. **First commit wins between grant and timeout.** The queue-deadline timer and a grant both lock the
   waiter run first; whichever commits first decides, and the other observes a run that is no longer
   queued and is an audited no-op. A waiter that times out never held a permit and finishes without its
   workflow failure handler.
7. **Policy never revokes, queues are bounded.** Lowering a declared or policy limit MUST NOT release
   existing holders; it only affects new acquisitions. Cancelling a queued run removes its waiter row in the
   cancellation transaction. Queued runs count against a namespace `queued_runs` ceiling in addition to
   `active_runs`, so waiting can never hold more than that share of the namespace. A deployment MUST refuse
   to activate a new digest key version while any holder or waiter exists under the old one.

## Considered options

1. **Durable rows, run-then-semaphore order, grant by the waiter's shard owner** — chosen. One run lock per
   transaction; the existing fence makes a paused or partitioned replica unable to grant; restart needs no
   recovery beyond what the scheduler already does (queued runs are found by an index).
2. **Grant inside the releasing transaction** — rejected: it locks two unrelated runs in a data-dependent
   order (holder then waiter), the pattern ADR-0030 rejected for sorted cross-run locking, and it makes a
   holder's terminal commit fail when the waiter is locked by another writer.
3. **Refuse instead of queue (return `QUOTA_EXCEEDED` when the key is busy)** — rejected as the only mode:
   it pushes durable waiting to every caller and cannot be used by event and cron starts, which must not
   lose work. A full per-key queue still refuses synchronously, and a reset refuses because it reproduces
   a run that was already admitted once.
4. **Advisory locks or an external lock service** — rejected for the same reasons as in ADR-0020: they
   belong to a connection or to a second source of truth, and the action's transaction cannot check them.
5. **Count holders by scanning runs and steps (no semaphore row)** — rejected for run permits: there is no
   row to lock for an empty key and no place to keep the queue. Step permits keep the existing claim-time
   pattern but still use a row so the count is checked under one lock.
6. **One fixed limit per key (instead of the acquirer's limit with skip-ahead)** — rejected: definitions
   and versions sharing a group can legitimately declare different limits during a rollover, and a key that
   never drains would never adopt a new limit. Skip-ahead avoids head-of-line blocking at the cost of
   possible starvation of a lower-limit waiter, bounded by its queue timeout.

## Consequences

- Good: no lost or duplicated permit across crashes, duplicate grants, shard moves or resets; queued work
  holds no lease or worker; the queue is inspectable and its waiters are ordinary runs that can be
  cancelled; draft tests cannot block or be blocked by ordinary runs.
- Bad: a grant costs one extra fenced transaction after release (latency of one scheduler pass); a hot key
  serializes its holders on one row; oldest-first across shards means a younger waiter on a live shard
  waits for an older eligible one on a shard being taken over (bounded by the shard lease, 15 s default); a
  lower-limit waiter can starve until its queue timeout while higher-limit waiters are granted.
- Harder: every future writer that changes a run's permit state, and every future run creator, must follow
  decisions 2 and 3; a writer that needs two runs (for example a future priority preemption) needs a new
  decision.
- Compatibility: additive tables and journal events; definitions without `concurrency` are unaffected.
  Rollback disables the `concurrencyKeys` capability for new publications and starts while existing holders
  and waiters drain with compatible readers; tables are dropped only when empty (down migration refuses
  otherwise).

## Evidence

None yet. Required before acceptance: real PostgreSQL tests for the race in decision 6 (20 rounds), crash
before and after each commit of admission, release, grant and timeout, duplicate grant attempts from two
replicas across a shard takeover, cancel of a queued run, child of the same key, an older lower-limit
waiter with a younger higher-limit waiter, a reset of a permit-holding source (with feature 018), every
run creator of decision 3, claim against attempt-end barriers, and the consistency assertion in decision
1; a benchmark of one key with 1, 16 and 256 contenders on a stated environment. Spec:
`specs/022-flow-control-step-policies/spec.md` (FR-022-017..022); plan:
`specs/022-flow-control-step-policies/plan.md`; research: D1178–D1182, D1196–D1198, D1600, D1602, G1165, G1167; reset rules:
`specs/022-flow-control-step-policies/contracts/journal-events.md`.
