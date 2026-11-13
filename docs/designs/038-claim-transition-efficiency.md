# Claim and transition efficiency

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-12

--------------------------------------------------------------------------------

## Overview

Make a step transition cost the database what it needs and no more: cut the work a worker's claim and
report do in PostgreSQL, and put a budget on it that a test enforces. No guarantee of feature 002 or 010
changes, no lock is removed or reordered, and the claim stays one transaction per run.

This is the second draft. The first was reviewed by the architecture, security and correctness reviewers
on 2026-11-12 (8 Blocker and 21 Major findings between them, on the text; nothing of it was built). They
are answered here: three of the first draft's steps are out (see Non-Goals), and what remains only
changes which candidates a poll tries and how cheaply it drops one.

## The Problem

The transitions scenario of feature 010 (`tests/performance/scalability-ha/run.sh`, three chained
activities per run) reached 1,513 transitions per second on 2026-10-11 at `9092aa1`. On 2026-11-11, at
`develop` `ed11d044` on the same machine, it reached 368. Pull request 203 removed two advisory-lock
mutexes (the namespace secrets lock in the worker's authority check and the per-activity admission
lock) and the figure rose to 763, with the machine busier than in October. It is still half. What
changed between the two commits is not attributed yet; the claim path itself did not (G246).

**Without concurrency** (`work_budget.rs`, pull request 205, now in `tests/integration/claim-transition-efficiency/`, on the
tree of pull request 203): a poll that leases one step is 46 statements in 2 transactions and writes 6
rows; a completion that makes the next step ready is 58 statements and 12 rows; the one that ends the
run 55 and 10. A transition is 104 statements. Among them, for a workflow that has none of these, five
projection loads per command (sagas, scope sagas, scopes, collections, loops), and one
`UPDATE steps SET node_kind` that rewrites the value the row already holds.

**With concurrency**, a run of the scenario with `pg_stat_statements` on (pull request 203 applied;
1,000 runs per configuration, nine configurations, 27,000 leases):

| What | Count | Per lease |
|---|---:|---:|
| Polls | 15,319 | 0.57 |
| Candidates returned by the scans | 910,716 | 33.7 |
| Candidates probed (a transaction, the chain lock, the run row with `SKIP LOCKED`) | 904,750 | 33.5 |
| Probes that got the run lock and loaded the whole run | 203,567 | 7.5 |
| Leases granted | 27,000 | 1 |

Every poller of an activity reads the same oldest candidates, up to 64, and tries each in turn: 77.5% of
the probes find the run locked by another poller and 19.5% load a run that has nothing left to lease.
A poll also keeps walking its window after the session can take nothing more (no room, drained or
gone): each of those costs a full run load.

Part of the ratio is the benchmark's shape: up to 128 workers ask for 16 tasks each from 1,000 runs with
one ready step, so demand exceeds supply and a poll leases 1.76 tasks on average. The targets below are
stated for a backlog at least as large as the demand.

With four and eight API replicas the backends wait on `Lock:transactionid` (up to 29%),
`LWLock:BufferContent` (21%) and `LWLock:MultiXactGen` (7%). The rows behind them are not identified.
Candidates the code offers: the sixteen `run_active_counts` slot rows of a namespace, the one
`tenant_retained_usage` row of a tenant (locked at commit by the deferred quota flush), the worker-session
row, and the membership row every report locks `FOR SHARE`.

The counts above do not depend on the machine's load; the rates do, and are stated as such.

## Requirements

- **FR-038-001** A test MUST fail when the statements, the transactions or the rows written by a lease
  or by a completion of a fixed workflow shape, run by one worker with nothing racing, differ from a
  recorded budget. The budget MUST change in the change that moves it. Three kinds of statement stay
  separate whatever the budget: the clock read after the run lock, the clock read after the session
  lock, and a count read after the advisory lock that guards it.
- **FR-038-002** A probed candidate that has no step the session can lease MUST cost at most one
  statement after the run lock is granted, and no projection load.
- **FR-038-003** A poll MUST stop at the first candidate that proves the session can take nothing more.
- **FR-038-004** Concurrent pollers MUST NOT all try the fetched window in the same order. The order a
  poll tries it in is a preference derived from a value the server holds and the client does not choose
  per request; it never removes a candidate. A lone poller MUST be served in `ready_at`
  order, and every poll MUST still have the oldest ready step of each of its activities in its window.
