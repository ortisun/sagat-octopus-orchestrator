# A published benchmark against Temporal and Windmill

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

The owner's aim for release 0.3.0 is that the platform surpass Temporal and Windmill, and the owner asked for a
published benchmark of the same workflow shapes on the three products, crash-recovery time included (Owner decision
2026-11-24-1, pull request 273, whose record quotes the owner: "so that we surpass Windmill and Temporal"). This design
is that benchmark.

One harness runs the same nine shape variants (chained activities, fan-out and fan-in, signals, timers, sagas, scripts)
on this platform, on Temporal (self-hosted, PostgreSQL persistence) and on Windmill (self-hosted). The load is open
loop: work arrives on a schedule that does not wait for the system. The three products run in turn, in rotating order,
on the same declared machine, with the same PostgreSQL build, each against a database that already holds a history.
The harness measures throughput, the latency of a whole workflow as each product reports its end (median and 99th
percentile), the time to recover from three kinds of crash, the work lost, repeated or sent to review, and the cost per
million transitions. A report of a fixed format is published with its raw data, its configuration and the command that
reproduces it. A result where this platform loses is published as a loss.

The hot-path remedies that the assessment of 2026-11-11 left without a specification (a lean queue of ready steps,
waking parked streams on commit, the next step in a completion's answer, a throughput regression gate in CI) were the
second part of this design's first draft. Its review showed they need their own design round; they are design 052,
whose gains this benchmark measures.

**Terms used below.** A *run* is one execution of a workflow; a *step* is one node of it, and an *activity* is a step
that a worker executes. A *transition* is one completed step recorded durably (Requirement 051-008). A *shape* is a
workflow defined once in product-neutral terms and implemented for each product. A *rung* is one target arrival rate of
the load, measured over a fixed window. *Open loop* means that starts are sent at their scheduled times whatever the
system does; *coordinated omission* is the error of measuring latency from when a request was sent instead of when it
was due, which hides the queueing of a stalled system.

## The Problem

**There is no comparison, only vendor figures.** The only competitor table in the repository
(`docs/roadmap-assessment-2026-11-11.md`, "Numbers to beat") holds vendor claims measured on other hardware with other
shapes: Temporal self-hosted 1,350 state transitions per second on a 4-core database, Windmill "up to 1,000 steps per
second per worker" with 54 ms of overhead per job. Nobody runs the three products on the same machine with the same
workflows, and nobody publishes crash-recovery times. The claim "faster than Temporal" cannot be made today, in either
direction.

**The platform's own figures are not a benchmark.** What `tests/performance/scalability-ha/` measures
(`tests/performance/reports/scalability-ha.md`):

- The transitions scenario calls the worker and run services inside one process, with no gRPC, TLS or network hop
  (`tests/support/engine.rs`, `Replica`; `tests/performance/scalability-ha/poll_client.rs`). The figures leave out the
  wire path a real worker pays.
- The load is closed loop: 3,000 runs are started first, then workers poll as fast as they can and complete serially.
  No harness of the repository has an open-loop generator, and none measures the latency of a whole workflow.
- Everything shares one laptop that other work also uses (load average 12 in October, 26 on 2026-11-12). The best row
  was 1,513 transitions per second on 2026-10-11 at `9092aa1`, 368 on 2026-11-11 at `ed11d044`, and 1,026 on
  2026-11-12 at `1e28a138` after design 038.
- No crash-recovery time is measured. The disaster-recovery drill times a database promotion (0.2 s, synchronous
  standby, loopback, `tests/performance/reports/disaster-recovery.md`) but never points the platform at the promoted
  database, and the 15 s shard takeover in its total is a constant, not a measurement.

## Requirements

- **Requirement 051-001 Shapes.** The harness MUST run these nine variants, each defined once in a product-neutral
  description (`tests/performance/competitive/shapes.md`) and implemented for each product in that product's own idiom:
  1. *Chain of three*: three activities in sequence, each returning its input; and *chain of ten*.
  2. *Fan-out*: one activity, then 100 activities in parallel, then one activity that joins their outputs.
  3. *Signal*: the workflow starts, waits for a named signal, then runs one activity; the driver sends the signal 1 s
     after the start is acknowledged.
  4. *Timer*: the workflow sleeps 1 s durably, then runs one activity; and *timer burst*: 100,000 workflows whose 1 s
     timers fall due within the same 10 s.
  5. *Saga*: three activities; the third fails permanently; the compensations of the first two run in reverse order.
  6. *Script*: one step that runs a short user script that returns its input, in a TypeScript and a Python variant.
- **Requirement 051-002 Products and versions.** The three products MUST run self-hosted from images pinned by digest:
  this platform at the commit under test; Temporal server with its PostgreSQL persistence; Windmill community edition
  with its PostgreSQL. No managed or cloud service. The versions, digests and every configuration file are part of the
  report.
- **Requirement 051-003 Same machine, same database build.** In one benchmark job the three products MUST run in turn
  on the same machine and the same PostgreSQL build, each in its own database, pre-populated before its first rung with
  the history of Requirement 051-015 and kept across the job's rungs. The machine MUST be declared (provider, runner
  class, CPU model, vCPU, memory, disk, kernel) and MUST NOT be the shared development laptop. Each product gets the same
  CPU set and memory limit (Technical Plan, "The machine").
