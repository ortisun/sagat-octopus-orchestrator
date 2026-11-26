# Hot-path dispatch: a lean ready queue, wake on commit, the next step in a completion, and a throughput gate

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

The assessment of 2026-11-11 (`docs/roadmap-assessment-2026-11-11.md`, "Remedies") listed hot-path remedies that have no
specification. Design 038 built the statement budget and the cheap drop of a stale candidate. Four remain, and this
design specifies them:

1. **A lean queue of ready steps** (`ready_steps`), filled by a trigger in the transaction that makes a step ready and
   claimed with `FOR UPDATE SKIP LOCKED`, so that concurrent pollers, in one process or many, stop trying the same
   candidates. It is an index of candidates, not an authority: a lease is still decided from the step rows read under
   the run lock.
2. **Wake on commit**: a parked worker stream claims again as soon as a step it serves becomes ready, instead of
   sleeping up to `idle_retry` (500 ms).
3. **The next step in the answer to a completion**: a worker session that completes a step may receive, in the same
   answer, the lease of the next ready step of the same run that it serves, granted by the claim's own routine inside
   the completion's transaction.
4. **A throughput regression gate in CI**, beside design 038's statement budget, on the pull requests that change the
   hot path.

Each is switchable and is kept only if design 051's benchmark shows a gain larger than the range of its repetitions.

