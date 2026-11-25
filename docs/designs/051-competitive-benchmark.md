# A published benchmark against Temporal and Windmill

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

The owner's aim for release 0.3.0 is that the platform surpass Temporal and Windmill, and the owner asked for a
published benchmark of the same workflow shapes on the three products, crash-recovery time included (Owner decision
2026-11-24-1, pull request 273, whose record quotes the owner: "so that we surpass Windmill and Temporal"). This design
is that benchmark. It changes no platform code.

One harness runs the same eight ladder variants (chained activities, fan-out and fan-in, signals, timers, sagas, scripts)
and one timer-burst procedure on this platform, on Temporal (self-hosted, PostgreSQL persistence) and on Windmill
(self-hosted). The load is open loop. The three products run in turn on the same declared machine and the same
PostgreSQL build, each against a restored database that already holds a history. The harness measures throughput, the
latency of a whole workflow as each product records its end, the time to recover from three kinds of crash, the work
lost, stuck, failed, repeated or sent to a person, and the cost per million transitions. A report of a fixed format is
published with its raw data, its configuration and the command that reproduces it. A result where this platform loses is
published as a loss.

The hot-path remedies that the first draft of this design also carried are design 052, whose gains this harness
measures.

**Terms used below.** A *run* is one execution of a workflow; a *step* is one node of it, and an *activity* is a step
that a worker executes. A *transition* is one completed step recorded durably (Requirement 051-008). A *shape* is a
workflow defined once in product-neutral terms and implemented for each product. A *rung* is one target arrival rate of
the load, measured over a fixed window. *Open loop* means that starts are sent at their scheduled times whatever the
system does; *coordinated omission* is the error of measuring latency from when a request was sent instead of when it
was due. A *censored* latency is one not observed because the workflow had not ended, or failed.

## The Problem

**There is no comparison, only vendor figures.** The only competitor table in the repository
(`docs/roadmap-assessment-2026-11-11.md`, "Numbers to beat") holds vendor claims measured on other hardware with other
shapes: Temporal self-hosted 1,350 state transitions per second on a 4-core database, Windmill "up to 1,000 steps per
second per worker" with 54 ms of overhead per job. Nobody runs the three products on the same machine with the same
workflows, and nobody publishes crash-recovery times.

**The platform's own figures are not a benchmark.** The transitions scenario of `tests/performance/scalability-ha/`
calls the services inside one process, with no gRPC, TLS or network hop (`tests/support/engine.rs`, `Replica`), in a
closed loop, on a shared laptop (load average 12 in October, 26 on 2026-11-12). The best row was 1,513 transitions per
second on 2026-10-11 at `9092aa1`, 368 on 2026-11-11 at `ed11d044`, and 1,026 on 2026-11-12 at `1e28a138` after design
038 (`tests/performance/reports/scalability-ha.md`). No crash-recovery time is measured: the disaster-recovery drill
times a promotion on loopback (0.2 s) and adds a 15 s shard takeover as a constant.

## Requirements

- **Requirement 051-001 Shapes.** The harness MUST run these eight ladder variants and one procedure, each defined once
  in `tests/performance/competitive/shapes.md` (steps, payload sizes, expected outcome, transitions per workflow,
  timeouts, retry policy, idempotency mechanism per product) and implemented for each product in its own idiom: a chain
  of three activities and a chain of ten; a fan-out of one activity, 100 in parallel and a join; a signal shape (start,
  wait for a named signal sent 1 s after the start is acknowledged, one activity); a timer shape (a durable 1 s sleep,
  one activity); a saga (three activities, the third failing permanently, the first two compensated in reverse; expected
  outcome compensated, five transitions); a script step in TypeScript and one in Python; and the *timer-burst procedure*
  (Requirement 051-016).