- **Requirement 051-004 Best configuration within fixed constraints.** Each product MUST run in the best configuration
  the harness authors can find for each shape, chosen by pilot runs that are published beside the report, within the
  fixed constraints of Technical Plan, "Fixed constraints"; each constraint that departs from a product's documented
  recommendation is a deviation line of the report.
- **Requirement 051-005 Open-loop load.** The driver MUST start workflows on a precomputed schedule of exponentially
  distributed inter-arrival times at a target rate, from a fixed seed, independent of completions. Latency MUST be
  measured from the scheduled start time. An *arrival* is a scheduled start; a start the product refuses is retried by
  the driver with the same idempotency key under the product's documented retry advice for at most 10 s, and an arrival
  still refused then is a failed arrival.
- **Requirement 051-006 Rungs and the maximum sustained rate.** Each variant MUST start at a rate chosen per variant by
  the pilots and double until the first rung that is not sustained or eight rungs, then bisect three times between the
  last sustained and the first unsustained rung, so that the maximum sustained rate is known to within one eighth of the
  last doubling. A rung is *sustained* when, over its window, at least 99% of its arrivals completed, the backlog at the
  end is not larger than at the start by more than 1% of the window's arrivals, and no more than 1% of its workflows are
  still open at the end of the drain. If the first rung is not sustained, the rate halves until one is. The latency at
  50% of the maximum sustained rate is measured as a rung of its own.
- **Requirement 051-007 What is measured.** For each product, variant and rung: arrivals, completions, failures,
  completions and transitions per second, the median and 99th percentile of workflow latency, the timer lateness (timer
  variants) and the signal-to-resume latency (signal shape), CPU seconds and peak memory of the product's processes and
  of PostgreSQL, the WAL bytes and commits per transition, and the database connections used. A workflow still open at
  the end of the drain is counted in the latency at the drain's bound and named in the report.
- **Requirement 051-008 One unit of work, observed by the product.** A *transition* in every product's figures MUST mean
  one completed step of the shape: an activity execution, a compensation or a script step whose result the product
  recorded durably. A workflow's end MUST be observed through the product's own result API (this platform's run result,
  Temporal's workflow result, Windmill's job result), on the driver's clock. Every activity MUST also record its
  workflow, step and attempt with the driver's counter, so that repeated executions are counted. Product internal units
  (Temporal's state transitions, Windmill's jobs) MUST NOT be compared; they MAY be reported beside the common unit.
- **Requirement 051-009 Crash recovery.** The harness MUST inject three faults into each product while it runs the chain
  of three at 50% of its maximum sustained rate: (a) `kill -9` of one server process of the product while another keeps
  running (Technical Plan, "Faults per product"); (b) `kill -9` of a worker process with activities in flight; (c)
  `kill -9` of the PostgreSQL primary of design 048's development topology, with a synchronous standby promoted and the
  product reconnecting through the same address. For each it MUST measure: the time from the fault to the first
  workflow completed after it; the time until the backlog and the 99th percentile latency of new starts, smoothed over
  10 s, are back to their level before the fault; acknowledged workflows lost (a start the product acknowledged that has
  not completed after a 10-minute drain); activity executions repeated; and work sent to a person (on this platform, a
  step in review). The timeouts that decide recovery (activity heartbeat and start-to-close) MUST be the same for the
  three products and are fixed in `shapes.md`.