- **FR-038-005** A command MUST NOT run a projection load for a kind of state its run cannot have. The
  decision MUST come from a fact that is true by construction and checked by a test; when it is unknown
  everything is loaded. It MUST NOT skip a lock, the chain lookup, an authority, quota or capacity read,
  the protocol check or the secrets of a step.
- **FR-038-006** A transition MUST NOT rewrite a step's `node_kind` when the kind read under the run
  lock already equals the definition's. A `legacyControl` row MUST still be upgraded once, and any other
  difference, or a missing row, MUST still be refused as corruption, on the root and on the scoped path.
- **FR-038-007** The claim path MUST count candidates fetched, candidates probed, runs loaded and leases
  granted, as metrics whose only labels are closed enumerations (no tenant, namespace, activity,
  session, worker, pool or run).
- **FR-038-008** The budget target MUST run on a pull request that changes the hot path, not only on one
  that changes the test.
- **SC-038-001** With a backlog at least as large as the demand, runs loaded per lease fall from 7.5 to
  at most 1.5 and candidates probed per lease from 33.5 to at most 4, measured by FR-038-007's counters
  in a test that holds K pollers at a barrier on one window.
- **SC-038-002** On a machine whose load average is at most the October report's, with its PostgreSQL
  version and configuration, the row "1 API replica, 16 workers" of the transitions table is at least
  1,513 transitions per second, and the row "8 replicas, 128 workers" is not below the row "1 replica,
  128 workers".
- **NFR-038-SEC** This design adds no grant, role membership or `SECURITY DEFINER` function to the
  runtime role. `pg_stat_statements` and `pg_locks` are read only by the owner role in a disposable test
  database; production metrics come from application counters.

## Goals

- A regression of the hot path fails a test on the pull request that causes it.
- A poll's cost follows what it leases, not the size of its window.
- The October figure is passed on the same machine, with the reason for each gain stated.

## Non-Goals

- No change of the journal, of the transaction boundary (one claim transaction per run, a lease
  returned only after its own commit) or of the lock order of `docs/architecture/execution-semantics.md`.
  No row lock in the candidate scan, and never more than one run per claim transaction: either would
  need an ADR refining ADR-0030 and ADR-0078.
- **The authority and session checks are not changed.** The membership row lock is what a revocation
  typed as SQL waits for, and the session row lock held to commit is the only claim-time fence of a root
  claim and what keeps `max_inflight` exact. If T038-2 names them as the contended rows, that is a
  design of its own, with an ADR and the security review first.
- **The start of a run is not changed.** Sharing the active-run admission lock diverges from ADR-0020
  decision 9, and its margin needs a bound the database enforces on concurrent starters. A design of
  its own.
- No second database, cell or partitioning; no push dispatch or hint subscription (feature 019); no
  change of the worker protocol.

## Open Questions

- What the contended rows are with eight replicas (T038-2). The answer may make a follow-up design the
  larger gain.
- Whether statement counts are the same on PostgreSQL 16, which CI uses, as on 18.6, where the budget
  was recorded.

## Owner decisions needed

Both answered on 2026-11-12 (OD-2026-11-12-3 and OD-2026-11-12-4, `docs/owner-decisions.md`), as recommended:

- Whether a poll may try its window in an order other than oldest-first, within FR-038-004's limits.
  `tests/contract/sdk-conformance/scenarios/signals-and-events.json` says a worker is leased its tasks
  in the order they became ready, which FR-038-004 keeps for a lone poller only. No requirement of
  feature 002 or 010 promises more. Gates T038-8.
- Whether the budget target may run on pull requests that change the hot-path files (FR-038-008). It
  widens what the 2026-11-04 rule runs on a pull request (a suite runs only when its own files change),
  by one target of about a second. Gates T038-10.

## Critical User Journeys (CUJs)

### CUJ 1: Many workers on one activity

*   **Given**: 128 workers polling one activity through eight API replicas, and a backlog larger than
    what they ask for.
*   **When**: they poll at once.
*   **Then**: they try the window in different orders, a stale candidate costs one statement, and
    almost every run that is loaded grants a lease.

### CUJ 2: A change that makes the hot path heavier

*   **Given**: a change in `crates/infra-postgres` that adds a statement to every completion.
*   **When**: its pull request runs.
*   **Then**: the budget test runs, fails and lists the statements of the completion, and the author
    removes the statement or moves the budget with the reason in the same change.

### CUJ 3: A worker at capacity

