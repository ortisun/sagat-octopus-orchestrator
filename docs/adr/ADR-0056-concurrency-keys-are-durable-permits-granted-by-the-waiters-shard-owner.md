# ADR-0056: Concurrency keys are durable permits taken after the run lock, and a queued run is granted oldest first by its own shard owner

**Status:** Accepted (2026-11-13, OD-2026-11-13-17: the owner's "Aceito tudo")
**Decided by:** agent proposal, accepted by the owner on 2026-11-13 (OD-2026-11-13-17) (2026-10-26; revised after architecture review 2026-10-27)
**Refines / supersedes:** refines ADR-0020 (sharded, fenced scheduler) and ADR-0027 (lock order and
quotas); builds on ADR-0002, ADR-0012 and ADR-0030 (chain-before-run order). Supersedes none.
**Enforced by:** the fault suite `tests/fault/flow-control-step-policies/concurrency.rs` (grant against timeout in four
orders over 20 rounds, a crash inside admission, grant, timeout and release, the release crash over 20 rounds, a takeover
against a grant in flight); `support::flow_control::key_consistency`, run by `World::assert_consistent` after every test
of the suites (counters, rows, gate projections and journals; waiters of ended runs; empty key rows; the oldest wait);
the lock-order tests of `tests/integration/flow-control-step-policies/concurrency_locks.rs` (admission, end of a run,
cancellation, timeout, each against a reference in slot order, and a reversed control that deadlocks); the
architecture checks `gate_row_write_errors`, `key_row_insert_errors`, `run_creator_errors` and
`queued_runs_count_errors` of `scripts/check-architecture.py`, and `runtime_privileges` (every statement as
`sagat_app`). Still missing (2026-11-20): the claim-against-attempt-end barrier, the child-of-the-same-key and
lower-limit-waiter cases at run level, and the benchmark of one key with 1, 16 and 256 contenders (T022-062, T022-089).
*Added 2026-11-27 (feature 022 decision D022-330), for decision 2(a):* the `slot_order.rs` case of a binding replacement
against a lease-expiry pass that releases a cacheable successor (task T022-254); the rule of
`scripts/check-architecture.py` on step and session locks before the lookup round, with its negative fixtures (task
T022-255); the tests that keep the arguments of its allow list (task T022-271).

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
   changes, holding that run's row lock; a commit that ends the run MUST lock its active-run counter row
   (`run_active_counts_lock_slot`) before it gives back any key; the `runs` row is written by `persist` after
   the evaluation rounds (ADR-0094). *(Replaced 2026-11-25, delegate OD-2026-11-24-4, feature 022 decision D022-234,
   accepted 2026-11-25 with the amendments, decision D022-271. The sentence read "and MUST have written that run's `runs` row first, so the `run_active_counts` trigger rows
   precede the semaphore", which the build since ADR-0094 does not do; what it protected, the counter row before
   the key, is kept by the explicit lock. Punctuation revised 2026-11-26, decision D022-285.)* It MUST lock semaphore rows in `(pool, scope, group, key_digest)` order, after
   every entity lock the path takes before it acquires, releases or grants a permit. After a semaphore it
   MAY lock only (a) the transition rows this ADR lists for that path — on the terminal path the namespace
   failure-handler policy row (~~`FOR SHARE`~~, struck 2026-11-25: read without a lock) — and (b) quota, rate and usage rows, which always come last
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

   *Amended 2026-11-24, delegate (OD-2026-11-24-4; feature 022 decisions D022-204 and D022-208, ADR-0135 and ADR-0136
   proposed):* the as-built order. Since ADR-0094 a transition on an existing run decides its keys in evaluation rounds
   before `persist`: the path's entity locks (chain, run; a shard fence only where a pass already holds one) → the run's
   active-run counter row when the commit ends a run that holds a permit (T022-022) → the lookup round's cache scope and
   entry rows, `FOR SHARE` (feature 022 US01) → the gates round's semaphores, then windows, each sorted → `persist`, which
   writes the `runs` row, the journal and the projections → usage rows last. The terminal path reads the namespace
   failure-handler policy row without a lock and records its `config_revision` on the dispatch row, so that row is no
   longer a transition row of this decision. A cacheable step released after the gates round of the same commit (by a
   key failure's route) is looked up in a later round, after the semaphores; this is allowed because the only exclusive
   takers of cache rows (invalidation, purge, erasure, the store writer's insert) lock nothing after one but usage rows
   (ADR-0135 decision 7). *Revised 2026-11-25 (decisions D022-225, D022-234, D022-237):* the lookup round takes the
   secret-access and membership locks of the step's secrets, shared, before the cache rows; the namespace handler's
   revision is ~~read with the run's definition, not on the terminal path~~ read by the left join of the statement that
   locks the run (struck and replaced 2026-11-27, feature 022 decision D022-312, as D022-255 corrected); a domain property test checks that no progress
   emits a filed gate decision and `run.finished` in one commit (task T022-234).

   *Struck and restated 2026-11-25, delegate (OD-2026-11-24-4; feature 022 decisions D022-254 and D022-255; ADR-0094 is
   the authority):* in the per-path orders above, the clauses "→ `runs` update → semaphore" and "namespace
   failure-handler policy row (`FOR SHARE`)" no longer describe any path, and the first order of the amendment of
   2026-11-24 put the counter row before the rounds, which the build does not do. The order of every transition on an
   existing run is: entity locks (the statement that locks the run also reads the namespace handler configuration by a
   left join, `FOR UPDATE OF runs`) → the rounds (lookup, then gates) → `persist`: the active-run counter row when the
   commit ends the run, the give-backs, the rest (the `runs` row, the journal, the projections, the dispatch insert) →
   usage rows. The counter row after cache rows makes no cycle: deciders lock cache rows shared only, and no exclusive
   taker of a cache row takes the counter row.

   2(a). *Added 2026-11-25 (decision D022-254); rewritten 2026-11-25 on its true premise (decision D022-265):* the
   transition rows this decision lists for feature 022's lookup round are the secret-access and membership locks of a
   cacheable step's secret ids (`sagat_lock_secret_access`, `sagat_lock_memberships`, shared, ascending), then, after an
   unlocked pre-read of the entry's immutable `producer_subject`, that subject's membership lock and the activity's
   binding advisory lock (shared), all before the cache rows, the entry read `FOR SHARE` re-checking the subject; at an
   admission the locks and the pre-read are at slot 5 of ADR-0050 decision 7 and the re-check at slot 7. The binding
   lock is not a leaf: a binding replacement takes it exclusively and then locks `worker_sessions` and `steps` rows
   (`crates/infra-postgres/src/bindings.rs`). The rule that keeps it safe is: ~~no decider locks a step or a
   `worker_sessions` row before its lookup round~~ no deciding path locks a `steps` row in `LEASED` or `RUNNING`, or a
   `worker_sessions` row, before its lookup round (restated 2026-11-27, feature 022 decisions D022-282 and D022-308: the
   scoped-event finish locks a `WAITING` step, `crates/infra-postgres/src/execution/scoped_events.rs:164`, and a binding
   replacement updates only `LEASED` or `RUNNING` rows, `crates/infra-postgres/src/bindings.rs:245-246`; its acceptance
   is recorded after the short re-read of these fixes); it is stated here and checked by `scripts/check-architecture.py`
   (~~every `steps ... FOR UPDATE` and every `worker_sessions` lock on a deciding path refused unless its enclosing
   function is on the rule's allow list with its argument~~ every `FOR UPDATE`, `FOR NO KEY UPDATE`, `FOR SHARE`, `FOR
   KEY SHARE` and `UPDATE` of a `steps` or `worker_sessions` row in the adapter refused unless its enclosing function is
   on the rule's allow list with its argument, a deciding path being a transaction that runs `Engine::run`, whose callers
   the rule names; restated 2026-11-27 after the fourth re-reads, feature 022 decisions D022-324 to D022-326, task
   T022-255) and a `slot_order.rs` case (a binding replacement against a lease-expiry pass that releases a cacheable
   successor, task T022-254).

   *Accepted 2026-11-25, delegate OD-2026-11-24-4, after three review rounds recorded in
   specs/022-flow-control-step-policies/reviews/2026-11-25/ (decision D022-271):* the feature 022 amendments of this
   decision dated 2026-11-24 and 2026-11-25.
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

Built (2026-11-20, feature 022 release 0.3.0 slice, pull request 237): the tests named under **Enforced by** above, run
against real PostgreSQL as the schema owner and as `sagat_app`. Not yet: the benchmark below. Originally required: real PostgreSQL tests for the race in decision 6 (20 rounds), crash
before and after each commit of admission, release, grant and timeout, duplicate grant attempts from two
replicas across a shard takeover, cancel of a queued run, child of the same key, an older lower-limit
waiter with a younger higher-limit waiter, a reset of a permit-holding source (with feature 018), every
run creator of decision 3, claim against attempt-end barriers, and the consistency assertion in decision
1; a benchmark of one key with 1, 16 and 256 contenders on a stated environment. Spec:
`specs/022-flow-control-step-policies/spec.md` (FR-022-017..022); plan:
`specs/022-flow-control-step-policies/plan.md`; research: D1178–D1182, D1196–D1198, D1600, D1602, G1165, G1167; reset rules:
`specs/022-flow-control-step-policies/contracts/journal-events.md`.

## Revision (2026-11-13)

Accepted by the owner on 2026-11-13 (OD-2026-11-13-17); the "Decided by" line above predates the acceptance.
The reviews of feature 022's release 0.3.0 slice found that decision 3's step permit at claim leaves
permit-less steps `READY` in the claim's window and that decision 5's strict oldest-first per waiter shard
limits throughput and starves dual-gated heads. [ADR-0089](ADR-0089-a-gated-step-waits-outside-ready-and-one-fenced-grant-routine-serves-every-waiter-by-key.md)
(proposed at the time) would supersede those two decisions; it was accepted the same day (OD-2026-11-13-19).

Note (2026-11-19, OD-2026-11-19-2): the last sentence of decision 7 (no new digest key version while any holder or waiter
exists under the old one) is superseded by ADR-0093, accepted with the operator's force: a key version is rotated per key
with live rows, and an emergency rotation (the operator's force, ADR-0093 decision 5) may over-admit a key by at most what
its old row holds until it drains; meanwhile the order of decision 5 holds within each of the key's two rows, not across
them.

Note (2026-11-20, T022-064, D022-052 to D022-054; delegated by OD-2026-11-20-6): decision 3's delayed start "acquires or
enqueues when the delay elapses" through the planner's `Evaluation::StartGates`, in the release's transaction; when a
ceiling of decision 7 refuses it (a full key queue, `queued_runs`, the live keys), the release is an asynchronous creator
and follows option 3's "must not lose work": nothing is written, and the run is held back from the release pass with a
backoff (5 s doubling to 5 min, `start_release_deferrals`), bounded by the run's own deadline. The release takes the
namespace's admission lock before it counts, so `queued_runs` stays exact. Decision 3's reset "acquires a fresh permit in
the reset admission transaction or is refused" right after `run.reset`, under the reservation rule of ADR-0089 (a permit a
queued run waits for is not free); the source's `concurrency.*` events restored with the prefix are history.