- **Requirement 051-010 Repetitions and order.** Every measurement MUST be repeated in at least three benchmark jobs on
  fresh machines of the declared class, and the report MUST show the median of the repetitions and their range. Within
  a job the products run rung by rung, in an order that rotates by rung and by repetition (each product first, second
  and third equally often), so that a drift of the machine does not favour one product.
- **Requirement 051-011 Cost per million transitions.** For each product and variant the report MUST state
  `cost = price_per_minute ÷ (60 × sustained_transitions_per_second) × 1,000,000`, with the price per minute of the
  declared machine at the date of the run and its source. The figure states what the machine costs, not what a
  deployment of the product costs.
- **Requirement 051-012 Report format and reproduction.** Each campaign MUST publish a dated section of
  `tests/performance/reports/competitive-benchmark.md` in the format of Technical Plan, "The report", and its raw data
  under `tests/performance/reports/raw/competitive/<date>-<commit>/`. One command, `tests/performance/competitive/run.sh`,
  with the same inputs, MUST reproduce it on a machine of the declared class. The method and the configurations MUST be
  published in the repository before the first paid run.
- **Requirement 051-013 Failures are reported.** A run that fails, a rung that did not sustain, a product that could
  not run a shape natively, and every deviation from this design MUST be stated in the report. A number that was not
  measured is never filled in.
- **Requirement 051-014 Isolation from the platform.** The harness and the competitor images MUST live only under
  `tests/performance/competitive/`, outside the Cargo workspace, and run only in the benchmark job. No production crate
  depends on them, and no competitor code is vendored.
- **Requirement 051-015 A history before the measurement.** Before its first rung, each product's database MUST hold
  1,000,000 completed chains of three, loaded through the product itself, so that the measurement is not of empty
  tables.

### Success criteria

- **Success criterion 051-001** A first campaign is published with all nine variants on all three products, three
  repetitions each, the crash-recovery table, and every deviation stated.
- **Success criterion 051-002** The report of every later campaign of release 0.3.0 states, for the chain of three and
  the fan-out, which product has the highest maximum sustained rate and which the lowest 99th percentile latency at 50%
  of its own maximum, from the medians of the repetitions, and whether the ranges overlap. The goal is that this
  platform leads both for both shapes; the criterion is that the answer is published either way.
- **Success criterion 051-003** In the crash-recovery table, this platform loses no acknowledged workflow under any of
  the three faults and sends no work to a person because of a fault, and its recovery times are reported for each.

### User stories

- **User story 051-01** A buyer comparing the three products reads one page with the same shapes on the same machine and
  can rerun it.
- **User story 051-02** An operator sizing a deployment reads the latency and recovery figures for the shape that looks
  like their workload.
- **User story 051-03** A contributor measures the gain of a hot-path change (design 052) with the same harness.

## Goals

- A comparison anyone can reproduce, and that a competitor's engineer would accept as fair.
- The first published crash-recovery times of the three products.
- One harness for every later performance claim of the platform.

## Non-Goals

- **No vendor-claim comparison.** Figures from other hardware are not put next to measured ones.
- **No managed services.** Temporal Cloud and Windmill's cloud are not measured.
- **No tuning of the competitors' source code** and no patched builds: only their published images and settings.
- **No multi-machine benchmark in the first campaign.** A campaign on three separate machines with a network between
  them is specified (Technical Plan, "The second tier") but needs an account the owner provides; until then every report
  says the products and the database share one machine.
- **No change to the platform.** Design 052 carries the hot-path changes; this design only measures.

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **One machine or several for the first campaign?** One. A GitHub-hosted larger runner can be started by a workflow
   of this repository with no account other than the organization's, and two jobs of a workflow cannot reach each other
   over a network, so three machines need a cloud account. One 64-vCPU runner, partitioned by CPU sets and memory
   limits, gives every product the same partition; the second tier follows when the owner provides an account. Delegate.
2. **Which PostgreSQL major?** The one the platform's CI uses (16 today), pinned by digest, for all three products. A
   later major is a separate campaign. Delegate.