*   **Given**: a session whose `max_inflight` is reached.
*   **When**: it polls.
*   **Then**: the poll ends after the first candidate that shows there is no room, not after the window.

## Technical Plan (Architecture)

Statements stay in `crates/infra-postgres`, the claim loop in `crates/application`, the tests in
`tests/integration/durable-execution`. The claim's lock sequence is unchanged; the design writes it down
where it adds a statement (FR-038-002), as ADR-0078's open item asks.

1. **Budget** (built, pull request 205). One worker, one run at a time, equality on top-level statement
   counts, transactions and rows written, with the statements listed on failure. To add: a reading of
   `pg_stat_statements_info.dealloc` around each window (an evicted entry restarts its count), rows
   written by `WITH … UPDATE` statements, and a line for a poll at capacity.
2. **Attribute and locate.** Run the budget test's shape at `9092aa1` and list the statements of a lease
   and of a completion that were added since. Sample `pg_locks` joined to `pg_stat_activity`, with
   relation and tuple, in the eight-replica configuration and name the rows. Both go to
   `specs/038-claim-transition-efficiency/research.md`.
3. **Counters** (FR-038-007), before the herd is touched, because its test reads them.
4. **Herd test.** K sessions fetch the same window and are held at a barrier before they claim; the test
   asserts the counters' worst case, first at today's values.
5. **Stop at capacity** (FR-038-003). The claim tells "the run is locked" from "the session can take
   nothing more"; the poll ends on the second.
6. **Cheap drop** (FR-038-002). Built as the claim's run read (`RunStore::begin_claim`): the row is returned only with a
   ready root step of an activity the session serves, so a run with nothing for the session is dropped without loading.
   A scoped candidate is still opened to find out. The plan was: After the run lock and before the load, one indexed `EXISTS` for a ready
   step of the session's activities in that run. The step rows are read again under the lock as today.
7. **`node_kind`** (FR-038-006). The kind is already read with the step rows (`stored_node_kind`); the
   write is skipped when it equals the definition's, in `ensure_node_kind` and `ensure_path_node_kind`.
8. **Projection loads** (FR-038-005). One domain predicate per kind, from the pinned definition where
   the kind follows from it, and a property test that no row of a kind exists for a run the predicate
   excludes, operator commands, cancellation and timeouts included. A kind whose rows can come from
   elsewhere (a parent, a legacy row, a continuation policy) is always loaded.
9. **Window order** (FR-038-004). The application tries the fetched window from an offset, wrapping, so
   no candidate is removed; the scan and its `ready_at` order are unchanged. The offset is the poll's turn
   among the polls of its namespace in flight in the process, times what each asks for: a poll alone has
   turn 0 and keeps the oldest-first order, which an offset from the session's id would not (it would
   start a lone worker anywhere). The turn is per process, so polls in different API replicas can still
   collide. Fetching again after k consecutive skipped candidates is not built.
10. **Selector rule** (FR-038-008) in `scripts/select-test-targets.py`, with a negative fixture. The budget tests move to a
    target of their own, `integration_claim_transition_efficiency`, so that what such a pull request runs is the budget and
    not a five-minute suite; the rule needs constitution 4.4.0 (CI gates).

