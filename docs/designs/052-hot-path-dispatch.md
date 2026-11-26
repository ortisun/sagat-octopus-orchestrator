# Hot-path dispatch: a bounded and spread poll, wake on commit, and a throughput gate

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

The assessment of 2026-11-11 (`docs/roadmap-assessment-2026-11-11.md`, "Remedies") listed hot-path remedies that have no
specification. Design 038 built the statement budget and the cheap drop of a stale candidate. This design specifies the
next three, chosen so that each costs no new table, protocol field or authority and is measured before anything heavier
is built:

1. **A bounded and spread poll**: a poll stops after a run of probes that leased nothing, and a poll that met
   contention starts its next window at a point spread by its session, so that pollers of several replicas stop trying
   the same candidates.
2. **Wake on commit**: a parked worker stream claims again as soon as a step it serves becomes ready, instead of
   sleeping up to `idle_retry` (500 ms), within a process and, best effort, across processes.
3. **A throughput regression gate in CI**, beside design 038's statement budget, on the pull requests that change the
   hot path.

Two heavier remedies of this design's earlier drafts are not built in 0.3.0 and wait for this design's measurement
(Open Questions 2 and 3): a queue of ready steps claimed with `SKIP LOCKED`, and the lease of the next step in the answer
to a completion. Each remedy here is switchable and is kept only if its on and off measurement shows a gain larger than
the range of its repetitions.

This design was the second part of design 051's first draft, whose review found it not ready; its second draft was
reviewed in full on 2026-11-26 (`docs/designs/052-hot-path-dispatch.reviews/2026-11-26/`), and this draft answers that
round, chiefly by measuring the cheap remedies before the queue and the next-step lease.

**Terms used below.** A *poll* (unary or a stream) asks the server for work; a *claim* is the transaction that grants a
lease; a *candidate* is a ready step a poll considers, read by the candidate scan (`claimable_paths`,
`crates/infra-postgres/src/workers.rs`) with every filter the scan applies in SQL; the *window* is the list of up to 64
candidates a poll reads (four times what it asks for); a *probe* is a claim transaction begun for one candidate. The
*run lock* is the row lock on the run that every transaction changing the run takes; a claim takes it with
`SKIP LOCKED` (`RUN_FOR_CLAIM`, `crates/infra-postgres/src/execution.rs`), so a probe of a run another transaction holds
leases nothing. *Design 038's budget* is `tests/integration/claim-transition-efficiency/work_budget.rs`: 38 statements
and 2 transactions for a lease, 50 and 1 for a completion that makes the next step ready, at `develop` `546d5aa5`.
*Design 038's turn* is `crates/application/src/worker/poll_order.rs`: a poll starts past the polls of its namespace in
flight in the same process (its Requirement FR-038-004).

## The Problem

From design 038's research (`specs/038-claim-transition-efficiency/research.md`) and the code at `546d5aa5`:

- Probes are two thirds of all database round trips. A poll reads up to 64 candidates with no row lock and opens one
  transaction per candidate. In the closed-loop scenario of 2026-11-12 there were 58 probes per lease, a scenario with
  more demand than supply in which workers spin (the research, item 5). Design 038's turn spreads pollers within one
  process only, and a poll walks its whole window, wrapping round, even after a run of probes that found their runs
  locked; with four and eight API replicas `LWLock:LockManager` reached 4% to 11% of active backends
  (`tests/performance/reports/scalability-ha.md`).
- A stream that finds nothing sleeps `idle_retry` (500 ms, `crates/application/src/worker.rs`) and claims again
  (`crates/api-grpc/src/service.rs`); a unary poll that finds nothing returns `retry_after_ms = 500` at once. A step that
  becomes ready while its streams sleep waits up to 500 ms: up to 1,500 ms of idle waiting in a chain of three under light
  load. Broker hints are published through the outbox (`crates/scheduler/src/outbox.rs`, `crates/infra-nats/src/lib.rs`),
  each an acknowledged JetStream publish at the scheduler's pace, and nothing subscribes to them.
- No CI job gates throughput. Design 038's budget gates statement counts, which caught the fourfold regression of
  November, but not a slowdown that adds no statement.

## Requirements

### The poll