3. **What load generator?** A driver of our own in Rust under `tests/performance/competitive/driver/`, outside the
   workspace, with one adapter per product that uses its official client: this platform's Rust SDK; Temporal's Go SDK
   through a small sidecar the driver calls over a local socket, whose own overhead is measured with a no-op adapter and
   reported; Windmill's REST API. One driver and one schedule make the rates comparable; Temporal's `omes` runs once in
   the pilots as a cross-check that the driver does not slow Temporal down. Delegate.
4. **Which language for activities?** Go for this platform and Temporal (both have an official Go SDK); for Windmill,
   the step language the pilot finds fastest for a step that returns its input, every candidate published. The script
   variants use TypeScript and Python on all three, with their isolation stated (Technical Plan, "Fixed constraints").
   Delegate.
5. **Which Temporal features are allowed?** Its defaults and its documented performance features (eager workflow
   start and eager activity dispatch, if the pilot shows them faster), not local activities, which have different
   durability and retry semantics from an activity and would not be the same shape. Delegate.
6. **Is beating the competitors a release gate?** No. Publishing the honest report is the gate (Success criteria 051-001
   and 051-003); leading is the goal (051-002). Making the release wait for a lead would push the report towards the
   result. Delegate (Owner decision 2026-11-24-4, pull request 274).

## Owner decisions needed

Both stay the owner's because they spend money or need an account (Owner decision 2026-11-24-4 keeps them):

- **The runner budget of the first campaign**: a 64-vCPU GitHub-hosted larger runner for about 12,654 minutes, US$
  2,049.95 at the price of 2026-11-24 (Technical Plan, "Cost of the campaigns"). Gates Tasks 051-9 to 051-11.
- **A cloud account for the second tier** (three machines in one zone). Gates Task 051-14.

## Critical User Journeys (CUJs)

### CUJ 1: Reading the comparison

*   **Given**: the published report of a campaign.
*   **When**: a reader opens the section of the chain of three.
*   **Then**: they see, for each product, the latency against the arrival rate at every rung, the maximum sustained rate
    with its resolution, the cost per million transitions, the machine and the configuration, and a link to the raw data
    and the command.

### CUJ 2: Reproducing it

*   **Given**: a GitHub organization with larger runners enabled.
*   **When**: someone runs the `competitive-benchmark` workflow by hand on a fork, at a tagged commit.
*   **Then**: the job produces a dated section and raw data in the same format; the figures fall within the range the
    report published, or the report's variance section says why not.

### CUJ 3: A crash under load

*   **Given**: each product running the chain of three at half its maximum sustained rate.
*   **When**: the database primary is killed.
*   **Then**: the report shows how long each product took to complete a workflow again and to return to its earlier
    latency, how many acknowledged workflows were lost, how many activities ran twice and how much work went to a person.

## Technical Plan (Architecture)

**Layout.** Everything lives under `tests/performance/competitive/`, outside the Cargo workspace (a rule of
`scripts/check-architecture.py` fails if a workspace member depends on it):

| Path | What it holds |
|---|---|
| `shapes.md` | The product-neutral definition of each variant: steps, inputs, outputs, timeouts, retry policy, what counts as complete |
| `driver/` | The Rust driver: schedule, adapters, histograms (HdrHistogram), per-second counters, the execution-record counter, the fault injector |
| `sagat/`, `temporal/`, `windmill/` | For each product: the compose file with images by digest, the configuration, the workflow and activity code of each variant, the history loader |
| `postgres/` | The PostgreSQL configuration and design 048's development topology (Patroni, etcd, HAProxy) |
| `run.sh` | The one command: prepares the machine, loads the histories, runs the rungs in rotating order, injects the faults, writes the raw data and the report section |
| `.github/workflows/competitive-benchmark.yml` | A `workflow_dispatch` workflow only, with the runner label as an input |

**The machine.** The first tier is one GitHub-hosted larger runner, Linux x64, 64 vCPU, 256 GB of memory, 2,040 GB of
disk (the published specification of the class, `docs.github.com/en/actions/reference/runners/larger-runners`, read on
2026-11-24). CPU sets are derived at start from `lscpu -e` by physical core, so that two hardware threads of one core are
never in two sets; the CPU model is recorded.