- **Requirement 051-002 Products and versions.** The three products MUST run self-hosted from images pinned by digest:
  this platform at the commit under test; Temporal server, each of its services in its own process, with PostgreSQL
  persistence; Windmill community edition with its PostgreSQL. The versions, digests and configuration files are part of
  the report, and so are the tasks of designs 045 and 048 that the platform's commit contains.
- **Requirement 051-003 Same machine, same database build.** In one benchmark job the three products MUST run in turn on
  the same machine and the same PostgreSQL build, each in its own database restored from its history snapshot
  (Requirement 051-015). The machine MUST be declared (runner class, CPU model, vCPU, memory, disk, kernel) and MUST NOT
  be the shared development laptop. Each product gets the same CPU set and memory limit; CPU sets do not isolate the
  disk or the shared caches, which the report states.
- **Requirement 051-004 Best configuration within fixed constraints.** Each product MUST run in one configuration, the
  best the pilots find across the shapes (a per-shape change only where a pilot shows it matters), within the fixed
  constraints of Technical Plan, "Fixed constraints"; each constraint and each excluded feature is a deviation line of
  the report.
- **Requirement 051-005 Open-loop load.** The driver MUST start workflows on a precomputed schedule of exponentially
  distributed inter-arrival times at a target rate, from a fixed seed, independent of completions. An *arrival* is a
  scheduled start. A start the product refuses is retried with the same idempotency key under its documented advice for
  at most 10 s; a retry answered "already exists" counts as accepted; an arrival still refused after 10 s is a failed
  arrival.
- **Requirement 051-006 Rungs and the maximum sustained rate.** Each ladder variant MUST start at the rate the pilots
  chose for it and double at most six times, then bisect three times between the last sustained and the first
  unsustained rung; if the first rung is not sustained the rate halves, at most three times, in place of doubling rungs.
  A rung is *sustained* when, by the end of its drain, at least 99% of its arrivals ended with the shape's expected
  outcome, the backlog at the end is not larger than at the start by more than 1% of the window's arrivals, and the 99th
  percentile latency (censored values included as infinite) is under the variant's latency ceiling fixed in `shapes.md`.
  A ladder that sustains every doubling rung reports "at least" its last rate. Two products whose bisection intervals
  overlap, or whose repetition ranges overlap, are reported as tied.
- **Requirement 051-007 What is measured.** For each product, variant and rung: arrivals, failed arrivals, completions by
  outcome, transitions per second, the median and 99th percentile of workflow latency from the scheduled start, with
  failed and open workflows counted as infinite and the 99th percentile reported as "more than" its bound when censored
  values reach 1%, the signal-to-resume latency (signal shape: from the signal's send to the activity's start record),
  CPU seconds and peak memory of the product's processes and of PostgreSQL, the WAL bytes and commits per transition of
  the product's database, and the database connections used. Every percentile carries its sample count; one from fewer
  than 2,000 samples is marked.
- **Requirement 051-008 One unit of work, recorded by the product.** A *transition* is one completed step of the shape;
  a workflow's transitions are its shape's constant count (`shapes.md`), counted for each workflow that ended with the
  expected outcome. A workflow's end MUST be its end time as the product records it (this platform's run end, Temporal's
  close time, Windmill's completion time), read after the rung; the product's result API is polled only to count
  completions during the rung, at a fixed low rate whose request load is reported. Every activity MUST record its
  workflow and step, synchronously and acknowledged, to the driver's counter at its start and at its end; a *repeat* is
  any execution of a (workflow, step) beyond the shape's expected count. Script steps, whose containers have no network,
  are counted from the product's own records. Product internal units MUST NOT be compared.
