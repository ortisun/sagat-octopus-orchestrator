# Claim and transition efficiency

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-12

--------------------------------------------------------------------------------

## Overview

Make a step transition cost the database what it needs and no more: cut the work a worker's claim and
report do in PostgreSQL, put a budget on it that a test enforces, and remove the serialisation points
that are not required by a guarantee. No guarantee of feature 002 or 010 changes.

## The Problem

The transitions scenario of feature 010 (`tests/performance/scalability-ha/run.sh`, three chained
activities per run) reached 1,513 transitions per second on 2026-10-11. On 2026-11-11, at `develop`
`ed11d044` on the same machine, it reached 368. Pull request 203 removed two advisory-lock mutexes and
the figure rose to 763, with the machine busier than in October. It is still half.

A run of the same scenario with `pg_stat_statements` on (1,000 runs per configuration, nine
configurations, 27,000 leases and 27,000 completions, with pull request 203 applied) recorded:

| What | Count | Per lease |
|---|---:|---:|
| Polls (candidate scans) | 15,319 | 0.57 |
| Candidate rows returned by the scans | 910,716 | 33.7 |
| Transactions begun | 966,647 | 35.8 |
| Run rows read to probe a candidate | 904,750 | 33.5 |
| `chain_segments` lookups, all returning no row | 940,750 | 34.8 |
| Claim transactions that got the run lock and loaded the run | 203,567 | 7.5 |
| Leases granted | 27,000 | 1 |
| `run_events` rows inserted | 126,000 | 4.7 |
| `UPDATE steps SET node_kind` (one row each) | 54,000 | 2 |

So a lease costs about 34 probe transactions and 7.5 full run loads, because every poller of an
activity reads the same oldest candidates (up to 64 of them) and opens a transaction for each. The
loads themselves read, for a workflow that has none of them, the run's sagas, scope sagas, iteration
scopes, collections, loop iterations and chain, one statement each (239,567 times each here).

The same profile shows the start of a run waiting, on average, 102 ms for an exclusive advisory lock
(20,899 acquisitions for 9,000 starts at a concurrency of 80): the namespace and tenant active-run
quota locks. That is outside the transitions figure and inside the start figure of the report.

With four and eight API replicas the backends wait on `Lock:transactionid` (up to 29%),
`LWLock:BufferContent` (21%) and `LWLock:MultiXactGen` (7%). Every report locks one membership row
`FOR SHARE` and every claim locks the worker-session row `FOR UPDATE`; that these are the rows is a
hypothesis the first task confirms or discards.

These figures come from one machine that was busy with other work. The counts do not depend on the
load; the rates do, and are stated as such wherever they appear.

## Requirements

- **FR-038-001** A test MUST fail when the statements, transactions or rows written per lease and per
  completion of a fixed workflow shape exceed a recorded budget, and the budget MUST be lowered in the
  change that earns it.
- **FR-038-002** A poll MUST NOT open a transaction for a candidate that another poller is known, at
  the time of the scan, to hold or to have leased.
- **FR-038-003** Concurrent pollers of one activity MUST NOT all receive the same candidates, while
  the oldest ready step of an activity is still leased within a bound stated in the design's tests.
- **FR-038-004** Loading a run for a command MUST NOT read a kind of state the run's pinned definition
  cannot have.
- **FR-038-005** A transition MUST NOT rewrite a step's `node_kind` when it already holds the value.
- **FR-038-006** Starting a run MUST NOT wait for another start of the same namespace or tenant while
  the active-run limits are not near; the limits MUST stay exact.
- **FR-038-007** The authority and session checks of a report and of a claim MUST keep what they prove
  (a revocation or a session replacement that has returned stops the next command) without every
  command of a namespace locking one shared row.
- **SC-038-001** On a machine whose load average is at most the one of the October report, the
  transitions scenario reaches at least 1,513 transitions per second at best, and does not fall below
  its one-worker-group figure when API replicas are added.