Each step is its own pull request and moves the budget or the counters' bound it earns.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The cheap drop reads before the load and acts on a stale answer | It only decides to skip; a lease is still decided from the rows read under the run lock. A step that becomes ready after the `EXISTS` is found by the next poll, as a step that becomes ready after today's load is |
| An offset starves the oldest step | It is an order of trying, not a filter: a poll with room reaches every candidate of its window. Tests: a lone poller in `ready_at` order; two pollers at a barrier each lease what they asked from a backlog of twice that; the oldest step leased within one poll that had room |
| A skipped load misses state | Decided by a predicate with a property test; unknown means load; `verify_run` in the suites is a second, late detector, not the proof |
| The budget flakes | Counts of one worker with nothing racing; `dealloc` read around each window; only top-level statements, so the server's `track` setting does not matter |
| A guard test is edited to make a change pass | The guards named in the Verification Plan pass unmodified |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | One claim transaction per run, a lease returned after its own commit, the lock order unchanged (Non-Goals) |
| II. Modular domain, ports and adapters | Pass | The predicates of step 8 are domain functions of the definition; statements stay in `infra-postgres` |
| III. External effects are at-least-once; sagas compensate | Pass | The lease grant is unchanged; a compensation is claimed by the same path |
| IV. Compatibility is versioned and pinned | Pass | No schema, protocol or lock change, so old and new binaries run together |
| V. Code, tenant boundary and secrets are isolated | Pass | The authority and session checks are untouched (Non-Goals); NFR-038-SEC |
| VI. Observability and audit are part of the feature | Pass | FR-038-007 |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Each test is seen failing first (a hand mutation where the budget starts at today's value); races with barriers |
| Delivery Constraints | Pass | Single region, one primary |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A statement after the run lock (step 6) | 19.5% of the probes load a whole run to find nothing | Loading and finding out: the measured waste |
| A try order per session (step 9) | 77.5% of the probes find the run locked by a poller trying the same order | The same order for all: the herd |
| A CI selector rule (step 10) | A budget that runs only when its own file changes missed a fourfold regression | Running it at release only: how it was missed |

## Alternatives Considered

- **Row locks in the candidate scan** (`FOR UPDATE … SKIP LOCKED`). It locks rows the outer limit
  discards, takes run locks before the chain lock, and lets a poller that cannot lease hide the head of
  the queue from one that can. Out, with the ADRs it would need.
- **Several runs per claim transaction.** One failing run would roll back the others' leases, and
  activity locks sorted per run would deadlock across runs.
- **A partition of the window by session hash.** A step hashed to a session that is full or dead is
  returned to nobody. An order of trying has no such case.
- **A separate task queue table claimed with one statement**, as Windmill does. The larger redesign; kept
  for after the counters show what the present path can do.
- **More connections or replicas.** The profile shows more replicas lower the throughput.

## Non-Functional Requirements

- **NFR-038-PERF** SC-038-001 by test; SC-038-002 by the benchmark, with the machine, its load and the
  PostgreSQL configuration stated, appended to the report as a dated section beside October's.
- **NFR-038-SEC** as above.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T038-1 Budget the work of a lease and of a completion, one worker, equality, statements listed on failure.
  Refs: FR-038-001
- [ ] T038-2 Attribute the regression since `9092aa1` and name the contended rows with eight replicas.
  Refs: SC-038-002
- [ ] T038-3 Count candidates fetched, probed, runs loaded and leases, with the telemetry test.
  Refs: FR-038-007
- [ ] T038-4 Add the barrier test of K pollers on one window, at today's counters.
  Refs: SC-038-001
- [ ] T038-5 End a poll at the first candidate that proves the session can take nothing more.
  Refs: FR-038-003
- [ ] T038-6 Drop a candidate with nothing to lease for one statement after the run lock.
  Refs: FR-038-002, SC-038-001
- [ ] T038-7 Skip the `node_kind` write when the kind read already matches, on both paths.
  Refs: FR-038-006
- [ ] T038-8 Try the window from a per-session offset and fetch again after k skips.
  Refs: FR-038-004, SC-038-001
  Owner-gate: OD-2026-11-12-3
- [ ] T038-9 Skip projection loads by a domain predicate with its property test.
  Refs: FR-038-005
- [ ] T038-10 Select the budget target on pull requests that change the hot-path files.
  Refs: FR-038-008
  Owner-gate: OD-2026-11-12-4
- [ ] T038-11 Run the transitions scenario on a quiet machine and append the dated section to the report.
  Refs: SC-038-002, NFR-038-PERF

#### Follow-up tasks

None yet.

### Verification Plan

Each pull request runs `integration_durable_execution` (also as the runtime role), the suites of what it
touches, `cargo clippy --workspace --all-targets --locked -- -D warnings` and the three checkers; T038-5,
T038-6 and T038-8 add the fault suites of features 002 and 010 and the SDK conformance scenario
`signals-and-events`. These guards pass unmodified: in `concurrency.rs`,
`session_replacement_revalidates_the_token_and_lowered_limit_at_the_claim_fence`,
`a_worker_never_holds_more_tasks_than_it_declared`,
`a_capped_backlog_cannot_hide_another_activity_past_the_candidate_limit`,
`concurrent_pollers_never_overshoot_an_activity_quota`; in `hardening.rs`,
`a_run_that_cannot_be_leased_does_not_keep_the_others_from_a_worker`; in `activity_locking.rs`,
`a_revocation_waits_for_a_report_that_checked_and_stops_the_next_one` and
`opposite_node_activity_orders_do_not_abort_a_concurrent_claim`. T038-11 states the machine, its load and
the PostgreSQL configuration.