- **Requirement 051-009 Crash recovery.** The harness MUST inject three faults into each product while it runs the chain
  of three at 50% of its maximum sustained rate: (a) `kill -9` of one of two server processes behind the product's load
  balancer; (b) `kill -9` of one of two worker processes with activities in flight; (c) stopping the container of the
  PostgreSQL primary (Patroni and PostgreSQL together) of design 048's development topology, with a synchronous standby
  promoted and the product reconnecting through the same address. For each it MUST measure: the time until the last
  workflow in flight at the fault ends; the time until the completion rate is at least 90% of its rate before the fault
  and the 99th percentile latency of new starts within 20% of its level before the fault, held for three consecutive
  10 s windows of at least 100 samples each; for fault (c), the promotion time of Patroni on its own; and, after a
  10-minute drain, each acknowledged start classified from the product's own record as ended, failed, stuck (present and
  not ended) or lost (absent), with repeats and work sent to a person. The timeouts that decide recovery are fixed per
  product in `shapes.md`, equal where the products can express them and a deviation line where they cannot.
- **Requirement 051-010 Repetitions and order.** Every throughput measurement MUST be repeated in three benchmark jobs on
  fresh machines, and every crash measurement in six; the report shows the median and the range. Within a job the
  products run in passes: each pass runs the next rung of every product still climbing, in an order that rotates by pass
  and by repetition; over six crash repetitions every order of the three products is used once. The report tests for an
  order effect (a product's result by its position) and states it.
- **Requirement 051-011 Cost per million transitions.** For each product and variant the report MUST state
  `cost = price_per_minute ÷ (60 × sustained_transitions_per_second) × 1,000,000`, with the price per minute of the
  declared machine at the date of the run and its source, labelled as the reciprocal of throughput on one machine, not
  the cost of a deployment.
- **Requirement 051-012 Report format and reproduction.** Each campaign MUST publish, through a pull request, a dated
  section of `tests/performance/reports/competitive-benchmark.md` in the format of Technical Plan, "The report"; its raw
  data are an artifact of the workflow run, linked from the section. One command, `tests/performance/competitive/run.sh`,
  reproduces it on a machine of the declared class. The method and the configurations MUST be published in the
  repository before the first campaign run.
- **Requirement 051-013 Failures are reported.** A run that fails, a rung that did not sustain, a product that could not
  run a shape natively, and every deviation from this design MUST be stated in the report. A number that was not
  measured is never filled in.
- **Requirement 051-014 Isolation.** The competitor images, their adapters and the benchmark workflow MUST live only
  under `tests/performance/competitive/` and run only by hand in the benchmark workflow. The driver and this platform's
  adapter MAY be reused by designs 048 and 052. The driver is outside the Cargo workspace (excluded in the root
  `Cargo.toml`), its lock file and the Temporal sidecar's modules are audited in CI, and no production crate depends on
  any of it.
- **Requirement 051-015 A history, loaded once.** Each product's database MUST hold 1,000,000 completed chains of three,
  loaded through the product itself once per campaign in a job of its own, then snapshotted (the data directory, pinned
  by digest) and restored before each product's turn in every later job; after each restore a `VACUUM (ANALYZE)` and a
  `CHECKPOINT` run before the first rung.
- **Requirement 051-016 The timer-burst procedure.** For each product, 100,000 workflows are started over 5 minutes at a
  steady rate, each sleeping until one absolute instant 10 minutes after the first start; the procedure measures the
  time from that instant until every timer has fired and the lateness percentiles of the firings. It is not a ladder.

### Success criteria

- **Success criterion 051-001** A first campaign is published with all variants and the procedure on all three products,
  the crash-recovery table, and every deviation stated.
- **Success criterion 051-002** The report of every later campaign of release 0.3.0 states, for the chain of three and the
  fan-out, which product has the highest maximum sustained rate, and which the lowest 99th percentile latency at a common
  rate (half the lowest maximum of the three), with ties as Requirement 051-006 defines them. The goal is that this
  platform leads both for both shapes; the criterion is that the answer is published either way.
- **Success criterion 051-003** In the crash-recovery table, this platform loses no acknowledged workflow under any of the
  three faults, and its recovery times are reported for each. That it sends no work to a person follows from its
  activities being declared safe to retry, which the report says.

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

- **No vendor-claim comparison**, no managed services, no tuning of the competitors' source code.
- **No multi-machine benchmark in the first campaign.** The second tier needs an account the owner provides.
- **No change to the platform.** Design 052 carries the hot-path changes; designs 045 and 048 carry what decides this
  platform's crash results (activity timeouts, the failover behaviour), and the report names which of their tasks the
  commit under test has.

## Open Questions

Each was decided under the delegation of Owner decision 2026-11-20-6, with its reasoning.

1. **One machine or several for the first campaign?** One 64-vCPU GitHub-hosted runner, partitioned by CPU sets and
   memory limits: reproducible with no account beyond the organization's, while two jobs of a workflow cannot reach each
   other over a network. The second tier follows when the owner provides an account. Delegate.
2. **Which PostgreSQL major?** The one the platform's CI uses (16 today), pinned by digest, for all three. Delegate.
3. **What load generator?** A driver of our own in Rust, outside the workspace, with one adapter per product over its
   official client: this platform's Rust SDK; Temporal's Go SDK through a sidecar whose own overhead is measured with a
   no-op adapter; Windmill's REST API. Temporal's `omes` runs once in the pilots as a cross-check. Delegate.
4. **Which language for activities?** Go for this platform and Temporal; for Windmill, the step language the pilot finds
   fastest, every candidate published. The script variants use TypeScript and Python on all three, isolated as stated in
   "Fixed constraints". Delegate.
5. **Which competitor features are allowed?** Their defaults and documented performance features that run inside their
   own partition: Temporal's eager activity dispatch if faster; not its eager workflow start (it runs the worker in the
   starting client, which is the driver's partition), not local activities (different semantics). Windmill's dedicated
   workers are an Enterprise Edition feature, excluded, and the report says so. Delegate.
6. **Is beating the competitors a release gate?** No. Publishing the honest report is the gate (Success criteria 051-001
   and 051-003); leading is the goal (051-002). Delegate (Owner decision 2026-11-24-4, pull request 274).

## Owner decisions needed

Both stay the owner's because they spend money or need an account (Owner decision 2026-11-24-4 keeps them):

- **The runner budget**: the first campaign, about 12,912 minutes of a 64-vCPU larger runner, US$ 2,091.74 at the price
  of 2026-11-24; each later campaign of 0.3.0 at the same commit's versions, about 11,232 minutes, US$ 1,819.58; and design
  052's on-and-off measurement, about 1,368 minutes, US$ 221.62 (Technical Plan, "Cost of the campaigns"). Gates Tasks
  051-9 to 051-11 and 051-13.
- **A cloud account for the second tier** (three machines in one zone). Gates Task 051-14.

## Critical User Journeys (CUJs)

### CUJ 1: Reading the comparison

*   **Given**: the published report of a campaign.
*   **When**: a reader opens the section of the chain of three.
*   **Then**: they see, for each product, the latency against the arrival rate at every rung, the maximum sustained rate
    with its resolution and any tie, the cost per million transitions, the machine and the configuration, and links to the
    raw data and the command.

### CUJ 2: Reproducing it

*   **Given**: a GitHub organization with larger runners enabled.
*   **When**: someone runs the `competitive-benchmark` workflow by hand on a fork, at a tagged commit.
*   **Then**: the job produces a dated section and raw data in the same format; each maximum sustained rate falls within
    the published median plus or minus twice the published standard deviation of the repetitions, or the report of the
    new run says why not.

### CUJ 3: A crash under load

*   **Given**: each product running the chain of three at half its maximum sustained rate.
*   **When**: the database primary's container is stopped.
*   **Then**: the report shows Patroni's promotion time, how long each product took to end the workflows in flight and to
    hold its earlier rate and latency, and how many acknowledged workflows were lost, stuck or failed, how many activities
    ran twice and how much work went to a person.

## Technical Plan (Architecture)

**Layout.** Everything lives under `tests/performance/competitive/`:

| Path | What it holds |
|---|---|
| `shapes.md` | Each variant and the procedure: steps, payload sizes, expected outcome, transitions per workflow, latency ceiling, timeouts and retry policy per product, idempotency mechanism per product |
| `driver/` | The Rust driver (its own workspace): schedule, adapters, histograms (HdrHistogram), per-second counters, the execution-record counter, the fault injector, the plan check |
| `sagat/`, `temporal/`, `windmill/` | For each product: the compose file with images by digest and an internal network, its load balancer, the configuration, the code of each variant, the history loader |
| `postgres/` | The PostgreSQL configuration and design 048's development topology (Patroni, etcd, HAProxy) |
| `run.sh` | The one command: prepares the machine, restores the snapshots, runs the passes, injects the faults, writes the raw data and the report section |
| `.github/workflows/competitive-benchmark.yml` | `workflow_dispatch` only, read-only permissions, no secrets, `timeout-minutes` 355 per job, the runner label an input |

A rule of `scripts/check-architecture.py` fails when the workflow has another trigger, a write permission, a `secrets.`
reference or a timeout above 355, when an image under `tests/performance/competitive/` is not pinned by `@sha256:`, or
when a workspace member depends on the driver.

**The machine.** One GitHub-hosted larger runner, Linux x64, 64 vCPU, 256 GB of memory, 2,040 GB of disk (the published
specification of the class, `docs.github.com/en/actions/reference/runners/larger-runners`, read on 2026-11-24). CPU sets
are derived at start from `lscpu -e` by physical core, so that two hardware threads of one core are never in two sets;
the CPU model is recorded. Only the product under test runs during its turn; the others are stopped.

| Component | vCPU | Memory limit |
|---|---:|---:|
| Three equal PostgreSQL nodes with their Patroni agents | 3 × 8 = 24 | 3 × 32 = 96 GB |
| etcd (three members) and HAProxy for the database | 2 | 4 GB |
| The product under test (servers, workers, its load balancer) | 28 | 96 GB |
| The driver and the Temporal sidecar | 6 | 16 GB |
| Metrics collection | 4 | 8 GB |

24 + 2 + 28 + 6 + 4 = 64 vCPU; 96 + 4 + 96 + 16 + 8 = 220 GB of 256, leaving 36 GB to the system. The three database
nodes are equal so that a promotion changes no capacity. The standbys are on during every run, synchronous, because a
deployment that wants zero loss of acknowledged work pays for them. Uninjected leader changes are detected from
Patroni's history and reported.

**Fixed constraints** (each a deviation line where it departs from a product's documentation):

- One PostgreSQL configuration for all three; product-specific session settings are allowed and listed.
- Temporal's visibility store is SQL on the same PostgreSQL.
- Isolation, stated per row: activities of all three run unsandboxed in their workers; for the script variants this
  platform runs a container per run, Windmill its sandbox (`nsjail`), and Temporal an unisolated activity, marked "not
  equivalent".
- Timeouts as `shapes.md` fixes them: this platform's lease time to live (and, once design 045 is built, its per-activity
  heartbeat timeout), Temporal's heartbeat and start-to-close timeouts, Windmill's zombie-job timeout; this platform's
  activities declare themselves safe to retry.

**Passes and timing.** A rung is 60 s of warm-up, a 300 s window and up to 60 s of drain: 7 minutes. Before each rung the
product's backlog is drained to empty (at most 60 s, then cancelled through the product's API, recorded); between
products a `CHECKPOINT` runs and the job waits, at most 2 minutes, for the standbys' lag to reach zero and autovacuum to
go idle, recorded, and charged to no product. A product's ladder is at most 6 doubling (or halving) rungs, 3 bisection
rungs, the rung at half its maximum and the common-rate rung: 11 rungs. Before starting, `run.sh` computes the plan's
minutes and refuses to start above 350.