- **SC-038-002** Leases per claim transaction rise from 0.13 (27,000 of 203,567) to at least 0.8, and
  candidate probes per lease fall from 33.7 to at most 2, in the profiled scenario.
- **NFR-038-OBS** The counts above are exported as metrics (candidates scanned, probes, claim
  transactions, leases) so that the ratio is visible in production, with no tenant or activity label.

## Goals

- A regression of the hot path fails a test, not a benchmark someone has to remember to run.
- The October figure is passed on the same machine, with the reason for each gain stated.
- Adding API replicas does not lower throughput.

## Non-Goals

- No change of the journal, of the transactional boundary or of the fencing.
- No second database, cell or partitioning: those are the ceiling of one primary and a later design.
- No push dispatch or hint subscription (feature 019 and a later design).
- No change of the worker protocol.

## Open Questions

- Whether `FOR UPDATE … SKIP LOCKED` in the candidate scan itself (the scan becomes the claim of the
  simple case) is allowed for the runtime role on `steps` and `runs`, and what it does to the
  oldest-first order G246 records. The first slice measures before this is chosen.
- Whether oldest-first per activity is a guarantee or a habit. FR-038-003 assumes a bound is enough.

## Owner decisions needed

- Accept this design for implementation (it is new scope: the assessment of 2026-11-11 listed it as
  having no specification).
- Whether a bounded departure from oldest-first claiming is acceptable (FR-038-003).

## Critical User Journeys (CUJs)

### CUJ 1: Many workers on one activity

*   **Given**: 128 workers polling one activity through eight API replicas, and a backlog of ready steps.
*   **When**: they poll at once.
*   **Then**: each receives different steps, almost every claim transaction grants a lease, and the
    throughput is not lower than with one replica.

### CUJ 2: A change that makes the hot path heavier

*   **Given**: a change that adds a statement to every completion.
*   **When**: its pull request runs the suite it touched.
*   **Then**: the budget test fails and names the statement, and the author either removes it or raises
    the budget with the reason in the same change.

## Technical Plan (Architecture)

The work stays in `crates/infra-postgres` (statements) and `crates/application` (the claim loop), with
the budget test in `tests/integration/durable-execution`.

1. **Measure as a test.** A test runs a fixed shape (three chained activities, a fixed number of runs
   and of concurrent sessions) against a database with `pg_stat_statements`, and asserts ceilings on
   statements, transactions and written rows per lease and per completion. `tests/integration/
   workflow-primitives/scope_frontier.rs` already asserts work per transition for one statement; this
   generalises it. The first budget is today's count, so the test is green before anything changes.
2. **Find the contended rows.** Sample `pg_locks` with the relation and tuple during the eight-replica
   configuration and name the rows behind `Lock:transactionid` and the multixact waits.
3. **Stop the herd.** The candidate scan returns, per poll, candidates that other polls in flight are
   unlikely to take, and skips runs that are locked. Candidates considered, in order of preference
   after step 1's measurement: skip-locked selection inside the scan; a per-poll offset into the
   oldest window; a hash partition of the window by session. One probe transaction per candidate
   becomes one per poll.
4. **Load what the definition has.** The pinned definition says whether a run can have sagas, scopes,
   loops, collections or a chain; the load skips the statements for kinds it cannot have.
5. **Do not rewrite `node_kind`.** `ensure_node_kind` updates only when the stored value differs.
6. **Starts.** The active-run admission reads its counters and takes its exclusive lock only when the
   count is within a margin of the limit that the concurrent starters cannot exceed; otherwise it
   shares it. The margin and its proof of exactness are in the task's test.
7. **Authority and session rows.** Decided after step 2. The candidates are a version check instead of
   a row lock for the membership, and one session renewal per poll instead of one lock per candidate.