| Component | vCPU | Memory limit |
|---|---:|---:|
| PostgreSQL primary | 16 | 64 GB |
| Two synchronous standbys | 4 + 4 | 16 + 16 GB |
| etcd (three members) and HAProxy | 2 | 4 GB |
| The product under test (servers and workers) | 28 | 96 GB |
| The driver and the Temporal sidecar | 6 | 16 GB |
| Metrics collection | 4 | 8 GB |

16 + 8 + 2 + 28 + 6 + 4 = 64 vCPU; 64 + 32 + 4 + 96 + 16 + 8 = 220 GB of 256, leaving 36 GB to the system. Only the
product under test runs during its turn. The standbys are on during every run, in synchronous mode, because a deployment
that wants zero loss of acknowledged work pays for them, and fault (c) needs them.

**Fixed constraints** (each a deviation line where it departs from a product's documentation):

- One PostgreSQL configuration file for all three products; product-specific session settings are allowed and listed.
- Temporal's visibility store is SQL on the same PostgreSQL (its Elasticsearch option is a second database the others do
  not get).
- Isolation of the script variants, stated in the report: this platform runs each script in a container per run;
  Windmill runs with its sandbox (`nsjail`) on; Temporal has no script sandbox, so its script is an activity in a Python
  or TypeScript worker, not isolated, and the report marks that row "not equivalent".
- Timeouts and retries as `shapes.md` fixes them; this platform's activities declare themselves safe to retry, so that a
  lapsed lease is retried as the others' are, not sent to review.

**Order.** One job runs one variant for the three products. Each repetition uses one of the three rotations of the
product order, so that over three repetitions each product is first, second and third once; within a job the order
also rotates by rung. A rung is 60 s of warm-up, a measured window, and up to 60 s of drain. The window is 300 s, or
long enough for 2,000 workflow completions at the rung's rate, at most 900 s; a 99th percentile from 2,000 samples has
20 samples above it, and the report gives the sample count of every percentile.

**Faults per product.**

| Fault | This platform | Temporal | Windmill |
|---|---|---|---|
| (a) server process | one of two engine replicas | the history service of one of two server processes | one of two server processes |
| (b) worker | one of two worker processes | one of two worker processes | one of two worker processes |
| (c) database | the primary of design 048's development topology, for all three |

A crash job runs one fault for the three products: 5 minutes before the fault, 10 after, then the 10-minute drain. Each
of the five repetitions of a fault is a job of its own on a fresh runner.

**What each adapter does.** It starts a workflow with an idempotency key, sends a signal, and awaits the result through
the product's result API; the end time is when that call returns, on the driver's clock. Every activity records
(workflow, step, attempt) with the driver's counter over a local socket; the counter gives the repeats and is checked
against the product's own record of each workflow after the run.

**The report.** A dated section of `tests/performance/reports/competitive-benchmark.md`:

1. The environment: runner class, CPU model, kernel, PostgreSQL version and configuration, the CPU sets and memory
   limits, the date, the commit of this platform, the versions and digests of the competitors.
2. One table per variant: for each product, the maximum sustained rate with its resolution, the median and 99th
   percentile latency at 50% of it, the 99th percentile at the maximum, CPU seconds per thousand transitions of the
   product and of PostgreSQL, WAL bytes per transition, cost per million transitions; each cell the median of the
   repetitions with its range.
3. One latency-against-rate chart per variant, the three products on it.
4. The crash-recovery table: per product and fault, time to the first completion, time back to the earlier latency,
   acknowledged lost, activities repeated, work sent to a person.
5. Variance: the coefficient of variation of each product's maximum sustained rate across repetitions.
6. Deviations, fixed constraints and failures (Requirement 051-013), and the pilot runs that chose each configuration.
7. How to reproduce: the command, the inputs, the runner label.

**The second tier.** Three machines of one instance type in one zone of one cloud provider: the database, the product,
the driver, with the network round trip measured and stated. The same harness, `SAGAT_COMPETITIVE_TOPOLOGY=three-hosts`.

**Cost of the campaigns.** At US$ 0.162 per minute for a 64-vCPU Linux x64 larger runner
(`docs.github.com/en/billing/reference/actions-runner-pricing`, read on 2026-11-24). Each throughput job runs at most
8 doubling rungs, 3 bisection rungs and 1 rung at half the maximum, 12 rungs per product, plus about 75 minutes to
prepare the machine and load the three histories:

| Campaign | Minutes | Cost |
|---|---:|---:|
| Throughput, 8 variants × 3 repetitions = 24 jobs, rungs of at most 7 minutes: 3 × 12 × 7 = 252 + 75 = 327 minutes each | 24 × 327 = 7,848 | 7,848 × 0.162 = US$ 1,271.38 |
| Throughput, fan-out × 3 repetitions, rungs of at most 17 minutes, each repetition split in two jobs to stay under the six-hour job limit: 3 × 12 × 17 = 612 + 2 × 75 = 762 minutes per repetition | 3 × 762 = 2,286 | 2,286 × 0.162 = US$ 370.33 |
| Crash recovery, 3 faults × 5 repetitions = 15 jobs: 3 × 25 + 30 + 15 = 120 minutes each | 15 × 120 = 1,800 | 1,800 × 0.162 = US$ 291.60 |
| Pilots: the competitors' configurations and the starting rates, about 4 jobs of 3 hours | 4 × 180 = 720 | 720 × 0.162 = US$ 116.64 |
| Total of one full campaign | 7,848 + 2,286 + 1,800 + 720 = 12,654 | 12,654 × 0.162 = US$ 2,049.95 |

The pilot of Task 051-8 runs first on the standard hosted runner, which costs nothing for this public repository, at
small rates.

**Cost per million transitions, worked through.** For a product that sustains 2,000 transitions per second on the
64-vCPU runner: one minute holds 60 × 2,000 = 120,000 transitions, so a million cost 0.162 ÷ 120,000 × 1,000,000 =
US$ 1.35. The figure is an example of the arithmetic, not a measurement.

**Order of the build.** Design 048's development topology (its Task 048-2) comes before Task 051-7, and this harness
(Tasks 051-2 and 051-6) before design 048's drills (its Tasks 048-15 to 048-17), which reuse the driver.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The comparison is unfair to a competitor and is dismissed | Each competitor's best configuration found by published pilots; fixed constraints stated; the method published before the first paid run; a correction is a new dated section. Inviting the vendors to read the method is the owner's act |
| A hosted runner is a shared virtual machine whose speed varies | Three jobs on fresh runners, rotating order, the coefficient of variation published |
| One machine hides the network cost | Stated in every report of the first tier; the second tier measures it |
| A competitor's licence forbids publishing benchmarks | Temporal is MIT-licensed and Windmill's community edition AGPL-3.0; neither restricts publishing measurements; Task 051-1 rechecks the pinned versions and the report links them |
| The harness measures itself | The driver has its own cores; a no-op adapter measures its ceiling, and the Temporal sidecar's hop is measured; the campaign stops if the ceiling is under ten times the highest sustained rate |
| Empty tables flatter one product | The history of Requirement 051-015 |
| Money is spent on a broken harness | The pilot on the free runner at small rates first |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | n/a | The design measures; it changes no state handling |
| II. Modular domain, ports and adapters | Pass | Test code outside the workspace; no crate depends on it (Requirement 051-014) |
| III. External effects are at-least-once; sagas compensate | Pass | Repeated executions and work sent to a person are counted, not hidden (Requirement 051-009) |
| IV. Compatibility is versioned and pinned | Pass | Competitor images and the platform's commit pinned |
| V. Code, tenant boundary and secrets are isolated | Pass | The job holds no repository secret and runs only by hand; script isolation is stated per product |
| VI. Observability and audit are part of the feature | Pass | The report states the environment, the method and the failures |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash recovery is measured for every product; a loss is published as a loss; unmeasured numbers are never filled in |
| Delivery Constraints | Pass | "Targets are not promises": every figure states its environment; the first tier shares one machine and says so |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A benchmark harness for three products | A claim of surpassing needs the same shapes on the same machine | Comparing vendor figures: different hardware, shapes and units |
| A Temporal Go sidecar | The driver is Rust and Temporal has no official Rust SDK | `omes` as Temporal's driver: a different load model per product |
| One machine split by CPU sets in the first tier | Reproducible with a GitHub organization, no cloud account | Waiting for three machines: no report in 0.3.0 without an account |
| A pre-loaded history | Empty tables are not a workload | Fresh databases: favours products whose cost grows with history |

## Alternatives Considered