**Faults per product.**

| Fault | This platform | Temporal | Windmill |
|---|---|---|---|
| (a) server process | one of two engine replicas | one of two history-service processes | one of two server processes (on Windmill the server serves the API; the report marks this row "API only") |
| (b) worker | one of two worker processes | one of two worker processes | one of two worker processes |
| (c) database | the primary's container in design 048's development topology, for all three |

A crash job runs one fault, one repetition, for the three products. Before each product's turn the topology is rebuilt
(the old primary rejoined by `pg_rewind`, the original primary restored by a planned switchover) and checked, timed and
charged to no product.

**The report.** A dated section of `tests/performance/reports/competitive-benchmark.md`: the environment (runner class,
CPU model, kernel, PostgreSQL version and configuration, CPU sets and memory limits, date, this platform's commit and its
design 045 and 048 tasks, competitor versions and digests); one table per variant (maximum sustained rate with resolution
and ties, median and 99th percentile at half the maximum and at the common rate, the 99th percentile at the maximum, CPU
seconds per thousand transitions of the product and of PostgreSQL, WAL bytes per transition, cost per million
transitions; each cell the median of the repetitions with its range and sample counts); one latency-against-rate chart
per variant; the timer-burst table; the crash-recovery table; the order-effect test; the coefficient of variation,
labelled as resting on three repetitions; deviations, fixed constraints, failures and the pilots; how to reproduce.