- **Requirement 052-001 A bounded walk.** A poll MUST end its walk after 8 consecutive probes that leased nothing
  (whatever the reason: a run another transaction holds, a step leased meanwhile, a refusal), in place of design 038's
  rule that a poll tries every candidate of its window (FR-038-004, refined: the order stays a preference, and the walk
  is now bounded). The ineligible sample that ADR-0075's activation condition (d) puts behind every eligible candidate
  (`INELIGIBLE_SAMPLE`, 4) and the scan's sample of pool-refused steps kept for the claim's audit are read and tried as
  today, behind the eligible window, and are not counted in the 8. A stream whose poll ended so parks (Requirement
  052-010) or, without a wake, waits its `idle_retry`.
- **Requirement 052-002 A spread start under contention.** A poll MUST start its eligible window as design 038's turn
  says, plus, when the previous poll of the same session ended by Requirement 052-001 or began with a probe that leased
  nothing, an offset of `h × wanted` positions modulo the window's length, where `h` is the session identifier's hash
  (`xxh3` of its bytes, modulo 8) and `wanted` what the poll asks for. A poll that met no contention keeps the scan's
  order, so a poll alone still tries the oldest candidate first. The offset is the server's, never the client's.
- **Requirement 052-003 Counted.** The poll MUST count, beside design 038's counters of fetched and probed candidates,
  the polls ended by the bound and the polls that started spread (`worker_poll_ended_by_bound_total`,
  `worker_poll_spread_total`, closed labels), and design 038's barrier test of K pollers MUST also run with two
  claimers that stand for two replicas (two application instances on one database, each with its own design 038 turn).

### Wake on commit

- **Requirement 052-010 No lost wake in a process.** A parked stream MUST register its binding keys (tenant, namespace,
  activity) and read their wake counters before its claim, and park only if no counter moved since; a commit that makes
  a step of those keys ready increments the key's counter after the commit. A barrier test MUST commit between a
  stream's empty claim and its park and see the stream claim at once.
