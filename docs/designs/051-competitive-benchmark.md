# A published benchmark against Temporal and Windmill, and the hot-path remedies it needs

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

The owner's goal for release 0.3.0 is to surpass Temporal and Windmill, scale, performance and resilience included, with
evidence anyone can reproduce (Owner decision 2026-11-24-1). This design has two parts.

**Part A, the benchmark.** One harness runs the same six workflow shapes (chained activities, fan-out and fan-in,
signals, timers, sagas, scripts) on this platform, on Temporal (self-hosted, PostgreSQL persistence) and on Windmill
(self-hosted). The load is open loop: work arrives on a schedule that does not wait for the system. The three products run
in turn on the same declared machine, with the same PostgreSQL build and configuration. The harness measures throughput,
the latency of a whole workflow (median and 99th percentile), the time to recover from a crash, the work lost or done
twice, and the cost per million transitions. A report of a fixed format is published with its raw data, its
configuration and the command that reproduces it. A result where this platform loses is published as a loss.

**Part B, the hot-path remedies.** The assessment of 2026-11-11 (`docs/roadmap-assessment-2026-11-11.md`, "Remedies")
lists remedies that have no specification. Design 038 built the budget and the cheap drop of a stale candidate. Four
remain, and Part B specifies them with their own requirements and tasks: a lean queue of ready steps claimed with
`FOR UPDATE SKIP LOCKED`, waking a parked worker stream when a step becomes ready instead of polling every 500 ms,
returning the next ready step of the run in the answer to a completion, and a throughput regression gate in CI. Part B
is in this design because Part A is what shows whether each remedy pays; each remedy is built and kept only if its
measurement says so.

**Terms used below.** A *run* is one execution of a workflow; a *step* is one node of it, and an *activity* is a step
that a worker executes. A worker *polls* (or holds a *stream*) to be given work; the server answers with a *lease*, the
right to execute one step's attempt, fenced by a token so that a stale attempt is refused. A *claim* is the database
transaction that grants a lease; a *candidate* is a ready step a poll considers, and a *probe* is a claim transaction
begun for one candidate. A *completion* is the worker's report of a step's result (`CompleteStep`). A *transition* is
one completed step recorded durably (Requirement 051-008). The *run lock* is the row lock on the run that every
transaction changing the run takes first. *Design 038's budget* is the test
`tests/integration/claim-transition-efficiency/work_budget.rs` that counts the statements, transactions and rows of a
lease and of a completion and fails when they change.

## The Problem

**There is no comparison, only vendor figures.** The only competitor table in the repository
(`docs/roadmap-assessment-2026-11-11.md`, "Numbers to beat") holds vendor claims measured on other hardware with other
shapes: Temporal self-hosted 1,350 state transitions per second on a 4-core database, Windmill "up to 1,000 steps per
second per worker" with 54 ms of overhead per job. Nobody runs the three products on the same machine with the same
workflow, and nobody publishes crash-recovery times. The claim "faster than Temporal" cannot be made today, in either
direction.

**The platform's own figures are not a benchmark.** What `tests/performance/scalability-ha/` measures
(`tests/performance/reports/scalability-ha.md`):

- The transitions scenario calls the worker and run services inside one process, with no gRPC, TLS or network hop
  (`tests/support/engine.rs`, `Replica`; `tests/performance/scalability-ha/poll_client.rs`). The figures leave out the
  wire path a real worker pays.
- The load is closed loop: 3,000 runs are started first, then workers poll as fast as they can and complete serially.
  No harness of the repository has an open-loop generator, and none measures the latency of a whole workflow from start
  to end.
- Everything shares one laptop that other work also uses (load average 12 in October, 26 on 2026-11-12). The best row
  was 1,513 transitions per second on 2026-10-11 at `9092aa1`, 368 on 2026-11-11 at `ed11d044`, and 1,026 on
  2026-11-12 at `1e28a138` after design 038. The October figure is not shown to be passed or not.
- No crash-recovery time is measured by the harness. The disaster-recovery drill times a database promotion (0.2 s,
  synchronous standby, loopback, `tests/performance/reports/disaster-recovery.md`) but never points the platform at the
  promoted database, and the 15 s shard takeover in its total is a constant, not a measurement.

**The hot path still has known costs.** From design 038's research (`specs/038-claim-transition-efficiency/research.md`)
and the code:

- Probes are two thirds of all database round trips. A poll reads up to 64 candidates with no row lock
  (`crates/infra-postgres/src/workers.rs`) and opens one transaction per candidate, which takes the run row with
  `SKIP LOCKED` (`RUN_FOR_CLAIM`, `crates/infra-postgres/src/execution.rs`). On 2026-11-12 there were 58 probes per lease
  in the closed-loop scenario: a probe became cheap, so pollers spin faster.
- A lease is 38 statements and 2 transactions; a completion that makes the next step ready is 50 statements and 1
  transaction (`tests/integration/claim-transition-efficiency/work_budget.rs`, `LEASE` and
  `COMPLETION_WITH_A_NEXT_STEP`). One transition is therefore 38 + 50 = 88 statements.
- A stream that finds nothing sleeps `idle_retry` (500 ms by default, `crates/application/src/worker.rs`) and asks
  again (`crates/api-grpc/src/service.rs`); a unary poll that finds nothing returns `retry_after_ms = 500` at once. A
  step that becomes ready while every stream of its activity sleeps waits up to 500 ms before anyone asks for it. For a
  chain of three activities under light load, that is up to 3 × 500 = 1,500 ms of idle waiting in one workflow. Broker
  hints are published (`crates/infra-nats/src/lib.rs`, `NatsPublisher`) but nothing subscribes to them.