**The second tier.** Three machines of one instance type in one zone: the database, the product, the driver, with the
network round trip measured; `SAGAT_COMPETITIVE_TOPOLOGY=three-hosts`.

**Cost of the campaigns.** At US$ 0.162 per minute for a 64-vCPU Linux x64 larger runner
(`docs.github.com/en/billing/reference/actions-runner-pricing`, read on 2026-11-24):

| Campaign | Minutes | Cost |
|---|---:|---:|
| History load: one job per product, at most 180 minutes of loading and 20 of preparation each | 3 × 200 = 600 | 600 × 0.162 = US$ 97.20 |
| Throughput: 8 variants × 3 repetitions = 24 jobs; each 3 products × 11 rungs × 7 = 231, plus 3 restores of 10, 2 inter-product waits of 2 and 20 of preparation: 231 + 30 + 4 + 20 = 285 | 24 × 285 = 6,840 | 6,840 × 0.162 = US$ 1,108.08 |
| Timer burst: 3 repetitions; each 3 products × 20 minutes + 30 of restores + 20 of preparation = 110 | 3 × 110 = 330 | 330 × 0.162 = US$ 53.46 |
| Crash recovery: 3 faults × 6 repetitions = 18 jobs; each 3 products × (25 measured + 10 rebuild) + 30 restores + 20 preparation = 155 | 18 × 155 = 2,790 | 2,790 × 0.162 = US$ 451.98 |
| Pilots: one configuration per product and each variant's starting rate, 6 jobs of 3 hours | 6 × 180 = 1,080 | 1,080 × 0.162 = US$ 174.96 |
| Total of the first campaign | 600 + 6,840 + 330 + 2,790 + 1,080 = 11,640 | 11,640 × 0.162 = US$ 1,885.68 |