- **Requirement 052-011 A best-effort wake across processes.** With a NATS connection configured, the process that
  commits a transaction making steps ready MUST publish, right after the commit and outside it, a core NATS message
  (not the outbox and not JetStream: a lost one costs at most `idle_retry`) on
  `sagat.wake.v1.<tenant>.<namespace>.<activity>`, outside the platform stream's filter `sagat.v1.>` so that JetStream
  never stores it, through a core-publish method of `crates/infra-nats` limited to that prefix, its tokens encoded as
  the broker's subject rules require (and the broker's subject permissions for the platform's user, where configured,
  gain `sagat.wake.v1.>`), once per
  key the transaction made ready and did not itself lease; each control plane and engine that serves worker streams
  subscribes and wakes by key. The paths that make work leasable with no commit of a step (a breaker that closes with
  time, quota freed by a lease's expiry) and those without a status change stay bounded by `idle_retry`. Without NATS,
  the wake covers the commits of the same process only. Unary polls are unchanged.
- **Requirement 052-012 No storm.** A wake MUST wake at most as many parked streams of the key as steps the commit made
  ready, coalesced per process within 10 ms; a woken stream that discovery admission refuses returns to parking, not to
  its end.

### The gate

- **Requirement 052-030 Throughput gate.** A pull request that changes the hot-path files (design 038's selector) MUST
  run, on the standard hosted runner, the in-process transitions scenario of `tests/performance/scalability-ha/`
  (a fixed number of chains of three run as fast as the scenario's workers complete them, its metric the completed
  transitions per second over the run, which moves with any slowdown since the load is closed), for the base and the
  head in the order A B B A A B B A A B B A (six pairs), each on a fresh database. It fails when the head is slower in
  all six pairs and the median of the six paired differences exceeds the minimum detectable effect
  MDE = (t(0.975, 5) + t(0.8, 5)) × sd / √6 = (2.571 + 0.920) × sd / 2.449 = 1.425 × sd, where sd is the standard
  deviation of the paired differences measured by a pilot of 30 A/A pairs on that runner. Under no change the first
  condition alone fires with probability 1/64, about 1.6%; the pilot's report states the false-positive rate it
  measured and the power at a 15% slowdown. A one-statement regression is the statement budget's to catch, not this
  gate's; the selector does not see a slowdown from a migration or a dependency's update, which the release's
  benchmark does.

### Success criteria

- **Success criterion 052-001** With two application instances and eight pollers each on one activity, in an open-loop
  run at 80% of the maximum sustained rate (not the spinning closed loop), probes per lease fall from today's value,
  measured first in the same run, to at most 4, counted by design 038's counters; above 4, Open Question 2's queue is
  written.
- **Success criterion 052-002** With wake on commit and one worker per activity, the 99th percentile of the time from a
  step becoming ready to its lease, in the chain of three at 10% of its maximum sustained rate on the free standard
  runner, with two application instances and runs started through the instance that does not hold the worker's stream,
  is under 50 ms, against up to 500 ms today; the median is reported.
- **Success criterion 052-003** The gate fails on a hand mutation that adds 15% to the completion's measured time and
  passes on its revert, both seen in CI, and the pilot's false-positive rate and power are reported.
- **Success criterion 052-004** Each remedy is measured on and off on the free standard runner and is removed if it
  improves neither the maximum sustained rate nor the 99th percentile latency by more than the range of the
  repetitions.

## Goals

- A poll's cost follows what it leases, across replicas, not the number of pollers.
- A step's wait for a lease under light load is not bounded below by a polling interval.
- A slowdown of the hot path fails the pull request that causes it.

## Non-Goals

- **No change of the journal, of the transaction boundary or of who decides a lease.** The run lock and the reducer
  decide, as today.
- **No new table and no protocol change in 0.3.0**: the queue and the next-step lease wait for measurement (Open
  Questions 2 and 3).
- **No push dispatch or worker routing** (feature 019's).
- **No long polls.** The unary poll's behaviour is unchanged.

## Open Questions

Each was decided under the delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4, with its reasoning.

1. **Is this design in release 0.3.0's scope?** Yes (Owner decision 2026-11-24-513): the owner wants to surpass both
   products on performance. Its remedies are justified by their own on and off measurement on the free standard runner
   (Success criterion 052-004), not by design 051's paid campaign, which is unscheduled (Owner decision 2026-11-25-1).
   Delegate.
2. **A queue of ready steps?** Not in 0.3.0 unless Success criterion 052-001 is missed. The earlier drafts' queue is an
   index of candidates claimed with `SKIP LOCKED`; its review (`critic_report.md` and `zoom_out_report.md` of this
   design's reviews) showed what it needs that the bounded and spread poll does not: the scan's own filters joined under
   the row lock, one row per run and activity for fan-out, an unlocked peek before a single locked row, a safe switch
   and backfill, a counted budget and a trigger that can change under design 053's rules. Those are a design of their
   own, written only if the measurement asks for it. Delegate (Owner decision 2026-11-24-525).
3. **The next step in the answer to a completion?** Not in 0.3.0. A woken stream already receives its next task by push,
   so what remains is one claim transaction; the remedy would be a second lease producer inside the completion (its
   locks ordered after the completion commits, under ADR-0098 and ADR-0099's withdrawal rules, marked provisional and
   acknowledged from protocol 1.8), in a protocol minor behind those `sdks/SPEC.md` already reserves (1.9 for design
   045, 1.10 to 1.12 for draft 019), in six SDKs. It is a follow-up if Success criterion 052-002 shows the claim after a
   wake dominates. Delegate (Owner decision 2026-11-24-525).
4. **Does the CI gate need a constitution amendment?** Yes: the constitution enumerates the suites a pull request runs
   ("and only those"); adding the gate widens them, as design 038's selector rule needed version 4.4.0. The owner
   approved it for constitution 5.1.0 (Owner decision 2026-11-25-3, point 3: a throughput regression gate joins the CI
   gates), which the coordinating session is drafting (the placement in 5.1.0 was Owner decision 2026-11-24-512). Task
   052-7 waits for its merge; Task 052-8 checks that its text admits the gate as Requirement 052-030 writes it.

## Owner decisions needed

None open. The constitution's amendment is approved (Owner decision 2026-11-25-3, point 3) and only its merge is
awaited (Task 052-7). The owner pays for no runner in 0.3.0 (Owner decision 2026-11-25-1, "Nada pago"): every
measurement here runs on the free standard runner, where an on and off pair on one machine is a fair comparison; this
design claims no number measured on a machine it was not run on.

## Critical User Journeys (CUJs)

### CUJ 1: Many workers on one activity across replicas

*   **Given**: 16 workers on one activity through two application instances, and a backlog larger than they ask for.
*   **When**: they poll at once.
*   **Then**: the first polls meet locked runs; their sessions' next polls start spread by their hashes; a poll that
    leases nothing in 8 probes ends and parks instead of walking 64.

### CUJ 2: A chain under light load

*   **Given**: one worker per activity of a chain of three, its streams parked, held by the instance that did not start
    the run.
*   **When**: a run starts.
*   **Then**: the first step's commit publishes a wake that the other instance receives; the worker's stream claims at
    once; each completion's commit wakes the next activity's stream the same way.

### CUJ 3: A slow change to the hot path

*   **Given**: a pull request that touches the claim and slows it without adding a statement.
*   **When**: CI runs the gate.
*   **Then**: the head is slower in all six pairs and by more than the minimum detectable effect, and the pull request
    fails.

## Technical Plan (Architecture)

**The poll.** `claim()` in `crates/application/src/worker.rs` counts consecutive probes that leased nothing and ends the
walk at 8; `poll_order.rs` adds the session's offset to design 038's turn when the session's previous poll met
contention, a flag kept on the session's in-memory state (not stored). The scan and its samples are unchanged.

**Wake.** An in-process registry of parked streams by key with a counter per key, read before the claim and checked
before parking. The store's commit of a transaction that made steps ready returns their keys (the execution store knows
them where it writes the `task.ready` hint today); the process increments the local counters and publishes one core
NATS message per key, after the commit; a subscriber in each process that serves worker streams wakes by key. The
`task.ready` outbox hint stays for its other readers.

**The gate.** A job `throughput-gate` in `ci.yml`, selected with design 038's budget target, running the in-process
transitions scenario for base and head, its pilot's sd recorded in `tests/performance/reports/throughput-gate.md`.

| Change | Files | Requirement |
|---|---|---|
| The bounded and spread poll and its counters | `crates/application/src/worker.rs`, `crates/application/src/worker/poll_order.rs`, `tests/integration/claim-transition-efficiency/` | 052-001, 052-002, 052-003 |
| The wake | `crates/application/src/worker.rs`, `crates/api-grpc/src/service.rs`, `crates/infra-postgres/src/execution.rs`, `crates/infra-nats/src/lib.rs` | 052-010, 052-011, 052-012 |
| The gate | `.github/workflows/ci.yml`, `tests/performance/scalability-ha/`, `tests/performance/reports/throughput-gate.md` | 052-030 |

**Compatibility and rollback.** Nothing is added to the schema or the protocol. Each remedy is a setting of the server
(`SAGAT_POLL_BOUND`, `SAGAT_POLL_SPREAD`, `SAGAT_WAKE`, each on by default once kept), so a rollback is a setting, and a
mixed fleet of releases needs no capability (design 053, Requirement 053-006): an older process walks its window and
sleeps its `idle_retry`, and ignores wake messages it does not subscribe to.

## Findings of the review rounds and their answers

The answers to the review of design 051's first draft are in `docs/designs/052-hot-path-dispatch.reviews/2026-11-26/`
beside this round's reports, which list each finding and where this draft answers it.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The bound hides eligible work deeper in the window | The next poll spreads its start; the counters show bound-ended polls; Success criterion 052-001 measures leases |
| The spread breaks the oldest-first order | Only after contention; a poll alone keeps the scan's order |
| A lost or late wake | Core NATS is a hint; `idle_retry` stays the upper bound |
| A wake storm | At most as many streams as steps made ready; coalescing; return to parking on refusal |
| The gate flakes | A measured sd from 30 A/A pairs, six of six pairs and the median beyond the minimum detectable effect |
| The cheap remedies are not enough | Success criterion 052-001's threshold writes the queue's design |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No transition changes; a lease is decided under the run lock as today |
| II. Modular domain, ports and adapters | Pass | The poll's order and bound in the application's claim loop; the wake's publish in `infra-nats`, its registry in the application |
| III. External effects are at-least-once; sagas compensate | Pass | Unchanged |
| IV. Compatibility is versioned and pinned | Pass | No schema or protocol change; settings only |
| V. Code, tenant boundary and secrets are isolated | Pass | A wake carries only the key, on the platform's subjects; the claim's authority and binding checks unchanged; wake keys are never metric labels |
| VI. Observability and audit are part of the feature | Pass | Bound-ended and spread polls, wakes published, received and coalesced, all with closed labels |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | The wake is a hint outside the outbox; nothing depends on its delivery |
| VIII. Failure is tested and contracts are verified, honestly | Pass | The two-claimer barrier test, the lost-wake barrier test, the gate's mutation and pilot, each seen failing first |
| Delivery Constraints | Pass with conditions | Constitution 5.0.0 as merged; the CI gate needs the CI gates of constitution 5.1.0, approved by the owner (Owner decision 2026-11-25-3, point 3) and being drafted, so Task 052-7 waits for its merge |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A bound and a spread start | Probes per lease across replicas | Nothing: the walk stays at 64 |
| A wake registry and a core NATS subscriber | Up to 500 ms of idle per step under light load | A shorter `idle_retry`: an empty claim per idle stream per interval |
| A throughput gate | A slowdown without an added statement passes the budget | Release-time benchmarks only |

## Alternatives Considered

- **A queue of ready steps claimed with `SKIP LOCKED`** (this design's earlier drafts). It makes probes per lease near
  one, at the cost of a high-turnover table, a trigger on `steps`, the scan's filters joined under its lock, a switch and
  a backfill; built only if Success criterion 052-001 is missed (Open Question 2).
- **The queue as the authority** (Windmill's model). One statement per claim; it moves the lease decision out of the
  run's transaction and changes every guard of features 002, 009 and 010.
- **The next step in the answer to a completion** (the earlier drafts). Open Question 3.
- **`LISTEN` and `NOTIFY` for the wake.** Its commit-time lock serializes the commits of every notifying transaction.
- **The outbox's `task.ready` hint for the wake.** An acknowledged JetStream publish at the scheduler's pace; a wake
  needs to be fast and may be lost.
- **A shared turn across processes.** A hot counter per namespace.

## Non-Functional Requirements

- **Non-functional requirement 052-001 Hot path.** A lease and a completion stay within design 038's budget unchanged:
  the bound and the spread change only which candidates a poll tries; the wake's publish is after the commit and
  outside the database.
- **Non-functional requirement 052-002 Security.** No new database grant; a wake message carries tenant, namespace and
  activity only, on the `sagat.wake.v1.>` subjects, under the broker's existing credentials; a forged wake costs one
  empty claim, as a stream's `idle_retry` does.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T052-1 Revise Architecture decisions 0133 (the bounded and spread poll, refining design 038's FR-038-004) and 0134 (the wake), and update `docs/architecture/execution-semantics.md`'s description of the poll.
  Refs: Requirement 052-001, Requirement 052-002, Requirement 052-011
- [ ] T052-2 Bound the walk and spread the start under contention, with their counters and settings, the tests `a_poll_ends_after_eight_probes_that_leased_nothing`, `the_samples_behind_the_eligible_window_are_tried_and_not_counted`, `a_poll_alone_keeps_the_order_of_the_scan` (kept), `a_session_that_met_contention_starts_spread_by_its_hash`, and design 038's barrier test of K pollers run with two claimers; design 038's guards unmodified.
  Refs: Requirement 052-001, Requirement 052-002, Requirement 052-003
- [ ] T052-3 Wake parked streams by key with counters read before the claim, at most as many as steps made ready, coalescing, and return to parking on refusal, with the barrier test `a_commit_between_an_empty_claim_and_the_park_wakes_the_stream`.
  Refs: Requirement 052-010, Requirement 052-012
- [ ] T052-4 Publish the core NATS wake after the commit of a transaction that made steps ready, subscribe in each process that serves worker streams, with the tests `a_wake_from_another_instance_claims_at_once` and `a_lost_wake_is_bounded_by_idle_retry`.
  Refs: Requirement 052-011
- [ ] T052-5 Measure each remedy on and off on the free standard runner with design 051's harness, Success criteria 052-001 and 052-002 with two application instances, and record the gain or the removal in `tests/performance/reports/hot-path-dispatch.md` with the runner named; when Success criterion 052-001 is missed, open the queue's design (Open Question 2).
  Refs: Success criterion 052-001, Success criterion 052-002, Success criterion 052-004
- [ ] T052-6 Pilot thirty A/A pairs of the gate on the standard runner, record sd, the false-positive rate and the power at 15% in `tests/performance/reports/throughput-gate.md`.
  Refs: Requirement 052-030
- [ ] T052-7 Add the `throughput-gate` job and prove it fails on the hand mutation and passes on its revert.
  Refs: Requirement 052-030, Success criterion 052-003
  Owner-gate: pending, the merge of constitution 5.1.0 (approved by Owner decision 2026-11-25-3, point 3)
- [ ] T052-8 When constitution 5.1.0 merges, check that its CI gates admit the throughput gate as Requirement 052-030 writes it, and record the merge commit here.
  Refs: Requirement 052-030

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs `integration_durable_execution` (also as the runtime role), design 038's budget target
`integration_claim_transition_efficiency`, the fault suites of features 002 and 010, `cargo clippy --workspace
--all-targets --locked -- -D warnings` and the checkers. The seven guards that design 038 lists in its Verification Plan
pass unmodified. Each new test is seen failing first, with a hand mutation where the behaviour already exists.