This design was the second part of design 051's first draft. That draft's review (`docs/designs/051-competitive-benchmark.reviews/2026-11-24/`)
found it not ready (Blockers and Majors on the queue's refusal paths, the lock order, the protocol minor, the lifecycle
of queue rows, the wake's completeness and the gate's statistics). This draft answers each finding (section "Findings of
the first review and their answers"); it has not yet had a review of its own.

**Terms used below.** A *poll* (unary or a stream) asks the server for work; a *claim* is the transaction that grants a
lease; a *candidate* is a ready step a poll considers; a *probe* is a claim transaction begun for one candidate. The
*run lock* is the row lock on the run that every transaction changing the run takes. *Design 038's budget* is
`tests/integration/claim-transition-efficiency/work_budget.rs`: 38 statements and 2 transactions for a lease, 50 and 1
for a completion that makes the next step ready, at `develop` `546d5aa5`.

## The Problem

From design 038's research (`specs/038-claim-transition-efficiency/research.md`) and the code at `546d5aa5`:

- Probes are two thirds of all database round trips. A poll reads up to 64 candidates with no row lock
  (`crates/infra-postgres/src/workers.rs`) and opens one transaction per candidate, which takes the run row with
  `SKIP LOCKED` (`RUN_FOR_CLAIM`, `crates/infra-postgres/src/execution.rs`). In the closed-loop scenario of 2026-11-12
  there were 58 probes per lease. Design 038's window order spreads pollers within one process only; with four and eight
  API replicas `LWLock:LockManager` reached 4% to 11% of active backends (`tests/performance/reports/scalability-ha.md`).
- A stream that finds nothing sleeps `idle_retry` (500 ms, `crates/application/src/worker.rs`) and claims again
  (`crates/api-grpc/src/service.rs`); a unary poll that finds nothing returns `retry_after_ms = 500` at once. A step that
  becomes ready while its streams sleep waits up to 500 ms: up to 1,500 ms of idle waiting in a chain of three under light
  load. Broker hints are published (`crates/infra-nats/src/lib.rs`) and nothing subscribes to them.
- The answer to `CompleteStep` (`StepCommandResponse`, `specs/001-platform-foundation/contracts/worker.proto`) cannot
  carry a task, so a worker that serves the next activity polls for it.
- No CI job gates throughput. Design 038's budget gates statement counts, which caught the fourfold regression of
  November, but not a slowdown that adds no statement.

## Requirements

### The queue

- **Requirement 052-001 Filled by construction.** Every step that becomes `READY` and is of a kind a worker leases MUST
  get a row in `ready_steps`, inserted by a trigger on `steps` in the transaction that makes it ready, keyed uniquely by
  (tenant, run, step identity, generation), with `ON CONFLICT DO NOTHING`. The row carries what the claim filters on:
  tenant, namespace, activity, `ready_at`, the minimum protocol minor, the kind (forward, compensation, reconciliation),
  the scoped flag and the capacity group, if any. A differential test MUST show that the trigger's "leasable" condition
  equals the domain's for every node kind.
- **Requirement 052-002 Claimed with `SKIP LOCKED`, never lost on a refusal.** A claim MUST take its candidates from
  the queue with `FOR UPDATE SKIP LOCKED`, per binding key (a lateral query per (tenant, namespace, activity), merged by
  `ready_at`, as today's scan is), excluding the rows the same poll has already tried. When the claim leases, it deletes
  the queue rows of the steps it leased (those it can lock with `SKIP LOCKED`; a row another poller holds is deleted by
  that poller when it finds the step leased). When the claim refuses a step that is still `READY` (paused run, stop
  reason, protocol minor, pool-restricted secrets, capacity, an open breaker, a full quota or `max_inflight`), it MUST
  leave the row in place and add it to the poll's exclusion set. When the step is no longer `READY`, it deletes the row.
- **Requirement 052-003 The lock order.** The claim's lock order MUST become: the queue row, then today's order from
  the chain row on (`docs/architecture/execution-semantics.md`: chain, run, drain fence, session, secrets, capacity group,
  activity, semaphores, usage). A queue row MUST be locked only with `SKIP LOCKED`, only by a claim, the sweeper or a
  completion's next-step lease, and always before the chain and run rows; the only transaction that waits for a queue row
  is the trigger's insert on a unique conflict, and it waits only for a transaction that never waits for a run lock. A
  barrier test per lock of the chain, a queue row held, MUST show no deadlock.
- **Requirement 052-004 A bounded table.** A scheduler pass per shard MUST delete, every 30 s and in bounded batches with
  `SKIP LOCKED`, the queue rows whose step is no longer `READY`, and add rows for leasable `READY` steps that have none,
  counting each in `ready_queue_repaired_total` and `ready_queue_swept_total`. The queue's size is a gauge with an
  alert. The queue has no foreign key; a run's purge leaves its rows to the sweeper. In the integration suites a forced
  pass in each suite's teardown MUST find nothing to repair, and `verify_run` MUST assert that every leasable `READY`
  step has a row.
- **Requirement 052-005 Upgrade, switch and rollback.** The switch is a one-row owner table, `ready_queue_switch
  (only_row boolean primary key, filling boolean, claiming boolean, backfilled_at timestamptz)`, read by the runtime
  role and written by owner commands of `control-plane` only. `ready-queue fill` sets `filling`, from which the
  trigger inserts (it reads the row, so with `filling` off it inserts nothing), and backfills the rows of the steps
  already `READY`, recording `backfilled_at`; `ready-queue claim on` sets `claiming`, refused before the backfill, and
  the claims of the processes that read it at their next heartbeat use the queue; `ready-queue claim off` returns them
  to today's scan, the trigger and the sweeper running on, so switching on again is consistent; `ready-queue fill off`
  also stops the trigger and empties the table, so a later `fill` backfills again. The queue needs no registry
  capability (design 053, Requirement 053-006): it is an index of candidates, filled by a database trigger whatever
  the binary; a release before this design claims by today's scan and ignores the queue; a row it leaves for a step it
  leased is deleted by a claim of this design that finds the step no longer `READY`, or by the sweeper. A rollback below
  this design's release turns `fill` off first (the runbook's step, `docs/runbooks/execution-engine.md`), since the
  older release has no sweeper and the trigger would otherwise grow the table; the queue's size alert catches a
  rollback that skipped it.
- **Requirement 052-006 Budgeted.** Design 038's budget MUST also count the rows inserted into and deleted from
  `ready_steps` by a lease and a completion (`pg_stat_xact_user_tables`), recorded in the change that adds the table.

### Wake on commit

- **Requirement 052-010 No lost wake in a process.** A parked stream MUST register its binding keys and read their wake
  counters before its claim, and park only if no counter moved since; a commit that makes a step of those keys ready
  increments the key's counter after the commit. A barrier test MUST commit between a stream's empty claim and its park
  and see the stream claim at once.
- **Requirement 052-011 Wake from every path.** With a notification transport configured, each engine and control-plane
  process MUST subscribe to the hints of ready steps and wake by key; every path that makes work leasable without a
  status change (unpause, a breaker closing, quota released, a binding added) MUST publish a hint too. Without a
  transport, the wake covers the paths of the same process only, and `idle_retry` stays the upper bound for the rest.
  Unary polls are unchanged.
- **Requirement 052-012 No storm.** A woken stream that discovery admission refuses MUST return to parking, not end;
  wakes of one key are coalesced per process within 10 ms.

### The next step in a completion

- **Requirement 052-020 Opt-in, by session.** A worker session MAY set `want_next` on `CompleteStep` in the protocol
  minor that follows delivery acknowledgement's (minor 1.8, reserved for it by Owner decision 2026-11-21-6, pull request 264), together
  with its session token; a completion without a session (task token, push) MUST refuse `want_next` with a closed reason.
- **Requirement 052-021 One list of checks.** When the completion makes a step of the same run ready that the session
  serves, the completion's transaction MUST call the claim's own per-run routine for it, taking its locks in the claim's
  order after the run row (the drain fence before the binding lock, as a scoped claim does), and MUST lease the step only
  if every check passes without writing a refusal; a check that would write (a pool refusal's audit, an oversized input
  that fails the step) declines, and the step stays `READY` for an ordinary claim. The next step is the first such step
  by `ready_at` then step identity; its queue row is deleted in the same transaction; its input fits the answer's byte
  budget or the lease declines.
- **Requirement 052-022 A lease handed to an answer follows the withdrawal rules.** `next_task` is a lease producer
  under Architecture decisions 0098 and 0099: a lease not handed to the transport is withdrawn; an idempotent replay
  (`already_applied`) never re-issues an earlier token, and re-leases the step with a new fence only if it is still
  leased to the same session and not started. The step-status writers' inventory and
  `scripts/check-capacity-activation.py` include the path.

### The gate

- **Requirement 052-030 Throughput gate.** A pull request that changes the hot-path files (design 038's selector) MUST
  run, on the standard hosted runner, the in-process transitions scenario and design 051's driver on the chain of three at
  a fixed rate through the wire, for the base and the head, in the order A B B A A B B A A B B A (six pairs), each on a
  fresh database, and fail when the head's median is slower by more than the minimum detectable effect that a pilot of
  thirty runs measures on that runner, and slower in at least five of six pairs. A one-statement regression is the
  statement budget's to catch, not this gate's.

### Success criteria

- **Success criterion 052-001** With two API replicas and eight pollers each on one activity, probes per lease in the
  closed-loop scenario fall from 58 to at most 2, and in the fan-out shape to at most 2, measured by design 038's
  counters.
- **Success criterion 052-002** With wake on commit and one worker per activity, the 99th percentile of the time from a
  step becoming ready to its lease, in the chain of three at 10% of its maximum sustained rate on the free standard runner,
  is under 50 ms, against up to 500 ms today; the median is reported.
- **Success criterion 052-003** The gate fails on a hand mutation that adds 15% to the completion's measured time and
  passes on its revert, both seen in CI.
- **Success criterion 052-004** Each remedy is measured on and off in design 051's chain of three and fan-out, and is
  removed if it improves neither the maximum sustained rate nor the 99th percentile latency by more than the range of
  the repetitions.

## Goals

- A poll's cost follows what it leases, across replicas, not the number of pollers.
- A step's wait for a lease under light load is not bounded below by a polling interval.
- A slowdown of the hot path fails the pull request that causes it.

## Non-Goals

- **No change of the journal, of the transaction boundary or of who decides a lease.** The queue is an index; the run
  lock and the reducer decide.
- **No push dispatch or worker routing** (feature 019's). Feature 019's priority and fairness keys go into the queue's
  row and order when 019 is built (Task 052-11).
- **No long polls.** The unary poll's behaviour is unchanged.

## Open Questions

1. **Is this design in release 0.3.0's scope?** Yes, confirmed by the coordinating session (Owner decision
   2026-11-24-513): the owner wants to surpass both products on performance, and design 051's benchmark is meaningless
   without these remedies.
2. **Does the CI gate need a constitution amendment?** Yes: the constitution enumerates the suites a pull request runs
   ("and only those"); adding the gate widens them, as design 038's selector rule needed version 4.4.0. The owner
   approved it for constitution 5.1.0 (Owner decision 2026-11-25-3, point 3: a throughput regression gate joins the CI
   gates), which the coordinating session is drafting (the placement in 5.1.0 was Owner decision 2026-11-24-512).
   Task 052-14 waits for its merge; Task 052-15 checks that its text admits the gate as Requirement 052-030 writes it.
3. **What when a refused head blocks the queue?** The poll's exclusion set walks past refused rows, at most today's
   window of 64, so the refused case costs no more than today and the common case costs one probe. Parking refused rows
   until a re-arming event is a follow-up if design 051 shows the refused case matters.

## Owner decisions needed

None open. The constitution's amendment is approved (Owner decision 2026-11-25-3, point 3) and only its merge is
awaited (Task 052-14). The owner pays for no runner in 0.3.0 (Owner decision 2026-11-25-1, "Nada pago"): the on and off
measurement of Task 052-12 runs on the free standard runner, where an on and off pair on one machine is still a fair
comparison, and its run on design 051's larger runner (US$ 208.01 in design 051's "Cost of the campaigns") is
unscheduled; this design claims no number measured on a machine it was not run on.

## Critical User Journeys (CUJs)

### CUJ 1: Many workers on one activity across replicas

*   **Given**: 16 workers on one activity through two API replicas, and a backlog larger than they ask for.
*   **When**: they poll at once.
*   **Then**: each claim locks a different queue row with `SKIP LOCKED`; almost every probe leases.

### CUJ 2: A chain under light load

*   **Given**: one worker per activity of a chain of three, parked.
*   **When**: a run starts.
*   **Then**: the first step's commit wakes its worker's stream, which claims at once; its completion returns the second
    step's lease in the answer; the second completion returns the third's.

### CUJ 3: A refused head of the queue

*   **Given**: the oldest queue rows of an activity belong to a paused run.
*   **When**: a worker polls.
*   **Then**: its claim refuses them, leaves them in place, excludes them for the rest of the poll, and leases the first
    eligible row after them.

## Technical Plan (Architecture)

**The queue.** `ready_steps(id, tenant_id, namespace_id, activity, ready_at, run_id, step_key, generation, min_minor,
kind, scoped, capacity_group)` with a unique key `(tenant_id, run_id, step_key, generation)` and an index `(tenant_id,
namespace_id, activity, ready_at)`. Its own autovacuum settings; its size and dead tuples are gauges. One claim of a
candidate, in its own transaction: lock the next queue row of the session's keys not in the exclusion set (`SKIP
LOCKED`), then today's claim of that run from the chain row on, leasing up to the session's remaining credit of that
run's ready steps and deleting their queue rows that it can lock; on a refusal that leaves the step `READY`, roll back
(the row stays) and exclude it.

**Who locks a queue row.** Claims, the sweeper and the next-step lease, always first and with `SKIP LOCKED`. Every
other writer of a step takes the run lock first and never locks a queue row; the trigger inserts with `ON CONFLICT DO
NOTHING`, which can wait only for a transaction that holds a row of the same key, and such a transaction never waits for
a run lock held by the trigger's transaction (it took the run row with `SKIP LOCKED` or already holds it).

**Wake.** An in-process registry of parked streams by key with a counter per key; the counter is read before the claim
and checked before parking. Commits that make steps ready return their keys where the adapter knows them; the hint
subscriber covers the rest when a transport is configured.

**The next step.** `CompleteStepRequest` gains `want_next` and `session_token`; `StepCommandResponse` gains
`next_task` (the existing `TaskLease` message) and `next_declined` (a closed reason). The completion calls the claim's
per-run routine; the six SDKs set `want_next` when they have credit, behind a setting; the conformance suite gains
`next-task-in-completion`, including a lost answer and a replay.

**The gate.** A job `throughput-gate` in `ci.yml`, selected with design 038's budget target; it shares its harness with
delivery acknowledgement's throughput gate where that exists.

## Findings of the first review and their answers

The first review of these remedies was the review of design 051's first draft (its architecture and correctness reports
of 2026-11-24). How this draft answers each finding that concerned them:

| Finding | Answer here |
|---|---|
| A refused `READY` step has no defined outcome (architecture Blocker 1, correctness Blocker 1) | Requirement 052-002: the row stays, the poll excludes it; Open Question 3 |
| The protocol minor is reserved, and withdrawal rules are ignored (architecture Blocker 2) | Requirements 052-020 and 052-022 |
| The lock order is incomplete; other ADRs state it (architecture Major 1, correctness Nit 1) | Requirement 052-003; Architecture decision 0133 refines 0030, 0050, 0056 and 0078 |
| Lifecycle, uniqueness, sweeper, off switch (architecture Major 2, correctness Majors 2 and 4) | Requirements 052-001, 052-004, 052-005 |
| No rows for runs in flight (architecture Major 3) | Requirement 052-005 |
| Fan-out and multi-step leases (architecture Major 4, correctness Major 1) | Requirement 052-002; Success criterion 052-001 |
| The next step's checks are a second copy; no session; lock order; writing refusals; lost answer; its queue row (architecture Major 5, correctness Majors 3, 13, 14, 15, Nit 11) | Requirements 052-020 to 052-022 |
| Feature 019 and 022 overlap (architecture Major 6) | Non-Goals; Task 052-11 |
| Scope (architecture Major 7) | Open Question 1 |
| The gate's statistics and mutation, the constitution's CI gates (architecture Major 8, correctness Majors 16 and 17, Nit 9) | Requirement 052-030; Open Question 2; Success criterion 052-003 |
| Long polls (architecture Major 9, correctness Major 12) | Non-Goals |
| The completeness check, Success criterion 051-004 passing today, the budget's blind spot (correctness Majors 5, 6, 7) | Requirements 052-004 and 052-006; Success criterion 052-001 with two replicas |
| The query on PostgreSQL 16, dead tuples (correctness Major 8) | Requirement 052-002's lateral query; the gauges of the Technical Plan |
| Lost wake, incomplete wake, wake storm (correctness Majors 9, 10, 11) | Requirements 052-010 to 052-012 |
| Rules without failing checks (architecture Major 17) | Requirements 052-001, 052-003, 052-004 name their tests |

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The queue misses a ready step | The trigger by construction; the sweeper's repair counted; the teardown pass and `verify_run` fail the suites |
| A deadlock through the queue | Requirement 052-003 and its barrier tests |
| The table grows | The sweeper, the size alert, no foreign key |
| A wake storm | Coalescing, return to parking on refusal |
| The next step bypasses a check | One routine shared with the claim; design 038's guards unmodified; the conformance scenario |
| The gate flakes | A measured minimum detectable effect, ABBA order, fresh databases, five of six pairs |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The queue row is written in the transaction that makes the step ready; a lease is decided under the run lock and returned after its own commit |
| II. Modular domain, ports and adapters | Pass | The queue and the wake are in `infra-postgres` and the application's claim loop |
| III. External effects are at-least-once; sagas compensate | Pass | The lease's meaning is unchanged |
| IV. Compatibility is versioned and pinned | Pass | The protocol fields are additive in a new minor; the table is additive; the scan stays as the fallback |
| V. Code, tenant boundary and secrets are isolated | Pass | The next step runs the claim's own authority and binding checks; wake keys are never metric labels |
| VI. Observability and audit are part of the feature | Pass | Repairs, sweeps, size, wakes, next-step grants and declines, all with closed labels |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | The wake uses the outbox hint as a hint |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Barrier tests per lock, a lost wake, a lost answer, a refused head, each seen failing first |
| Delivery Constraints | Pass with conditions | Constitution 5.0.0 as merged; the CI gate needs the CI gates of constitution 5.1.0, approved by the owner (Owner decision 2026-11-25-3, point 3) and being drafted, so Task 052-14 waits for its merge |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| `ready_steps` and its trigger | Probes are two thirds of the round trips, across replicas | Design 038's window order: per process |
| A wake registry and a hint subscriber | Up to 500 ms of idle per step under light load | A shorter `idle_retry`: an empty claim per idle stream per interval |
| Protocol fields for the next step | A poll round trip and a claim per step for a worker that serves the next activity | Wake alone: still a poll and a claim |
| A throughput gate | A slowdown without an added statement passes the budget | Release-time benchmarks only |

## Alternatives Considered

- **The queue as the authority** (Windmill's model). One statement per claim; it moves the lease decision out of the
  run's transaction and changes every guard of features 002, 009 and 010.
- **`LISTEN` and `NOTIFY` for the wake.** Its commit-time lock serializes the commits of every notifying transaction.
- **Row locks on `steps` in the candidate scan.** Design 038's reasons: locks before the chain row, rows the limit
  discards, a poller that cannot lease hiding the head.

## Non-Functional Requirements

- **Non-functional requirement 052-001 Hot path.** A lease and a completion stay within design 038's budget plus the
  queue's rows (Requirement 052-006), recorded in the change that adds them.
- **Non-functional requirement 052-002 Security.** No new grant beyond `ready_steps` for the runtime role; the next
  step's disclosure of secrets stays `ResolveSecrets` under the new lease.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T052-1 Write the review of this design (the egm-review stages and the three repository reviewers) and answer it.
  Refs: Requirement 052-001, Requirement 052-020, Requirement 052-030
- [ ] T052-2 Accept or revise Architecture decisions 0133 and 0134, and update `docs/architecture/execution-semantics.md` with the queue row in the claim's order.
  Refs: Requirement 052-003, Requirement 052-021
- [ ] T052-3 Add `ready_steps`, its trigger, unique key, grants and classification, the differential test of the leasable condition, and the budget's row counts.
  Refs: Requirement 052-001, Requirement 052-006
- [ ] T052-4 Claim from the queue per binding key with the exclusion set and the multi-step lease, with barrier tests per lock and per refusal kind; design 038's guards unmodified.
  Refs: Requirement 052-002, Requirement 052-003, Success criterion 052-001
- [ ] T052-5 Add the sweeper and reconciliation pass, its metrics and alert, the teardown pass and the `verify_run` assertion.
  Refs: Requirement 052-004
- [ ] T052-6 Add `ready_queue_switch`, the commands `ready-queue fill`, `fill off`, `claim on` and `claim off` with the backfill, the rollback step of `docs/runbooks/execution-engine.md`, and the tests `a_claim_of_this_design_deletes_the_row_of_a_step_an_older_release_leased` and `fill_off_stops_the_trigger_and_empties_the_queue`.
  Refs: Requirement 052-005
- [ ] T052-7 Wake parked streams by key with counters read before the claim, coalescing, and return to parking on refusal, with the barrier test of a commit between claim and park.
  Refs: Requirement 052-010, Requirement 052-012, Success criterion 052-002
- [ ] T052-8 Subscribe each process to ready-step hints and publish hints on the paths that re-arm work without a status change.
  Refs: Requirement 052-011
- [ ] T052-9 Add `want_next`, `session_token`, `next_task` and `next_declined` in the next free protocol minor, lease the next step through the claim's routine, and the conformance scenario with a lost answer and a replay.
  Refs: Requirement 052-020, Requirement 052-021, Requirement 052-022
- [ ] T052-10 Set `want_next` in the six SDKs behind a setting, with each SDK's tests.
  Refs: Requirement 052-020
- [ ] T052-11 Amend feature 019's plan and data model so that its priority and fairness keys go into the queue's row and order.
  Refs: Requirement 052-002
- [ ] T052-12 Measure each remedy on and off with design 051's harness on the free standard runner, record the gain or the removal in `tests/performance/reports/hot-path-dispatch.md` with the runner named, and leave the larger runner's run unscheduled (Owner decision 2026-11-25-1).
  Refs: Success criterion 052-002, Success criterion 052-004
- [ ] T052-13 Pilot thirty runs of the gate on the standard runner to measure its minimum detectable effect.
  Refs: Requirement 052-030
- [ ] T052-14 Add the `throughput-gate` job and prove it fails on the hand mutation and passes on its revert.
  Refs: Requirement 052-030, Success criterion 052-003
  Owner-gate: pending, the merge of constitution 5.1.0 (approved by Owner decision 2026-11-25-3, point 3)
- [ ] T052-15 When constitution 5.1.0 merges, check that its CI gates admit the throughput gate as Requirement 052-030 writes it, and record the merge commit here.
  Refs: Requirement 052-030

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs `integration_durable_execution` (also as the runtime role), design 038's budget target
`integration_claim_transition_efficiency`, the fault suites of features 002 and 010, `cargo clippy --workspace
--all-targets --locked -- -D warnings` and the checkers; Tasks 052-4 and 052-9 also run the SDK conformance scenarios
`signals-and-events` and `next-task-in-completion`. The seven guards that design 038 lists in its Verification Plan pass
unmodified. Each new test is seen failing first, with a hand mutation where the behaviour already exists.