A throughput job of 285 minutes is under the plan check's 350 and the job limit of 355. A later campaign at the same
versions reuses the snapshots and skips the pilots: 11,640 - 600 - 1,080 = 9,960 minutes, US$ 1,613.52. Design 052's
measurement runs this platform alone, on and off, for the chain of three and the fan-out, three repetitions each: 2 × 2 ×
3 = 12 jobs of 11 × 7 + 10 + 20 = 107 minutes, 12 × 107 = 1,284 minutes, US$ 208.01. The pilot of Task 051-8 runs first
on the standard hosted runner, free for this public repository, in a reduced profile (a history of 10,000 workflows, no
standbys, no partition table), as a functional check only.

**Cost per million transitions, worked through.** For a product that sustains 2,000 transitions per second: one minute
holds 60 × 2,000 = 120,000 transitions, so a million cost 0.162 ÷ 120,000 × 1,000,000 = US$ 1.35. The figure is an
example of the arithmetic, not a measurement.

**Order of the build.** Design 048's development topology (its Task 048-2) comes before Task 051-7; the driver (Task
051-2) before design 048's mixed-writer test and Compose drill (its Tasks 048-15 and 048-17), which reuse it.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The comparison is unfair to a competitor and is dismissed | Pilots published; fixed constraints and excluded features stated; the method published before the first campaign run; a correction is a new dated section. Inviting the vendors to read the method is the owner's act |
| A hosted runner is a shared virtual machine whose speed varies | Fresh runners per repetition, rotating order, the order-effect test |
| One machine hides the network cost, and shares disk and caches | Stated in every report of the first tier; the second tier measures the network |
| A competitor's licence forbids publishing benchmarks | Temporal is MIT-licensed and Windmill's community edition AGPL-3.0; neither restricts publishing measurements; Task 051-1 rechecks the pinned versions |
| The harness measures itself | Its own cores; a no-op adapter measures its ceiling; the sidecar's hop is measured; the campaign stops below ten times the highest sustained rate |
| One rung or product contaminates the next | Backlog drained or cancelled before each rung; a checkpoint and waits between products, recorded |
| A job is killed by the time limit | The plan check refuses a plan above 350 minutes |
| The history load is slower than planned | It is its own job per product, measured in the pilot; a slower load is a longer job, not a longer campaign job |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | n/a | The design measures; it changes no state handling |
| II. Modular domain, ports and adapters | Pass | Test code outside the workspace; the architecture rule keeps it there |
| III. External effects are at-least-once; sagas compensate | Pass | Repeats, stuck, failed and lost work are counted (Requirement 051-009) |
| IV. Compatibility is versioned and pinned | Pass | Images pinned by digest and checked; the platform's commit named |
| V. Code, tenant boundary and secrets are isolated | Pass | The workflow holds no secret and has read-only permissions; competitor containers on internal networks, probed |
| VI. Observability and audit are part of the feature | Pass | The report states the environment, the method and the failures |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash recovery measured for every product; censored latencies never hidden; a loss is published as a loss |
| Delivery Constraints | Pass | "Targets are not promises": every figure states its environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A harness for three products | A claim of surpassing needs the same shapes on the same machine | Vendor figures: different hardware, shapes and units |
| A Temporal sidecar | The driver is Rust and Temporal has no official Rust SDK | `omes` for Temporal: a different load model |
| One machine split by CPU sets | Reproducible with a GitHub organization, no cloud account | Waiting for three machines |
| Restored history snapshots | Empty tables are not a workload, and loading per job does not fit the job limit | Fresh databases; loading per job |
| Three equal database nodes | A promotion must not change capacity | A large primary with small standbys |