- The answer to `CompleteStep` (`StepCommandResponse`: `command_id`, `already_applied`, `step_status`, `run_status`,
  `specs/001-platform-foundation/contracts/worker.proto`) cannot carry the next task, so a worker that serves the next
  activity of the same run must poll for it.
- No CI job gates throughput. The `scale` job runs the harness at 60 transition runs with no assertion on the rate
  (`.github/workflows/ci.yml`). Design 038's budget test gates statement counts, which is what caught the fourfold
  regression of November, but not the rate.

## Requirements

### Part A: the benchmark

- **Requirement 051-001 Shapes.** The harness MUST run these six shapes, each defined once in a product-neutral
  description (`tests/performance/competitive/shapes.md`) and implemented for each product in that product's own idiom:
  1. *Chain*: three activities in sequence, each returning its input (and a variant of ten).
  2. *Fan-out*: one activity, then 100 activities in parallel, then one activity that joins their outputs.
  3. *Signal*: the workflow starts, waits for a named signal, then runs one activity. The driver sends the signal a fixed
     1 s after the start is acknowledged.
  4. *Timer*: the workflow sleeps 1 s durably, then runs one activity. A second load profile starts 100,000 workflows
     whose timers fall due within the same 10 s.
  5. *Saga*: three activities; the third fails permanently; the compensations of the first two run in reverse order.
  6. *Script*: one step that runs a short user script (TypeScript and Python variants) that returns its input.
- **Requirement 051-002 Products and versions.** The three products MUST run self-hosted from images pinned by digest:
  this platform at the commit under test; Temporal server with its PostgreSQL persistence and its SQL visibility store on
  the same PostgreSQL; Windmill community edition with its PostgreSQL. No product may use a managed or cloud service. The
  versions, digests and every configuration file are part of the report.
- **Requirement 051-003 Same machine, same database.** In one benchmark job the three products MUST run in turn on the
  same machine, against one PostgreSQL build and one configuration file, each in a fresh database. The machine MUST be
  declared (provider, instance type, vCPU, memory, disk, kernel) and MUST NOT be the shared development laptop. Each
  product gets the same CPU and memory partition (Technical Plan, "The machine").
- **Requirement 051-004 Best configuration for every product.** Each product MUST run in the best configuration the
  harness authors can find for each shape, and the pilot runs that chose it MUST be published beside the report (for
  example the number of Temporal history shards and the Windmill worker kind for each step language). A configuration
  that the product's own documentation recommends is the starting point.
- **Requirement 051-005 Open-loop load.** The driver MUST start workflows on a precomputed schedule of exponentially
  distributed inter-arrival times at a target rate, from a fixed seed, independent of completions. Latency MUST be
  measured from the scheduled start time, not from the time the request was sent, so that a stalled system cannot hide
  its queueing delay (coordinated omission).
- **Requirement 051-006 Rate ladder and saturation.** Each shape MUST run at a ladder of rates that doubles from a
  starting rate until the product saturates or the ladder reaches eight rungs. A rung is *sustained* when, over its
  measured window, completions are at least 99% of arrivals and the backlog at the end of the window is not larger than
  at its start by more than 1% of the window's arrivals. The *maximum sustained rate* is the highest sustained rung.
- **Requirement 051-007 What is measured.** For each product, shape and rung: arrivals, completions, failures,
  completions per second, transitions per second, the median and 99th percentile of workflow latency (scheduled start to
  observed completion), the timer lateness (timer shape) and the signal-to-resume latency (signal shape), CPU seconds
  and peak resident memory of the product's processes and of PostgreSQL, the database's WAL bytes and commits per
  transition, and the database connections used.