- **Use each vendor's benchmark tool.** Different loops and units; one driver, with `omes` as a cross-check.
- **Run on the development laptop.** Not valid evidence: shared, closed loop, other work running.
- **Measure closed loop and report the peak.** Hides queueing; open loop with latency from the scheduled start.
- **A doubling ladder alone.** A factor-two resolution cannot rank products; bisection gives one eighth.
- **Compare only throughput.** A product that loses or repeats work under a crash can look faster; crash recovery and
  integrity are part of every report.

## Non-Functional Requirements

- **Non-functional requirement 051-001 Reproducibility.** Two campaigns at the same commit on the same runner class
  give maximum sustained rates within the range their repetitions report, or the report explains the difference.
- **Non-functional requirement 051-002 Cost.** One full campaign stays under 14,000 runner minutes (12,654 planned;
  12,654 × 1.1 = 13,919.4, rounded up to 14,000).
- **Non-functional requirement 051-003 Security.** The benchmark job holds no secret of the repository and runs only by
  hand; the competitor images reach no network beyond the job's machine.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T051-1 Write `tests/performance/competitive/shapes.md` with the timeouts and retry policies, check the licences of the pinned competitor versions, and pin their images by digest.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-009
- [ ] T051-2 Build the driver outside the workspace: open-loop schedule from a seed, latency from the scheduled start, result-API completion, the execution-record counter, HdrHistogram output, per-second counters, a no-op adapter that measures its ceiling, and the architecture rule that keeps it out of the workspace.
  Refs: Requirement 051-005, Requirement 051-007, Requirement 051-008, Requirement 051-014
- [ ] T051-3 Write this platform's adapter, the nine variants as workflow versions with Go activities declared safe to retry, and its history loader.
  Refs: Requirement 051-001, Requirement 051-008, Requirement 051-015
- [ ] T051-4 Write Temporal's compose file (server, PostgreSQL persistence and SQL visibility), its Go worker and sidecar with the sidecar's measured hop, the nine variants and its history loader.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-015
- [ ] T051-5 Write Windmill's compose file with its sandbox on, its flows for the nine variants, the step-language pilot and its history loader.
  Refs: Requirement 051-001, Requirement 051-004, Requirement 051-015
- [ ] T051-6 Write `run.sh`: CPU sets by physical core, memory limits, the PostgreSQL configuration, rotating order, the rung rule with bisection and the half-maximum rung, raw data and the report section.
  Refs: Requirement 051-003, Requirement 051-006, Requirement 051-010, Requirement 051-012
- [ ] T051-7 Add the fault injector and the crash-recovery jobs on design 048's development topology, with the faults-per-product table.
  Refs: Requirement 051-009
- [ ] T051-8 Add the `competitive-benchmark` workflow (manual dispatch only), publish the method and configurations, and run a pilot on the standard runner at small rates.
  Refs: Requirement 051-012, Requirement 051-014
- [ ] T051-9 Run the pilots that choose each competitor's configuration and each variant's starting rate on the larger runner, and publish them.
  Refs: Requirement 051-004, Requirement 051-006
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-10 Run the first full campaign and publish the dated report section and raw data.
  Refs: Success criterion 051-001, Requirement 051-011, Requirement 051-013
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-11 Publish the crash-recovery table of the first campaign.
  Refs: Success criterion 051-003
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-12 Write the reader's guide to the report and link it from `docs/README.md` and the roadmap.
  Refs: User story 051-01, User story 051-02
- [ ] T051-13 State, in every later campaign of 0.3.0, the ranking of Success criterion 051-002 with the overlap of the ranges.
  Refs: Success criterion 051-002
- [ ] T051-14 Run the second tier on three machines and publish it.
  Refs: Requirement 051-003
  Owner-gate: pending, the owner's row providing the cloud account of "Owner decisions needed"

#### Follow-up tasks

None yet.

### Verification Plan

The design is verified by its own output. The pilot on the standard runner (Task 051-8) must produce a report section in
the format of Requirement 051-012 with every variant completing on every product, before money is spent. The no-op
adapter's ceiling must be at least ten times the highest sustained rate any product reaches, or the driver is the
bottleneck and the campaign stops. The driver's rung rule, bisection, rotation and latency accounting have unit tests
against a simulated product whose saturation point is known; the fault injector has a test that its recorded fault time
matches the process's death on the driver's clock to within 100 ms.