## Alternatives Considered

- **Each vendor's own benchmark tool.** Different loops and units; one driver, with `omes` as a cross-check.
- **The development laptop.** Not valid evidence.
- **Closed loop, peak throughput.** Hides queueing; open loop with latency from the scheduled start.
- **A doubling ladder alone.** A factor-two resolution cannot rank; bisection gives one eighth, and ties are reported.
- **The result API as the clock of a workflow's end.** Polling intervals differ per product and load the system; the
  product's recorded end time does neither.

## Non-Functional Requirements

- **Non-functional requirement 051-001 Reproducibility.** A new campaign at the same commit on the same runner class
  gives each maximum sustained rate within the published median plus or minus twice the published standard deviation,
  or the report explains the difference.
- **Non-functional requirement 051-002 Cost.** The first campaign stays under 12,900 runner minutes (11,640 planned;
  11,640 × 1.1 = 12,804, rounded up).
- **Non-functional requirement 051-003 Security.** The benchmark workflow holds no secret, has read-only permissions and
  runs only by hand; the competitor containers reach no network beyond their internal one, checked by a probe at start.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T051-1 Write `tests/performance/competitive/shapes.md` (steps, payloads, expected outcomes, transitions, latency ceilings, timeouts, idempotency per product), check the licences of the pinned competitor versions, and pin their images by digest.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-009