- **Requirement 051-008 One unit of work.** A *transition* in every product's figures MUST mean one completed step of
  the shape: an activity execution, a compensation or a script step whose result the product recorded durably. Product
  internal units (Temporal's state transitions, Windmill's jobs, this platform's commands) MUST NOT be compared with each
  other; they MAY be reported beside the common unit with their definition.
- **Requirement 051-009 Crash recovery.** The harness MUST inject three faults into each product while it runs the chain
  shape at 50% of its maximum sustained rate: (a) `kill -9` of one server process of the product while a second one
  keeps running; (b) `kill -9` of a worker process with activities in flight; (c) `kill -9` of the PostgreSQL primary,
  with a synchronous standby promoted by the reference topology of design 048 and the product reconnecting through the
  same address. For each it MUST measure the time from the fault to the first completed workflow, the time until the
  completion rate is back to 95% of its rate before the fault, the acknowledged workflows lost (a start the product
  acknowledged that never completes within a drain of 10 minutes), and the activity executions done more than once.
- **Requirement 051-010 Repetitions and variance.** Every measurement MUST be repeated in at least three benchmark jobs,
  each on a fresh machine of the declared type, and the report MUST show the median of the repetitions and their range.
  Within one job the three products MUST run interleaved rung by rung (Technical Plan, "Order"), so that a drift of the
  machine during a job affects them alike.
- **Requirement 051-011 Cost per million transitions.** For each product and shape the report MUST state
  `cost = price_per_minute ÷ (60 × sustained_transitions_per_second) × 1,000,000`, with the price per minute of the
  declared machine at the date of the run and its source. The figure states what the machine costs, not what a
  deployment of the product costs.
- **Requirement 051-012 Report format and reproduction.** Each campaign MUST publish a dated section of
  `tests/performance/reports/competitive-benchmark.md` in the format of Technical Plan, "The report", and its raw data
  (latency histograms, per-second counters, fault timelines, configuration files, image digests, the machine's
  description) under `tests/performance/reports/raw/competitive/<date>-<commit>/`. One command,
  `tests/performance/competitive/run.sh`, with the same inputs, MUST reproduce it on a machine of the declared type.
- **Requirement 051-013 Failures are reported.** A run that fails, a rung that did not sustain, a product that could not
  run a shape natively, and every deviation from this design MUST be stated in the report. A number that was not
  measured is never filled in.
- **Requirement 051-014 Isolation from the platform.** The harness and the competitor images MUST live only under
  `tests/performance/competitive/` and run only in the benchmark job. No production crate depends on them, and no
  competitor code is vendored into the repository.

### Part B: the hot-path remedies

- **Requirement 051-101 A lean queue of ready steps.** Every step that becomes `READY` and can be leased by a worker
  MUST have a row in a queue table, written in the same transaction that makes it ready (constitution, principle I), by
  a trigger on `steps` so that no path that makes a step ready can forget it. A claim MUST take its candidates from the
  queue with `FOR UPDATE SKIP LOCKED`, so that concurrent pollers never try the same candidate. The queue is an index of
  candidates, not an authority: a lease is still decided from the step rows read under the run lock, and a queue row
  whose step is no longer ready is deleted by the claim that finds it.
- **Requirement 051-102 The queue keeps the claim's guarantees.** The claim with the queue MUST keep, unchanged, the
  lock order of `docs/architecture/execution-semantics.md` ("Worker-session claim admission") after the queue row, one
  claim transaction per run, a lease returned only after its own commit, the order by `ready_at` for a lone poller, and
  the guards that design 038 names in its Verification Plan. The queue row lock MUST be taken only by a claim or by the
  queue's own sweeper, only with `SKIP LOCKED`, and always before the run row; no other transaction locks a queue row.
- **Requirement 051-103 The queue is complete.** A periodic reconciliation MUST find every leasable `READY` step without
  a queue row and add one, count what it added in a metric, and the integration suites MUST fail when it adds any. The
  candidate scan of today stays as that reconciliation's query.
- **Requirement 051-104 Wake on commit.** When a transaction makes a step ready, after its commit the process MUST wake
  its own parked streams and long polls that serve that step's tenant, namespace and activity; when an internal
  notification transport is configured, the outbox hint of the ready step MUST wake the parked streams of the other
  processes. A wake is a hint: a stream that wakes for nothing finds nothing, and a lost wake costs at most the
  `idle_retry` interval, which stays as the floor of correctness.
- **Requirement 051-105 The next step in the answer.** A worker session at protocol minor 1.8 or later MAY ask, in a
  `CompleteStep`, for the next ready step of the same run. When the completion makes a step ready that the session
  serves and can take (its bindings, its `max_inflight`, the activity's admission), the completion's transaction MUST
  lease it to the session under the same checks and locks as a claim, in the claim's lock order, and the answer MUST
  carry the lease. When any check refuses, the step stays `READY` in the queue and nothing else changes. A worker that
  does not ask, or an older SDK, sees no change.
- **Requirement 051-106 A throughput regression gate.** A pull request that changes the hot-path files (the selector of
  design 038, `scripts/select-test-targets.py`) MUST run a job that builds the base commit and the head commit, runs the
  transitions scenario of one configuration six times for each in alternating order on the same runner, and fails when
  the head is slower than the base in all six pairs and the median of the head is below 90% of the median of the base.
- **Requirement 051-107 Each remedy earns its place.** Each of 051-101, 051-104 and 051-105 MUST be switchable off by a
  setting in its first release, MUST be measured by Part A's chain and fan-out shapes with and without it, and MUST be
  removed if it does not improve either the maximum sustained rate or the 99th percentile latency of one of them by more
  than the range of the repetitions.

### Success criteria

- **Success criterion 051-001** A first campaign is published with all six shapes on all three products, three
  repetitions each, the crash-recovery table, and every deviation stated.
- **Success criterion 051-002** The report of every later campaign of release 0.3.0 states, for the chain shape and the
  fan-out shape, which product has the highest maximum sustained rate and which the lowest 99th percentile latency at
  50% of its own maximum sustained rate, from the medians of the repetitions. The goal is that this platform leads both
  for both shapes; the criterion is that the answer is published either way.
- **Success criterion 051-003** In the crash-recovery table, this platform loses no acknowledged workflow under any of
  the three faults, and its time back to 95% of the rate is reported for each.
- **Success criterion 051-004** With the queue of Requirement 051-101, probes per lease in design 038's barrier test of
  K pollers on one window are at most 1.1 (one probe per lease, plus at most one stale queue row in ten).
- **Success criterion 051-005** With wake on commit and one worker per activity, the median time from a step becoming
  ready to its lease, measured on the same machine in the chain shape at 10% of the maximum sustained rate, is below
  20 ms, against the up to 500 ms of today's idle sleep. With the wake, that time is one claim transaction plus the
  network; the October report measured a poll's median at 1 to 3 ms with up to 16 workers, so 20 ms leaves more than
  six times that for the wake and the network.
- **Success criterion 051-006** The regression gate fails on a hand mutation that adds a statement to every completion
  and passes on its revert, both seen in CI.

### User stories

- **User story 051-01** A buyer comparing the three products reads one page with the same shapes on the same machine and
  can rerun it.
- **User story 051-02** An operator sizing a deployment reads the latency and recovery figures for the shape that looks
  like their workload.
- **User story 051-03** A contributor whose change slows the hot path learns it on their pull request.

## Goals

- A comparison anyone can reproduce, and that a competitor's engineer would accept as fair.
- The first published crash-recovery times of the three products.
- A hot path whose cost per lease no longer grows with the number of pollers, and a step latency under light load that
  is not bounded below by a polling interval.

## Non-Goals

- **No vendor-claim comparison.** Figures from other hardware are not put next to measured ones in the report.
- **No managed services.** Temporal Cloud and Windmill's cloud are not measured; the comparison is self-hosted to
  self-hosted.
- **No tuning of the competitors' source code** and no patched builds: only their published images and settings.
- **No multi-machine benchmark in the first campaign.** A campaign on three separate machines with a network between
  them is specified (Technical Plan, "The second tier") but needs an account the owner provides; until then the report
  says the products and the database share one machine.
- **Part B does not change the journal, the transaction boundary or the worker protocol's existing calls.** It adds a
  table, a trigger, an optional field of `CompleteStepRequest` and `StepCommandResponse` in a new protocol minor, and a
  wake path. It does not add push dispatch or worker routing, which are feature 019's.

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6 and, where it accepts or rejects, of Owner
decision 2026-11-24-4. They are recorded here with their reasoning.

1. **One machine or several for the first campaign?** One. A GitHub-hosted larger runner can be started by a workflow in
   this repository with no account other than the organization's, and two jobs cannot reach each other over a network,
   so three machines would need a cloud account. One 64-vCPU runner, partitioned by CPU sets, gives every product the
   same partition, and the second tier follows when the owner provides an account. Delegate (Owner decision
   2026-11-20-6): the reproducible campaign first, the network campaign second.
2. **Which PostgreSQL major?** The one the platform's CI uses (16 today), pinned by digest, for all three products.
   Temporal's SQL persistence and Windmill both run on it. A later major is a separate campaign, not a mixed one.
   Delegate (Owner decision 2026-11-20-6): the version our tests prove is the one we publish.
3. **What load generator?** A driver of our own in Rust under `tests/performance/competitive/driver/`, with one adapter
   per product that uses the product's official client (this platform's Rust SDK, Temporal's Go SDK through a small
   sidecar the driver calls over a local socket, Windmill's REST API). Temporal's `omes` and Windmill's own benchmark
   scripts are closed loop or product-specific; one driver with one schedule is what makes the rates comparable. The
   pilot also runs `omes` once on Temporal to check that the driver does not slow Temporal down. Delegate (Owner
   decision 2026-11-20-6).
4. **Which language for activities?** Go for this platform and Temporal (both have an official Go SDK, so the worker
   language is the same); for Windmill, the step language the pilot finds fastest for a step that returns its input,
   all candidates published. The script shape uses TypeScript and Python on all three; on Temporal, which has no script
   sandbox, the script runs as an activity in a Python worker, and the report says so. Delegate (Owner decision
   2026-11-20-6): the competitor gets its best path.
5. **Is beating the competitors a release gate?** No. Publishing the honest report is the gate (Success criteria 051-001
   to 051-003); leading is the goal (051-002). Making the release wait for a lead would push the report towards the
   result. Delegate (Owner decision 2026-11-24-4).
6. **Does the lean queue need its own decision record?** Yes: it changes what a claim locks and in which order, which
   design 038 left to an architecture decision refining Architecture decisions 0030 and 0078. It is
   [Architecture decision 0133](../adr/ADR-0133-a-lean-queue-of-ready-steps-is-claimed-with-skip-locked-and-is-an-index-not-an-authority.md).
   The wake and the next step in the answer are
   [Architecture decision 0134](../adr/ADR-0134-a-ready-step-wakes-parked-streams-after-commit-and-a-completion-may-lease-the-next-step.md).

## Owner decisions needed

None that blocks the design. Two are the owner's because they spend money or need an account (Owner decision
2026-11-24-4 keeps both):

- **The runner budget of the first campaign**: a 64-vCPU GitHub-hosted larger runner for about 5,000 minutes, about
  US$ 818 at the price of 2026-11-24 (Technical Plan, "Cost of the campaigns": US$ 817.94). Gates Tasks 051-9 to
  051-12, whose `Owner-gate:` lines wait for the owner's row.
- **A cloud account for the second tier** (three machines in one zone). Gates Task 051-14.

## Critical User Journeys (CUJs)

### CUJ 1: Reading the comparison

*   **Given**: the published report of a campaign.
*   **When**: a reader opens the section of the chain shape.
*   **Then**: they see, for each product, the latency against the arrival rate at every rung, the maximum sustained rate,
    the cost per million transitions, the machine and the configuration, and a link to the raw data and the command.

### CUJ 2: Reproducing it

*   **Given**: a GitHub organization with larger runners enabled.
*   **When**: someone runs the `competitive-benchmark` workflow by hand on a fork, at a tagged commit.
*   **Then**: the job produces a dated section and raw data in the same format; the figures fall within the range the
    report published, or the report's variance section says why not.

### CUJ 3: A crash under load

*   **Given**: each product running the chain shape at half its maximum sustained rate.
*   **When**: the database primary is killed.
*   **Then**: the report shows how long each product took to complete a workflow again and to reach 95% of its rate, and
    how many acknowledged workflows were lost or activities repeated.

### CUJ 4: A slower pull request

*   **Given**: a pull request that adds a statement to every completion.
*   **When**: its CI runs.
*   **Then**: the statement budget of design 038 fails, and the throughput gate fails too if the cost shows in the rate,
    each naming what it measured.

## Technical Plan (Architecture)

### Part A

**Layout.** Everything lives under `tests/performance/competitive/`:

| Path | What it holds |
|---|---|
| `shapes.md` | The product-neutral definition of each shape: steps, inputs, outputs, what counts as complete |
| `driver/` | The Rust driver: schedule, adapters, histograms (HdrHistogram), per-second counters, fault injector |
| `sagat/`, `temporal/`, `windmill/` | For each product: the compose file with images by digest, the configuration, the workflow and activity code of each shape |
| `postgres/` | The one PostgreSQL configuration, the standby and the HA manager of design 048's development topology |
| `run.sh` | The one command: prepares the machine, runs the ladder of every shape for every product in the order below, injects the faults, writes the raw data and the report section |
| `.github/workflows/competitive-benchmark.yml` | A `workflow_dispatch` workflow only (never on a pull request or a push), with the runner label as an input |

**The machine.** The first tier is one GitHub-hosted larger runner, Linux x64, 64 vCPU, 256 GB of memory and 2,040 GB
of disk (the published specification of that runner class). The CPUs are split with CPU sets, so that each component
has its own cores whatever product runs:

| Component | vCPU | CPU set |
|---|---:|---|
| PostgreSQL primary | 16 | 0 to 15 |
| PostgreSQL synchronous standby and the HA manager | 8 | 16 to 23 |
| The product under test (its servers and its workers) | 28 | 24 to 51 |
| The driver | 8 | 52 to 59 |
| Metrics collection | 4 | 60 to 63 |

16 + 8 + 28 + 8 + 4 = 64. Only the product under test runs during its turn; the others are stopped and their databases
dropped. The standby is on during every throughput run, in synchronous mode, because a deployment that wants zero loss
of acknowledged work pays for it, and the crash fault (c) needs it.

**Order.** One benchmark job runs one shape for the three products. For each rung of the ladder it runs this platform,
Temporal, then Windmill, then the next rung, so that a slow period of the runner touches every product at the same rung.
A rung is 60 s of warm-up, 300 s measured, and up to 60 s of drain: 7 minutes at most. With eight rungs and three
products a job measures for at most 8 × 3 × 7 = 168 minutes, under the six hours a hosted job may last. Each shape runs
in three jobs on fresh runners (Requirement 051-010).

**Samples.** The starting rung is chosen so that the measured window holds at least 10,000 completions: at 50
workflows per second, 50 × 300 = 15,000. A 99th percentile from 15,000 samples has 150 samples above it.

**Saturation.** The driver stops a product's ladder at the first rung that is not sustained, and records that rung as
well, so the report shows where each product broke.

**Crash recovery.** A separate job per fault kind runs the chain shape for each product at half its maximum sustained
rate from the throughput jobs, for 5 minutes before the fault and 10 minutes after, five times. The fault injector
records the fault's time on the driver's clock; the driver's per-second completion counter gives the recovery times.
For fault (c), the database is the reference development topology of design 048: Patroni with a synchronous standby
and a virtual address the products connect to, so that all three see the same promotion.

**What each adapter does.** The adapter starts a workflow, sends a signal, and observes completion. Completion is
observed the same way for all three: the last activity of the shape writes its workflow identifier and time to a
counter service of the driver over a local socket, and the workflow's terminal status is checked afterwards through
the product's API for every workflow, so that a completion seen by the driver but not recorded by the product counts
as a failure.

**The report.** A dated section of `tests/performance/reports/competitive-benchmark.md`:

1. The environment: runner class and its published specification, kernel, PostgreSQL version and configuration, the
   CPU sets, the date, the commit of this platform, the versions and digests of the competitors.
2. One table per shape: for each product, the maximum sustained rate, the median and 99th percentile latency at 50% of
   it, CPU seconds per thousand transitions of the product and of PostgreSQL, WAL bytes per transition, cost per million
   transitions. Each cell is the median of the repetitions with the range beside it.
3. One latency-against-rate chart per shape, the three products on it.
4. The crash-recovery table: per product and fault, time to the first completion, time to 95% of the rate, acknowledged
   lost, activities repeated.
5. Variance: the coefficient of variation of each product's maximum sustained rate across the repetitions.
6. Deviations and failures (Requirement 051-013), and the pilot runs that chose each competitor's configuration.
7. How to reproduce: the command, the inputs, the runner label.

**The second tier.** Three machines of one instance type in one zone of one cloud provider: the database, the product,
and the driver, with the network round trip between them measured and stated. The same harness runs with a
`SAGAT_COMPETITIVE_TOPOLOGY=three-hosts` input. It needs an account (Owner decisions needed).

**Cost of the campaigns.** At the price published on 2026-11-24 for a 64-vCPU Linux x64 larger runner, US$ 0.162 per
minute:

| Campaign | Minutes | Cost |
|---|---:|---:|
| Throughput: 6 shapes × 3 repetitions = 18 jobs of at most 168 measured minutes and about 30 minutes of preparation | 18 × 198 = 3,564 | 3,564 × 0.162 = US$ 577.37 |
| Crash recovery: 3 faults × 3 products × 5 repetitions of 15 minutes, in 3 jobs with 30 minutes of preparation each | 3 × 3 × 5 × 15 + 3 × 30 = 765 | 765 × 0.162 = US$ 123.93 |
| Pilots: the competitors' configurations, about 4 jobs of 3 hours | 4 × 180 = 720 | 720 × 0.162 = US$ 116.64 |
| Total of one full campaign | 3,564 + 765 + 720 = 5,049 | US$ 817.94 |

The CI gate of Requirement 051-106 runs on the standard hosted runner, which costs nothing for a public repository.

**Cost per million transitions, worked through.** For a product that sustains 2,000 transitions per second on the
64-vCPU runner: one minute holds 60 × 2,000 = 120,000 transitions, so a million cost 0.162 ÷ 120,000 × 1,000,000 =
US$ 1.35. The figure is an example of the arithmetic, not a measurement.

### Part B

**The queue (Requirement 051-101, Architecture decision 0133).** A table `ready_steps` with one row per leasable ready
step: tenant, namespace, activity, `ready_at`, run, step, and an identity. Its index is (tenant, namespace, activity,
`ready_at`). A trigger on `steps` inserts the row when a step enters `READY` and is of a kind a worker leases; nothing
else inserts. The claim of one candidate becomes, in its own transaction:

1. `DELETE FROM ready_steps WHERE id = (SELECT id FROM ready_steps WHERE (tenant, namespace, activity) IN (the session's
   bindings) ORDER BY ready_at LIMIT 1 FOR UPDATE SKIP LOCKED) RETURNING run, step`.
2. The existing claim of that run from the run row on (`RUN_FOR_CLAIM` with `SKIP LOCKED`, then the drain fence, the
   session row and the activity locks, in today's order).
3. If the run row is held by another transaction, the transaction rolls back, which restores the queue row; the poll
   moves to the next queue row. If the step is no longer `READY`, the transaction commits the delete and leases nothing.

Who locks what: only claims and the queue's sweeper lock a queue row, always with `SKIP LOCKED` and before any run row;
every other transaction that changes a step takes the run row first and never touches a queue row (a cancelled or
completed step's queue row is left for the claim or the sweeper to delete). A claim that holds a queue row waits only
for locks below the run row (drain fence, session row, activity locks), whose holders never wait for a queue row, so no
cycle can form. A barrier test holds each lock of that chain in turn and asserts that no transaction deadlocks.

The queue is small (only ready steps) and has a high turnover, so it has its own autovacuum settings and a fill factor
of 70; its size is a metric. Design 049 does not partition it.

**Ordering with feature 019.** Feature 019 adds priority and fairness keys to the order of candidates. The queue's index
is the place they go: 019's build adds its key columns to `ready_steps` and to the index, and the claim's `ORDER BY`
follows. Part B does not build priority.

**The reconciliation (Requirement 051-103).** The scheduler's existing pass that finds lost leases also runs, every
30 s per shard, today's candidate scan restricted to steps without a queue row, inserts what it finds, and counts it in
`ready_queue_repaired_total`. In the integration suites, a non-zero count fails the test that caused it.

**Wake on commit (Requirement 051-104, Architecture decision 0134).** A transaction that makes steps ready returns their
(tenant, namespace, activity) keys to the application. After the commit, the application signals an in-process
registry of parked streams and long polls by key; each woken stream claims at once instead of sleeping. Across
processes, the outbox already publishes a hint per ready step when a transport is configured; a new subscriber in each
engine process maps the hint to the same registry. A key never appears in a metric label (constitution, principle V).
`idle_retry` stays the upper bound when no wake arrives; with wake on commit it can be raised (fewer empty claims), and
the setting's default is decided by Part A's measurement.

**The next step in the answer (Requirement 051-105, Architecture decision 0134).** `CompleteStepRequest` gains
`want_next` (protocol minor 1.8, additive, Architecture decision 0015). When it is set, the completion's transaction,
after the reducer has made the next steps ready and while it still holds the run row, runs the claim's checks for the
first ready step of the run whose activity the session serves: the drain fence, the session row (token, expiry,
`max_inflight`), the activity locks, the bindings and the authority. It leases the step if all pass and returns the
lease in `StepCommandResponse.next_task`. The lock order is the claim's, with the run row already held. The secrets of
the step are still fetched by `ResolveSecrets` under the new lease. The six SDKs set `want_next` when they have room,
behind a setting, and the conformance suite gains a scenario `next-task-in-completion`.

**The regression gate (Requirement 051-106).** A job `throughput-gate` in `ci.yml`, selected by the same rule as design
038's budget target. It builds the base and the head in release mode with a shared cache, starts one PostgreSQL, and runs
`step_transitions_per_second` for the configuration "1 API replica, 16 workers" with 3,000 runs, six times each,
alternating base and head. The decision: fail when the head is slower in all six pairs and the median of the head is
below 90% of the median of the base. If the two were equally fast, the chance that the head is slower in all six pairs
is (1/2)^6 = 1/64, about 1.6%, so a false failure is rare; the 90% condition keeps a real but negligible slowdown from
failing. The job prints both series.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The comparison is unfair to a competitor and is dismissed | Requirement 051-004: each competitor's best configuration found by published pilots; the configurations are in the report; the report invites corrections and a correction is a new dated section |
| A hosted runner is a shared virtual machine, so its speed varies between jobs | Three jobs on fresh runners, interleaved products within a job, the coefficient of variation published |
| One machine hides the network cost | Stated in every report of the first tier; the second tier measures it |
| A competitor's licence forbids publishing benchmarks | Temporal is MIT-licensed and Windmill's community edition AGPL-3.0; neither licence restricts publishing measurements. Task 051-1 rechecks the licences of the pinned versions and the report links them |
| The harness measures itself (as the laptop harness did) | The driver has its own cores; a pilot measures the driver alone against a no-op adapter and the report states its ceiling |
| The queue misses a ready step and a run stalls | The trigger inserts by construction; the reconciliation repairs and counts; the suites fail on a repair |
| The queue introduces a deadlock | Requirement 051-102's lock rule and the barrier test of each lock in the chain |
| A wake storm when many steps become ready at once | A wake marks a parked stream runnable; it is not a claim; one woken stream claims up to its credit |
| The next step in the answer bypasses a check | It runs the claim's checks in the claim's order, with the guards of design 038 passing unmodified, and the conformance scenario |
| The CI gate flakes | The six-of-six rule and the 90% margin; a flaky failure is investigated, not retried (`AGENTS.md`, rule 17) |
| Money is spent on a campaign whose harness is broken | The pilots run on the standard runner first at small rates; the larger runner is used only after the pilot passes |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | The queue row is written by a trigger in the transaction that makes the step ready; a lease is still decided under the run lock and returned after its commit; the next step in the answer is leased in the completion's own transaction |
| II. Modular domain, ports and adapters | Pass | The queue and the wake are in `infra-postgres` and the application's claim loop; the benchmark harness is test code under `tests/performance/` and no crate depends on it |
| III. External effects are at-least-once; sagas compensate | Pass | Nothing changes the lease grant's meaning; the crash-recovery measurement counts repeated activities instead of hiding them |
| IV. Compatibility is versioned and pinned | Pass | `want_next` and `next_task` are additive in protocol minor 1.8; the queue is an additive table; competitor images are pinned by digest |
| V. Code, tenant boundary and secrets are isolated | Pass | The next step in the answer runs the same authority and binding checks as a claim; wake keys are never metric labels; secrets are fetched under the new lease as today |
| VI. Observability and audit are part of the feature | Pass | `ready_queue_repaired_total`, queue size, wakes sent and received, next-task grants and refusals, all with closed labels only |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | The wake uses the outbox hint as a hint; a lost or duplicate wake changes no state |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash recovery is measured for every product; the report states failures; each remedy is removed if its measurement does not show a gain |
| Delivery Constraints | Pass with conditions | "Targets are not promises" holds: the report states the environment; the first tier shares one machine and says so (Complexity Tracking) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A benchmark harness for three products | A claim of surpassing needs the same shapes on the same machine | Comparing vendor figures: different hardware, shapes and units |
| A Temporal Go sidecar for the driver | The driver is Rust and Temporal has no official Rust SDK | Using `omes` as the driver for Temporal only: a different load model per product |
| One machine split by CPU sets in the first tier | Reproducible by anyone with a GitHub organization, no cloud account | Waiting for three machines: no report in 0.3.0 without an account |
| The `ready_steps` table and its trigger | Probes are two thirds of the round trips; a queue claimed with `SKIP LOCKED` gives one probe per lease | Design 038's window order: per process, so replicas still collide (`LWLock:LockManager` 4% to 11% with four and eight replicas) |
| A wake path | Up to 500 ms of idle per step under light load | A shorter `idle_retry`: more empty claims, each a database round trip, for every idle stream |
| A protocol field for the next step | A worker that serves the next activity polls for it after every completion | Relying on wake on commit alone: still a poll round trip and a claim transaction per step |

## Alternatives Considered

- **Use each vendor's benchmark tool.** `omes` for Temporal and Windmill's own scripts measure different things in
  different loops; the rates would not be comparable. One driver, with `omes` as a cross-check for Temporal.
- **Run on the development laptop.** Not valid evidence: shared, closed loop, other work running (the October and
  November load averages).
- **Measure closed loop and report the peak.** Hides queueing delay; a system that stalls looks as fast as one that
  does not. Open loop with latency from the scheduled start.
- **Compare only throughput.** A product that loses or repeats work under a crash can look faster. Crash recovery and
  integrity are part of every report.
- **A queue table that is the authority** (the step's state lives in the queue, as Windmill's queue holds its jobs).
  One statement per claim, but the run lock and the reducer would no longer decide a lease, which changes principle I's
  boundary and every guard of features 002, 009 and 010. The queue as an index keeps all of them.
- **`LISTEN` and `NOTIFY` for the wake.** A transaction that issues `NOTIFY` takes a lock on the notification queue at
  commit, which serializes the commits of every such transaction in the cluster; on the hot path that is the
  bottleneck this design removes. The in-process registry plus the existing outbox hint does not touch the commit.
- **Eager dispatch of every ready step to any session.** Feature 019's push dispatch and routing are that design; Part B
  only returns a step of the same run to the session that just completed one.

## Non-Functional Requirements

- **Non-functional requirement 051-001 Reproducibility.** Two campaigns at the same commit on the same runner class give
  maximum sustained rates within the range their repetitions report, or the report explains the difference.
- **Non-functional requirement 051-002 Cost.** One full campaign stays under 5,500 runner minutes (5,049 planned, with
  about 9% of margin: 5,049 × 1.09 = 5,503.41, rounded to 5,500).
- **Non-functional requirement 051-003 Hot path.** With Part B on, a lease and a completion each stay within the
  statement budget of design 038 plus the statements Part B adds, recorded in `work_budget.rs` in the change that adds
  them: one `DELETE ... RETURNING` per lease and one trigger insert per ready step.
- **Non-functional requirement 051-004 Security.** The benchmark job holds no secret of the repository and runs only by
  hand; the competitor images run without network access beyond the job's loopback.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Part A tasks are 051-1 to 051-14; Part B tasks are 051-20 to 051-29.

- [ ] T051-1 Write `tests/performance/competitive/shapes.md`, check the licences of the pinned competitor versions, and pin their images by digest.
  Refs: Requirement 051-001, Requirement 051-002
- [ ] T051-2 Build the driver: open-loop schedule from a seed, latency from the scheduled start, HdrHistogram output, per-second counters, and a no-op adapter that measures the driver's own ceiling.
  Refs: Requirement 051-005, Requirement 051-007
- [ ] T051-3 Write this platform's adapter and the six shapes as workflow versions with Go activities.
  Refs: Requirement 051-001, Requirement 051-008
- [ ] T051-4 Write Temporal's compose file (server, PostgreSQL persistence and SQL visibility), its Go worker and sidecar, and the six shapes.
  Refs: Requirement 051-001, Requirement 051-002
- [ ] T051-5 Write Windmill's compose file, its flows for the six shapes, and the step-language pilot.
  Refs: Requirement 051-001, Requirement 051-004
- [ ] T051-6 Write `run.sh`: CPU sets, the PostgreSQL configuration, the interleaved ladder, saturation, raw data and the report section.
  Refs: Requirement 051-003, Requirement 051-006, Requirement 051-010, Requirement 051-012
- [ ] T051-7 Add the fault injector and the crash-recovery jobs on the development topology of design 048.
  Refs: Requirement 051-009
- [ ] T051-8 Add the `competitive-benchmark` workflow (manual dispatch only) and run a pilot on the standard runner at small rates.
  Refs: Requirement 051-012, Requirement 051-014
- [ ] T051-9 Run the pilots that choose each competitor's configuration on the larger runner and publish them.
  Refs: Requirement 051-004
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-10 Run the first full campaign and publish the dated report section and raw data.
  Refs: Success criterion 051-001, Requirement 051-011, Requirement 051-013
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-11 Publish the crash-recovery table of the first campaign.
  Refs: Success criterion 051-003
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-12 Run the campaign again with Part B on and off, and record each remedy's gain or its removal.
  Refs: Requirement 051-107, Success criterion 051-002
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-13 Link the report from `docs/README.md` and the roadmap, and write the reader's guide to it.
  Refs: User story 051-01, User story 051-02
- [ ] T051-14 Run the second tier on three machines and publish it.
  Refs: Requirement 051-003
  Owner-gate: pending, the owner's row providing the cloud account of "Owner decisions needed"

- [ ] T051-20 Write Architecture decisions 0133 and 0134 and update `docs/architecture/execution-semantics.md` with the queue row in the claim's lock order.
  Refs: Requirement 051-101, Requirement 051-102, Requirement 051-104, Requirement 051-105
- [ ] T051-21 Add the `ready_steps` table, its trigger, grants and storage classification, with the migration tests.
  Refs: Requirement 051-101
- [ ] T051-22 Claim from the queue with `SKIP LOCKED`; the barrier tests of the lock chain; design 038's guards unmodified; the budget moved.
  Refs: Requirement 051-101, Requirement 051-102, Success criterion 051-004
- [ ] T051-23 Add the reconciliation, its metric, and the suite check that fails on a repair.
  Refs: Requirement 051-103
- [ ] T051-24 Wake parked streams and long polls after commit in process, with its tests and metrics.
  Refs: Requirement 051-104, Success criterion 051-005
- [ ] T051-25 Subscribe each engine process to the ready-step hints of the configured transport and wake by key.
  Refs: Requirement 051-104
- [ ] T051-26 Add `want_next` and `next_task` in protocol minor 1.8, lease the next step in the completion, and the conformance scenario.
  Refs: Requirement 051-105
- [ ] T051-27 Set `want_next` in the six SDKs behind a setting, with each SDK's tests.
  Refs: Requirement 051-105
- [ ] T051-28 Add the `throughput-gate` job and prove it fails on a hand mutation and passes on its revert.
  Refs: Requirement 051-106, Success criterion 051-006
- [ ] T051-29 Document the settings of the queue, the wake and the next step in `docs/runbooks/scaling.md`.
  Refs: Requirement 051-107

#### Follow-up tasks

None yet.

### Verification Plan

Part A is verified by its own output: the pilot on the standard runner (Task 051-8) must produce a report section in
the format of Requirement 051-012 with every shape completing on every product, before money is spent. The no-op
adapter's ceiling must be at least ten times the highest sustained rate any product reaches, or the driver is the
bottleneck and the campaign is stopped.

Part B: each pull request runs `integration_durable_execution` (also as the runtime role), design 038's budget target
`integration_claim_transition_efficiency`, the fault suites of features 002 and 010, `cargo clippy --workspace
--all-targets --locked -- -D warnings` and the three checkers. Tasks 051-22 and 051-26 also run the SDK conformance
scenarios `signals-and-events` and `next-task-in-completion`. These guards pass unmodified: the seven tests that design
038 lists in its Verification Plan. Task 051-22 adds a barrier test per lock of the claim's chain while a queue row is
held, and a test that a cancelled step's queue row is deleted by the next claim and never leased. Each test is seen
failing first, with a hand mutation where the behaviour already exists.