Each step is its own pull request, in this order, each with the budget lowered.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A cheaper claim weakens a fence | Every change keeps the existing fault and concurrency suites green, and adds a held-lock test for the interleaving it touches; trust-boundary slices get the independent security and correctness reviews |
| Departing from oldest-first starves an old step | FR-038-003's bound is a test; the oldest step is always inside the window every poll reads |
| A budget test that flakes | It asserts counts, which do not depend on machine load, on a fixed shape with a fixed concurrency |
| Skipping loads by definition misses state | The decision is made from the pinned definition, and `verify_run` (replay equals projection) runs in the suites |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | No change of the journal, the transaction boundary or the fences (Non-Goals) |
| II. Modular domain, ports and adapters | Pass | Statements stay in `infra-postgres`; the domain is not touched |
| III. External effects are at-least-once; sagas compensate | n/a | No external effect changes |
| IV. Compatibility is versioned and pinned | Pass | No protocol or schema change is planned; a migration, if step 3 needs an index, is additive |
| V. Code, tenant boundary and secrets are isolated | Pass with conditions | Step 7 changes how authority is rechecked; see Complexity Tracking |
| VI. Observability and audit are part of the feature | Pass | NFR-038-OBS |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Budget test first and seen green at today's counts, then lowered; races with held locks |
| Delivery Constraints | Pass | Single region, one primary |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A different authority recheck (step 7) | One shared row locked by every command of a namespace is a contention point | Keeping the row lock: measured as the suspect of the multixact waits; kept if step 2 clears it |
| A conditional exclusive lock at start (step 6) | Starts of a namespace wait 102 ms on average for each other | Always exclusive: it is the measured wait |

## Alternatives Considered

- **A separate task queue table claimed with one statement**, as Windmill does. It is the larger
  redesign and changes what the projection is; it is kept for after the budget shows what the present
  tables can do.
- **Raising the connection pool or adding replicas.** The profile shows more replicas lower the
  throughput; more connections add probes.
- **Push dispatch.** It removes polls, not the per-candidate transactions, and belongs to feature 019.

## Non-Functional Requirements

- **NFR-038-PERF** SC-038-001 and SC-038-002, measured with the environment and the load stated.
- **NFR-038-OBS** as above.
- **NFR-038-SEC** No new grant to the runtime role without the privilege audit passing.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T038-1 Add the work-per-lease and work-per-completion budget test at today's counts.
  Refs: FR-038-001
- [ ] T038-2 Name the rows behind the transaction-id and multixact waits with eight API replicas.
  Refs: FR-038-007
- [ ] T038-3 Skip the `node_kind` rewrite when the value is already stored, and lower the budget.
  Refs: FR-038-005
- [ ] T038-4 Skip the loads of state the pinned definition cannot have, and lower the budget.
  Refs: FR-038-004
- [ ] T038-5 Replace one probe transaction per candidate by one per poll, and lower the budget.
  Refs: FR-038-002
- [ ] T038-6 Give concurrent pollers different candidates within a tested bound on the oldest step.
  Refs: FR-038-003, SC-038-002
- [ ] T038-7 Share the active-run admission lock away from the limit, with the exactness test.
  Refs: FR-038-006
- [ ] T038-8 Change the authority and session rechecks as T038-2 decides, with independent reviews.
  Refs: FR-038-007
- [ ] T038-9 Export the claim ratio metrics.
  Refs: NFR-038-OBS
- [ ] T038-10 Run the transitions scenario on a quiet machine and regenerate the report.
  Refs: SC-038-001, NFR-038-PERF

#### Follow-up tasks

None yet.

### Verification Plan

Each task's pull request runs `integration_durable_execution` (also as the runtime role), the suites
of what it touches, `cargo clippy --workspace --all-targets --locked -- -D warnings` and the three
checkers. T038-5 to T038-8 add the fault suites of feature 002 and 010. T038-10 states the machine,
its load and the PostgreSQL configuration.