- [ ] T051-2 Build the driver in its own workspace: open-loop schedule, latency from the scheduled start with censoring, the execution-record counter, the recorded-end reader per product, HdrHistogram output, per-second counters, the no-op adapter, the plan check; exclude it in the root `Cargo.toml`; add its lock and the sidecar's modules to CI's audits; add the architecture rule.
  Refs: Requirement 051-005, Requirement 051-007, Requirement 051-008, Requirement 051-014
- [ ] T051-3 Write this platform's adapter, the variants as workflow versions with Go activities declared safe to retry, and its history loader.
  Refs: Requirement 051-001, Requirement 051-008, Requirement 051-015
- [ ] T051-4 Write Temporal's compose file (each service in its own process, PostgreSQL persistence and SQL visibility, a load balancer), its Go worker and sidecar with the sidecar's measured hop, the variants and its history loader.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-015
- [ ] T051-5 Write Windmill's compose file (two server processes behind a load balancer, its sandbox on for the script variants), its flows for the variants, the step-language pilot and its history loader.
  Refs: Requirement 051-001, Requirement 051-004, Requirement 051-015
- [ ] T051-6 Write `run.sh`: CPU sets by physical core, memory limits, snapshot restores, passes with rotation, the rung rule with bisection, halving, the half-maximum and common-rate rungs, the drains and waits between rungs and products, the timer-burst procedure, raw data and the report section.
  Refs: Requirement 051-003, Requirement 051-006, Requirement 051-010, Requirement 051-012, Requirement 051-016
- [ ] T051-7 Add the fault injector, the faults-per-product table, the topology rebuild between products, and the crash-recovery jobs on design 048's development topology, after design 048's Task 048-2.
  Refs: Requirement 051-009
- [ ] T051-8 Add the `competitive-benchmark` workflow, publish the method and configurations, and run the reduced-profile pilot on the standard runner.
  Refs: Requirement 051-012, Requirement 051-014
- [ ] T051-9 Run the history loads and the pilots that choose each product's configuration and each variant's starting rate on the larger runner, and publish them.
  Refs: Requirement 051-004, Requirement 051-006, Requirement 051-015
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-10 Run the first full campaign and publish the dated report section and raw data.
  Refs: Success criterion 051-001, Requirement 051-011, Requirement 051-013
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-11 Publish the crash-recovery table of the first campaign.
  Refs: Success criterion 051-003
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-12 Write the reader's guide to the report and link it from `docs/README.md` and the roadmap.
  Refs: User story 051-01, User story 051-02
- [ ] T051-13 State, in every later campaign of 0.3.0, the ranking of Success criterion 051-002 with its ties.
  Refs: Success criterion 051-002
  Owner-gate: pending, the owner's row approving the runner budget of "Owner decisions needed" (money stays the owner's under Owner decision 2026-11-24-4)
- [ ] T051-14 Run the second tier on three machines and publish it.
  Refs: Requirement 051-003
  Owner-gate: pending, the owner's row providing the cloud account of "Owner decisions needed"

#### Follow-up tasks

None yet.

### Verification Plan

The design is verified by its own output. The reduced-profile pilot (Task 051-8) must produce a report section in the
format of Requirement 051-012 with every variant completing on every product, before money is spent. The no-op
adapter's ceiling must be at least ten times the highest sustained rate any product reaches, or the campaign stops. The
driver's rung rule, bisection, ties, rotation, censoring and recorded-end reading have unit tests against a simulated
product whose saturation point is known; the fault injector has a test that its recorded fault time matches the
process's death on the driver's clock to within 100 ms; the plan check has a test that refuses a plan of 351 minutes;
the architecture rule has a negative fixture per refusal.
