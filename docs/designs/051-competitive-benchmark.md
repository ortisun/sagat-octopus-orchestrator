# A published benchmark against Temporal and Windmill

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

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
latency of a whole workflow, the time to recover from three kinds of crash, the work lost, stuck, failed, repeated or
sent to a person, and the processor time per transition. A report of a fixed format is published with its raw data, its
configuration and the command that reproduces it. A result where this platform loses is published as a loss.

Release 0.3.0 runs it on GitHub's free standard runner only (Owner decision 2026-11-25-1, "Nada pago"): a 4-vCPU
machine, a single PostgreSQL node, a history of 100,000 workflows, five repetitions. The report is comparable (the three
products on the same machine, shapes and load) and small, and says so: a figure from it is a figure of that runner,
never a product's limit, and no document claims to surpass a product beyond what a published campaign at the release's
commit shows (Requirement 051-017). Larger tiers are described in one section, "Later tiers (unscheduled)", for the
owner to decide on.

The hot-path remedies that the first draft of this design also carried are design 052. This harness's driver, this
platform's adapter and its history loader are also used by designs 049 and 053 (Requirement 051-014).

**Terms used below.** A *run* is one execution of a workflow; a *step* is one node of it, and an *activity* is a step
that a worker executes. A *transition* is one completed step recorded durably (Requirement 051-008). A *shape* is a
workflow defined once in product-neutral terms and implemented for each product. A *rung* is one target arrival rate of
the load, measured over a fixed window. *Open loop* means that starts are sent at their scheduled times whatever the
system does; *coordinated omission* is the error of measuring latency from when a request was sent instead of when it
was due. A *censored* latency is one not observed because the workflow had not ended, or failed. A *benchmark job* is one
GitHub Actions job on a fresh runner; a *campaign* is the set of jobs that produces one dated report section.

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

### The workload

- **Requirement 051-001 Shapes.** The harness MUST run these eight ladder variants and one procedure, each defined once
  in `tests/performance/competitive/shapes.md` (steps, payload sizes, expected outcome, transitions per workflow,
  timeouts, retry policy, idempotency mechanism per product, and the 99th percentile latency ceiling of Requirement
  051-006) and implemented for each product in its own idiom: a chain of three activities and a chain of ten; a fan-out
  of one activity, 100 in parallel and a join; a signal shape (start, wait for a named signal sent 1 s after the start
  is acknowledged, one activity); a timer shape (a durable 1 s sleep, one activity); a saga (three activities, the third
  failing permanently, the first two compensated in reverse; expected outcome compensated, five transitions); a script
  step in TypeScript and one in Python; and the timer-burst procedure (Requirement 051-016). Every activity's body is
  the same: it echoes its input and sleeps 20 ms. For the signal shape, `shapes.md` states what each product does with a
  signal that arrives before the workflow waits for it (Temporal buffers it; this platform accepts it only if design
  054's buffered signals are in the commit, which the report states, and otherwise the driver retries the send every
  100 ms until it is accepted; Windmill's behaviour is checked by the pilot and stated). Temporal has no native script
  step: its two script variants are reported as "not run natively" (Requirement 051-013) and are not built.
- **Requirement 051-002 Products, versions and deployments.** The three products MUST run self-hosted from images pinned
  by digest, at the latest stable release of each on the day the method is published: this platform at the commit under
  test; Temporal server with PostgreSQL persistence and SQL visibility on the same PostgreSQL; Windmill community edition
  with its PostgreSQL. Throughput and timer-burst jobs run each product's documented single-node deployment (its server
  processes as its documentation starts them on one host, one worker process); crash jobs run two server processes
  behind the product's load balancer and two worker processes (Temporal's services then each in their own process). The
  concurrency of one worker process (Temporal's activity slots, Windmill's workers per container, this platform's
  worker slots) is fixed per product by the pilot and published as a configuration line. The versions, digests and
  configuration files are part of the report, and so are the tasks of designs 045, 048, 052 and 054 that the platform's
  commit contains.
- **Requirement 051-003 One machine, shared fairly.** In one benchmark job the three products MUST run in turn on the
  same machine and the same PostgreSQL build, each in its own database restored from its history (Requirement 051-015).
  The machine is GitHub's free standard runner for a public repository (Linux x64, 4 vCPU, 16 GB of memory, 14 GB of
  disk as GitHub documents it; Task 051-1 records the page and its date); the benchmark workflow refuses to start when
  the repository is private (`github.event.repository.private`), where the runner is smaller and its minutes are
  billed. The report records, for every rung, the runner's CPU model and the processor steal time (`/proc/stat`); a rung
  whose steal exceeds 5% is run once more, and is marked if it exceeds it again. The four vCPUs are two cores with two
  hardware threads each, so the sets are not by physical core: the driver (with metrics collection) gets one thread, and
  the product and its database together get the other three and 12 GB of memory, split between them as the pilot
  chooses for that product (one or two threads to the database), the split published as a configuration line; the
  Temporal sidecar runs in Temporal's set and is charged to it. CPU sets do not isolate the disk or the shared caches,
  which the report states. The database is a single PostgreSQL node with no standby, which the report states beside
  every figure.
- **Requirement 051-004 Best configuration within fixed constraints.** Each product MUST run in one configuration, the
  best the pilots find across the shapes (a per-shape change only where a pilot shows it matters), within the fixed
  constraints of Technical Plan, "Fixed constraints"; each constraint and each excluded feature is a deviation line of
  the report, and the report publishes each product's tuning budget (the configurations tried and the runner minutes
  spent), so that the effort is seen to be equal. No product's database session may change durability
  (`synchronous_commit`, `commit_delay`, `commit_siblings`, `fsync`, `full_page_writes`), checked by `SHOW` on each
  product's connections at the start of its turn; this platform runs with `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY=false`,
  the only setting a single node allows, which the report states.

### The load and the measurement

- **Requirement 051-005 Open-loop load.** The driver MUST start workflows on a precomputed schedule of exponentially
  distributed inter-arrival times at a target rate, from a fixed seed, independent of completions. An *arrival* is a
  scheduled start; the latency samples are the arrivals scheduled within the rung's window. A start request times out
  after 5 s; a start the product refuses or that times out is retried with the same idempotency key under its
  documented advice for at most 10 s; only an acknowledgement or an "already exists" answer makes a start acknowledged;
  an arrival not acknowledged after 10 s is a failed arrival.
- **Requirement 051-006 Rungs and the maximum sustained rate.** Each ladder variant MUST start at the rate the pilots
  chose for it and double at most six times, then bisect three times between the last sustained and the first
  unsustained rung; if the first rung is not sustained the rate halves, at most three times, in place of doubling rungs;
  then come the rung at half the maximum and the rung at the common rate: at most 1 + 6 + 3 + 1 + 1 = 12 rungs. A rung is
  *sustained* when the backlog (acknowledged starts not yet ended, counted by the driver) at the end of its window is not
  larger than at its start by more than 1% of the window's arrivals, at least 99% of the window's arrivals ended with
  the shape's expected outcome by the end of the drain, and the 99th percentile latency (censored values counted as
  infinite) is under the variant's ceiling. Every ceiling is fixed in `shapes.md`, with its reasoning, at a commit before
  any pilot runs, never after. A ladder that sustains every doubling rung reports "at least" its last rate. A product
  that sustains no rung reports "no rate sustained under the ceiling" with its lowest rung's latency. The *common rate*
  of a variant is half the lowest maximum among the products that sustained a rung, or, when only one did or none did,
  the lowest rung of the ladder, stated.
- **Requirement 051-007 What is measured.** For each product, variant and rung: arrivals, failed arrivals, completions by
  outcome, transitions per second, the median and 99th percentile of workflow latency from the scheduled start (nearest
  rank; failed and open workflows counted as infinite, and the 99th percentile reported as "more than" the drain's end
  when censored values reach 1%); the signal-to-resume latency (signal shape: from the later of the accepted send and the
  workflow's recorded wait to the activity's start record); the processor seconds and peak memory of the product's
  processes and of PostgreSQL, background processes apart; the WAL bytes of the product's database (attributed by
  database with `pg_walinspect`) and its commits, per transition; the database connections used; the CPU model, steal
  time and free disk space. For this platform's runs, also the time from a step's end record to the next step's start
  record (the *transition latency* design 053 uses) and the statement latency per table from `pg_stat_statements` (what
  design 049 uses). Every percentile carries its sample count; one from fewer than 2,000 samples is marked.
- **Requirement 051-008 One unit of work, counted by the driver.** A *transition* is one completed step of the shape; a
  workflow's transitions are its shape's constant count (`shapes.md`), counted for each workflow that ended with the
  expected outcome. Every activity MUST append its (workflow, step, iteration, attempt) to an append-only file in its
  worker's partition before its body runs and again after it ends, so that a `kill -9` of the worker leaves its record;
  the files are collected after the rung. A *repeat* is any execution of a (workflow, step, iteration) beyond the shape's
  expected count. The driver counts completions during a rung from the last activity's end records, read from those
  files as they grow, and never polls the products during a rung. A workflow's end, for latency, is its end time as the
  product records it, read after the rung, the clock and the statement of each product stated in the report (this
  platform's run end, Temporal's close time, Windmill's completion time); the report gives the distribution of the
  recorded end minus the last activity's end record, and a negative value is a defect of the reader, which fails the
  rung. Script steps, whose containers have no network, are counted from the product's own records. Product internal
  units MUST NOT be compared.
- **Requirement 051-009 Crash recovery.** The harness MUST inject three faults into each product while it runs the chain
  of three at the variant's common rate (one absolute rate for the three products, from the chain-of-three throughput
  medians of the same campaign on the same CPU model, so the crash jobs run after those): (a) `kill -9` of one of two
  server processes behind the product's load balancer; (b) `kill -9` of one of two worker processes, fired only when the
  target worker's execution file shows at least four activities in flight; (c) `docker kill -s KILL` of the database's
  container and `docker start` of it on the same volume, a `CHECKPOINT` having run 60 s before the fault, the WAL
  replayed reported. The killed process of faults (a) and (b) is restarted after a fixed 5 s, the same for every
  product. Before the fault the harness checks that the 99th percentile latency matches the throughput jobs' at that
  rate, within their range. For each fault it MUST measure: the time until the last workflow in flight at the fault
  ends; the recovery time, until three consecutive windows of 200 samples each have their median and 90th percentile
  latency within the worst value of the same statistic over the five windows before the fault, capped at 10 minutes and
  reported as "more than" the cap; for fault (c), the database's own time to accept connections again; and, after a
  10-minute drain, each acknowledged start classified from the product's own record as ended, failed, stuck (present
  and not ended) or lost (absent), with repeats and work sent to a person, each as a fraction of the acknowledged
  starts. The timeouts that decide recovery are fixed per product in `shapes.md` (this platform's lease time to live for
  each fault, stated), equal where the products can express them and a deviation line where they cannot.
- **Requirement 051-010 Repetitions, order and ranking.** Every throughput measurement MUST be repeated in five benchmark
  jobs on fresh runners, every timer burst in five, and every crash measurement in six. Within a job each product's
  ladder runs as one contiguous turn, the products' order rotating by repetition (over six crash repetitions every order
  of the three products is used once). Products are ranked on the within-job ratio of each pair (one product's maximum
  sustained rate over the other's in the same job): a pair is tied when the range of its five ratios includes 1 or lies
  within one bisection step of it. The report shows each product's result by its position in the order (the median and
  range for each position) and says that five or six repetitions cannot establish an order effect.
- **Requirement 051-011** is withdrawn (the first draft's cost per million transitions on a priced runner, which no
  0.3.0 run uses; "Later tiers (unscheduled)").

### The report

- **Requirement 051-012 Report format and reproduction.** Each campaign MUST publish, through a pull request, a dated
  section of `tests/performance/reports/competitive-benchmark.md` in the format of Technical Plan, "The report"; its raw
  data are an artifact of the workflow run, linked from the section. One command, `tests/performance/competitive/run.sh`,
  reproduces it on a fork. The method and the configurations MUST be published in the repository before the first
  campaign runs. A reproduction agrees when each maximum sustained rate falls within the published range widened by one
  bisection step on the same CPU model, or each pair's within-job ratios fall within the published ratio range.
- **Requirement 051-013 Failures are reported.** A run that fails, a rung that did not sustain, a product that could not
  run a shape natively, and every deviation from this design MUST be stated in the report. A number that was not
  measured is never filled in.
- **Requirement 051-014 Isolation and reuse.** The competitor images, their adapters and the benchmark workflow MUST
  live only under `tests/performance/competitive/` and run only by hand in the benchmark workflow. The driver, this
  platform's adapter and its history loader are used by design 049 (its Tasks 049-2, 049-8 and 049-14, with the
  statement latency per table of Requirement 051-007) and design 053 (its Task 053-6, with the transition latency);
  designs 048 and 052 use their own tools (the drill of `tests/performance/scalability-ha/dr.rs` and the open-loop mode
  of the same directory). The driver is outside the Cargo workspace (excluded in the root `Cargo.toml`), its lock file
  and the Temporal sidecar's modules are audited in CI, and no production crate depends on any of it.
- **Requirement 051-015 A history, loaded once per campaign.** Each product's database MUST hold 100,000 completed chains
  of three, loaded through the product itself once per campaign in a job of its own, then stored as a workflow artifact
  (the data directory as a compressed archive, pinned by its SHA-256) and restored before each product's turn in every
  later job of the campaign; after each restore a `VACUUM (ANALYZE)` and a `CHECKPOINT` run before the first rung. Such a
  history may fit in the database's memory, which the report states.
- **Requirement 051-016 The timer-burst procedure.** For each product, 10,000 workflows are started over 5 minutes at a
  steady rate, each sleeping until one absolute instant 10 minutes after the first start; the procedure measures, from
  that instant, both the product's resume record and the next activity's start record of each workflow (until every
  one has fired, and the lateness percentiles of both). It is not a ladder.
- **Requirement 051-017 No claim beyond a comparable run.** A report, a release note or any other document MUST state that
  this platform leads, ties or trails a product only for the machine, shapes and rates a published campaign measured,
  citing its dated section, and a release note only a campaign run at that release's release-candidate commit; it MUST
  NOT claim to surpass Temporal or Windmill on throughput, latency or recovery time otherwise (Owner decision
  2026-11-25-1). A free runner's figure is a figure of that runner, never a product's limit. The check is a step of each
  release note's publication (`docs/releases/`), repeated every time.
- **Requirement 051-018 Fair accounting of shared resources.** During a product's turn, autovacuum MUST be off on the
  tables of the other products' databases (restored after the turn); this platform's per-run script containers are
  created under the product's control group and CPU set (`--cgroup-parent`, `--cpuset-cpus`), and the processor time of
  the container daemon and its shims during the turn is measured and charged to this platform (Windmill's sandbox runs
  inside its worker); the report gives the combined processor seconds of the product and its database per thousand
  transitions beside each maximum.
- **Requirement 051-019 The disk is budgeted.** The pilot MUST measure the bytes each product writes per workflow of each
  variant; `run.sh` frees the runner's preinstalled tool caches at start, records the free space at every rung, and its
  plan check refuses a plan whose projected disk use exceeds the free space less 2 GB, as it refuses one whose worst-case
  minutes exceed 350.

### Success criteria

- **Success criterion 051-001** A first campaign is published with all variants and the procedure on all three products,
  the crash-recovery table, and every deviation stated.
- **Success criterion 051-002** The report of every later campaign of release 0.3.0 states, for the chain of three and the
  fan-out, which product has the highest maximum sustained rate, and which the lowest 99th percentile latency at the
  common rate, with ties as Requirement 051-010 defines them. The goal is that this platform leads both for both shapes;
  the criterion is that the answer is published either way, and claimed only as Requirement 051-017 allows.
- **Success criterion 051-003** In the crash-recovery table, this platform loses no acknowledged workflow under any of the
  three process crashes (storage loss is not tested on this runner: a killed container keeps the operating system's
  page cache), and its recovery times are reported for each. That it sends no work to a person follows from its
  activities being declared safe to retry, which the report says.

### User stories

- **User story 051-01** A buyer comparing the three products reads one page with the same shapes on the same machine and
  can rerun it.
- **User story 051-02** An operator sizing a deployment reads the latency and recovery figures for the shape that looks
  like their workload, at the stated scale.

## Goals

- A comparison anyone can reproduce on a fork, and that a competitor's engineer would accept as fair.
- The first published crash-recovery times of the three products.

## Non-Goals

- **No vendor-claim comparison**, no managed services, no tuning of the competitors' source code.
- **No paid run in 0.3.0** (Owner decision 2026-11-25-1); "Later tiers (unscheduled)".
- **No change to the platform.** Design 052 carries the hot-path changes; designs 045 and 048 carry what decides this
  platform's crash results (activity timeouts, the failover behaviour), and the report names which of their tasks the
  commit under test has.

## Open Questions

Each was decided under the delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4, with its reasoning.

1. **Which machine for 0.3.0?** GitHub's free standard runner of a public repository (this repository is public), the
   only one 0.3.0 may use: reproducible by anyone with a fork, and free. Its scale is small, which every figure states.
   Delegate.
2. **Which PostgreSQL major?** The one the platform's CI uses (16 today), pinned by digest, for all three. Delegate.
3. **What load generator?** A driver of our own in Rust, outside the workspace, with one adapter per product over its
   official client: this platform's Rust SDK; Temporal's Go SDK through a sidecar whose own hop is measured with a no-op
   adapter and reported beside Temporal's latencies, which include it; Windmill's REST API. Temporal's `omes` runs once in
   the pilots as a cross-check. Delegate.
4. **Which language for activities?** Go for this platform and Temporal; for Windmill, the step language the pilot finds
   fastest, every candidate published. The script variants use TypeScript and Python on this platform and Windmill.
   Delegate.
5. **Which competitor features are allowed?** Their defaults and documented performance features that run inside their
   own partition: Temporal's eager activity dispatch if faster; not its eager workflow start (it runs the worker in the
   starting client, which is the driver's partition), not local activities (different semantics). Windmill's dedicated
   workers are an Enterprise Edition feature, excluded, and the report says so. Delegate.
6. **Is beating the competitors a release gate?** No. Publishing the honest report is the gate (Success criteria 051-001
   and 051-003); leading is the goal (051-002), claimed only as Requirement 051-017 allows. Delegate (Owner decision
   2026-11-24-4, pull request 274).

## Owner decisions needed

None open for 0.3.0: the owner decided that nothing is paid (Owner decision 2026-11-25-1). "Later tiers (unscheduled)"
lists what a later release would ask the owner for.

## Critical User Journeys (CUJs)

### CUJ 1: Reading the comparison

*   **Given**: the published report of a campaign.
*   **When**: a reader opens the section of the chain of three.
*   **Then**: they see, for each product, the latency against the arrival rate at every rung, the maximum sustained rate
    with its resolution and the pairs' ratios and ties, the processor time per thousand transitions, the machine, its
    scale and the configuration, and links to the raw data and the command.

### CUJ 2: Reproducing it

*   **Given**: a fork of the repository, which must be public.
*   **When**: someone runs the `competitive-benchmark` workflow by hand on the fork, at a tagged commit.
*   **Then**: the jobs produce a dated section and raw data in the same format, which agree with the published campaign
    as Requirement 051-012 defines it, or the new section says why not.

### CUJ 3: A crash under load

*   **Given**: each product running the chain of three at the common rate.
*   **When**: the database's container is killed and started again on its volume.
*   **Then**: the report shows the database's time to accept connections and the WAL it replayed, how long each product
    took to end the workflows in flight and to hold its earlier latency, and the fractions of acknowledged workflows
    lost, stuck or failed, of activities run twice and of work sent to a person.

## Technical Plan (Architecture)

**Layout.** Everything lives under `tests/performance/competitive/`:

| Path | What it holds |
|---|---|
| `shapes.md` | Each variant and the procedure: steps, payload sizes, the activity body, expected outcome, transitions per workflow, latency ceiling with its reasoning, timeouts and retry policy per product, idempotency mechanism per product, the early-signal outcome |
| `driver/` | The Rust driver (its own workspace): schedule, adapters, histograms (HdrHistogram), per-second counters, the reader of the workers' execution files, the recorded-end readers, the fault injector, the plan check, the ratio ranking |
| `sagat/`, `temporal/`, `windmill/` | For each product: the compose files (single-node, and two-of-each for crashes) with images by digest and an internal network, its load balancer, the configuration, the code of each variant, the history loader |
| `postgres/` | The PostgreSQL configuration (the fixed constraints) and its single node |
| `run.sh` | The one command: prepares the machine, restores the histories, runs the turns, injects the faults, writes the raw data and the report section |
| `.github/workflows/competitive-benchmark.yml` | `workflow_dispatch` only, read-only permissions, no secrets, `timeout-minutes` 355 per job, `ubuntu-latest`, refused on a private repository; one dispatch runs a campaign as a matrix with `max-parallel: 12`, so that the free plan's 20 concurrent jobs keep room for CI |

A rule of `scripts/check-architecture.py` fails when the workflow has another trigger, a write permission, a `secrets.`
reference, a timeout above 355, no refusal of a private repository or a `max-parallel` above 12, when an image under
`tests/performance/competitive/` is not pinned by `@sha256:`, or when a workspace member depends on the driver.

**The machine and its sets** (Requirement 051-003). Only the product under test runs during its turn; the others are
stopped.

| Component | Hardware threads | Memory limit |
|---|---:|---:|
| The product under test and its database (the split the pilot chooses) | 3 | 12 GB |
| The driver and metrics collection | 1 | 2 GB |

3 + 1 = 4 threads; 12 + 2 = 14 GB of 16, leaving 2 GB to the system.

**Fixed constraints** (each a deviation line where it departs from a product's documentation):

- One PostgreSQL configuration for all three, by a neutral rule: the server's defaults, with `shared_buffers` a quarter
  and `effective_cache_size` a half of the database's memory share, `max_wal_size` 1 GB, `max_connections` 200, and
  durability untouched (Requirement 051-004); product-specific session settings that do not touch durability are
  allowed and listed.
- Temporal's visibility store is SQL on the same PostgreSQL.
- Isolation, stated per row: activities of all three run unsandboxed in their workers; for the script variants this
  platform runs a container per run under its own control group, and Windmill its sandbox (`nsjail`).
- Timeouts as `shapes.md` fixes them: this platform's lease time to live (and, once design 045 is built, its per-activity
  heartbeat timeout), Temporal's heartbeat and start-to-close timeouts, Windmill's zombie-job timeout; this platform's
  activities declare themselves safe to retry.

**Turns and timing.** A rung is 60 s of warm-up, a 300 s window and up to 60 s of drain, the backlog drained to empty
or cancelled through the product's API within that drain (recorded): 7 minutes. A product's turn is its history's
restore (at most 10 minutes), its ladder of at most 12 rungs (84 minutes) and, after it, a `CHECKPOINT` and a wait of at
most 2 minutes for autovacuum to go idle, recorded and charged to no product: at most 96 minutes. A throughput job is
three turns and 20 minutes of preparation: 3 × 96 + 20 = 308 minutes in the worst case. A crash job is, per product, a
restore (10), the measurement (25) and the database's restoration (10), so 3 × 45 + 20 = 155 minutes. A timer-burst job
is, per product, a restore (10) and the procedure (5 of starts, 10 of sleep, at most 10 of firing), so 3 × 35 + 20 = 125
minutes. Before starting, `run.sh` computes the worst case of its plan and refuses one above 350 minutes (GitHub's limit
for a hosted job is 360), or above the disk budget (Requirement 051-019).

**A campaign.** The history job (3 × at most 60 minutes of loading and 20 of preparation, 200); the pilots (6 jobs of at
most 350 minutes: each product's configuration, the database split, the worker concurrency, each variant's starting rate,
the bytes per workflow); 8 variants × 5 repetitions = 40 throughput jobs (40 × 308 = 12,320 minutes); 5 timer-burst
jobs (625); 3 faults × 6 repetitions = 18 crash jobs (2,790); in all 200 + 2,100 + 12,320 + 625 + 2,790 = 18,035 runner
minutes at most, about 25 hours of wall time at 12 jobs at once, at no cost.

**Faults per product** (Requirement 051-009).

| Fault | This platform | Temporal | Windmill |
|---|---|---|---|
| (a) server process | one of two engine replicas | one of two history-service processes | one of two server processes (on Windmill the server serves the API; the report marks this row "API only") |
| (b) worker | one of two worker processes | one of two worker processes | one of two worker processes |
| (c) database | the database's container killed and started again on its volume, for all three |

A crash job runs one fault, one repetition, for the three products, each restored from its history before its turn.

**The report.** A dated section of `tests/performance/reports/competitive-benchmark.md`: the environment (runner class,
CPU models with each one's share of the jobs, steal per rung, kernel, PostgreSQL version and configuration, the sets and
memory limits with each product's database split, date, this platform's commit and its design 045, 048, 052 and 054
tasks, competitor versions and digests, each product's tuning budget); one table per variant (maximum sustained rate
with resolution, the pairs' within-job ratios and ties, median and 99th percentile at half the maximum and at the common
rate, the 99th percentile at the maximum, combined processor seconds per thousand transitions, WAL bytes per
transition; each cell the median of the repetitions with its range and sample counts); one latency-against-rate chart
per variant; the recorded end against the last activity's end; the timer-burst table; the crash-recovery table; the
results by position in the order; deviations, fixed constraints, failures and the pilots; the scale of the runner beside
every headline; how to reproduce.

**Order of the build.** The driver (Task 051-2) and this platform's adapter for the chain of three with its history
loader (the first part of Task 051-3) come first, before any competitor work, since designs 049 and 053 use them.

## Later tiers (unscheduled)

What a later release could ask the owner for, not built in 0.3.0: a 64-vCPU larger runner (three equal PostgreSQL nodes
with Patroni and a synchronous standby as design 048's development topology, its Task 048-3; a history of 1,000,000
workflows; three repetitions; fault (c) as the primary's loss with a promotion; the cost per million transitions at the
runner's price), and three machines in one zone (the database, the product and the driver apart, with the network round
trip and the clocks' offset between hosts measured). At the prices of 2026-11-24 the first larger-runner campaign was
estimated at 11,640 minutes, US$ 1,885.68 at US$ 0.162 a minute, and each later one at 9,960 minutes, US$ 1,613.52; the
estimate is to be recomputed with the method then current. Each is the owner's decision (Tasks 051-12 and 051-13).

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| The comparison is unfair to a competitor and is dismissed | Pilots and tuning budgets published; one neutral database configuration with durability checked; one budget for each product and its database; fixed constraints and excluded features stated; the method published before the first campaign; a correction is a new dated section. Inviting the vendors to read the method is the owner's act |
| A hosted runner is a shared virtual machine whose speed varies | Fresh runners per repetition, five repetitions, ranking on within-job ratios, CPU model and steal recorded, a rung with high steal run again |
| One small machine hides the network cost, shares disk and caches, and may not show a product's strengths | Stated beside every figure; no claim beyond it (Requirement 051-017); "Later tiers (unscheduled)" |
| A competitor's licence forbids publishing benchmarks | Temporal is MIT-licensed and Windmill's community edition AGPL-3.0; neither restricts publishing measurements; Task 051-1 rechecks the pinned versions |
| The harness measures itself | Its own thread; the workers' records written locally, not sent; a no-op adapter measures its ceiling; the sidecar's hop is measured and reported |
| One rung or product contaminates the next | The backlog drained or cancelled within each rung's drain; autovacuum off on the stopped products' tables; a checkpoint and a wait between products, recorded; WAL attributed by database |
| A job is killed by its time limit or fills the disk | The plan check refuses a worst case above 350 minutes or beyond the disk budget |

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
| Delivery Constraints | Pass | "Targets are not promises": every figure states its environment and scale; no paid run in 0.3.0 (Owner decision 2026-11-25-1) |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A harness for three products | A claim of surpassing needs the same shapes on the same machine | Vendor figures: different hardware, shapes and units |
| A Temporal sidecar | The driver is Rust and Temporal has no official Rust SDK | `omes` for Temporal: a different load model |
| One machine split by sets | Reproducible on a fork, free, no cloud account | Waiting for three machines |
| Restored history archives | Empty tables are not a workload, and loading in every job spends a fifth of it | Fresh databases; loading per job |

## Alternatives Considered

- **Each vendor's own benchmark tool.** Different loops and units; one driver, with `omes` as a cross-check.
- **The development laptop.** Not valid evidence.
- **Closed loop, peak throughput.** Hides queueing; open loop with latency from the scheduled start.
- **A doubling ladder alone.** A factor-two resolution cannot rank; bisection gives one eighth, and ties are reported.
- **Polling the products for completions during a rung.** It loads each product's database differently; the workers'
  own records count them.
- **Interleaving the products rung by rung.** Thirty switches per job do not fit its time; contiguous turns rotated by
  repetition keep the order effect visible.
- **A fixed database-to-product split.** It favours whichever architecture fits it; one budget per product, its split
  chosen by its pilot.
- **Specifying the larger tiers in full now** (this design's second draft). Stale by the time they are paid for, and
  untested paths in the scripts; a short section instead.

## Non-Functional Requirements

- **Non-functional requirement 051-001 Reproducibility.** A new campaign at the same commit on the same runner class
  agrees with the published one as Requirement 051-012 defines it, or the report explains the difference.
- **Non-functional requirement 051-002 Cost.** The campaign costs no money; each job stays within the plan check's 350
  minutes and its disk budget, and a campaign within 18,035 runner minutes.
- **Non-functional requirement 051-003 Security.** The benchmark workflow holds no secret, has read-only permissions,
  runs only by hand and only on a public repository; the competitor containers reach no network beyond their internal
  one, checked by a probe at start.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T051-1 Write `tests/performance/competitive/shapes.md` (steps, payloads, the activity body, expected outcomes, transitions, latency ceilings with their reasoning, committed before any pilot, timeouts, idempotency per product, the early-signal outcome), check the licences of the pinned competitor versions, pin their images by digest by the version rule, and record GitHub's page of the standard runner's specification and its date.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-006, Requirement 051-009
- [ ] T051-2 Build the driver in its own workspace: open-loop schedule, latency from the scheduled start with censoring and nearest-rank percentiles, the reader of the workers' execution files, the recorded-end readers per product, HdrHistogram output, per-second counters, the no-op adapter, the plan check (minutes and disk), the within-job ratio ranking, the transition latency and the statement latency per table; exclude it in the root `Cargo.toml`; add its lock and the sidecar's modules to CI's audits; add the architecture rule.
  Refs: Requirement 051-005, Requirement 051-006, Requirement 051-007, Requirement 051-008, Requirement 051-010, Requirement 051-014
- [ ] T051-3 Write this platform's adapter and its history loader, the chain of three first (for designs 049 and 053), then the other variants as workflow versions with Go activities declared safe to retry and the script containers under the product's control group.
  Refs: Requirement 051-001, Requirement 051-008, Requirement 051-015, Requirement 051-018
- [ ] T051-4 Write Temporal's compose files (single-node, and each service in its own process with two of each for crashes; PostgreSQL persistence and SQL visibility; a load balancer), its Go worker and sidecar in Temporal's set with the sidecar's measured hop, the variants but the scripts, and its history loader.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-015
- [ ] T051-5 Write Windmill's compose files (single-node, and two server processes behind a load balancer with two workers for crashes; its sandbox on for the script variants), its flows for the variants, the step-language pilot and its history loader.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-004, Requirement 051-015
- [ ] T051-6 Write `run.sh`: the sets of Requirement 051-003, memory limits, archive restores, contiguous turns rotated by repetition, the 12-rung rule with bisection and halving, the drains and waits, autovacuum off on the stopped databases, WAL by database, steal and CPU model, the disk budget, the timer-burst procedure, raw data and the report section.
  Refs: Requirement 051-003, Requirement 051-006, Requirement 051-010, Requirement 051-012, Requirement 051-016, Requirement 051-018, Requirement 051-019
- [ ] T051-7 Add the fault injector (the three faults, the 5 s restart, the in-flight trigger, the checkpoint before fault (c)), the recovery windows, the database's restoration between products, and the crash jobs after the chain-of-three throughput jobs.
  Refs: Requirement 051-009
- [ ] T051-8 Add the `competitive-benchmark` workflow (dispatch only, the matrix with `max-parallel: 12`, the refusal of a private repository), publish the method and configurations, and run the history job and the pilots (each product's configuration, database split, worker concurrency, tuning budget, each variant's starting rate, the bytes per workflow), published.
  Refs: Requirement 051-004, Requirement 051-012, Requirement 051-014, Requirement 051-015, Requirement 051-019
- [ ] T051-9 Run the first campaign, crash-recovery table included, and publish the dated report section and raw data.
  Refs: Success criterion 051-001, Success criterion 051-003, Requirement 051-013
- [ ] T051-10 Write the reader's guide to the report and link it from `docs/README.md` and the roadmap; state, in every later campaign of 0.3.0, the ranking of Success criterion 051-002 with its ties; run a campaign at the release-candidate commit of 0.3.0 before its release note is written.
  Refs: User story 051-01, User story 051-02, Success criterion 051-002, Requirement 051-017
- [ ] T051-11 Add to the release-note procedure of `docs/releases/` the step that checks every sentence naming Temporal or Windmill against Requirement 051-017, run for each release note.
  Refs: Requirement 051-017
- [ ] T051-12 Run the larger-runner tier of "Later tiers (unscheduled)" with the method then current, and publish it.
  Refs: Requirement 051-003
  Owner-gate: pending, the owner's decision to pay for the larger runner (Owner decision 2026-11-25-1: nothing paid in 0.3.0)
- [ ] T051-13 Run the three-machine tier and publish it.
  Refs: Requirement 051-003
  Owner-gate: pending, the owner's decision to provide a cloud account (Owner decision 2026-11-25-1: nothing paid in 0.3.0)

#### Follow-up tasks

None yet.

### Verification Plan

The design is verified by its own output. The pilots (Task 051-8) must produce a report section in the format of
Requirement 051-012 with every variant completing on every product before the first campaign runs. The no-op adapter's
ceiling must be at least ten times the highest sustained rate any product reaches, or the campaign stops. The driver's
rung rule, bisection, ties by within-job ratios, rotation, censoring, nearest-rank percentiles and recorded-end reading
have unit tests against a simulated product whose saturation point is known; the fault injector has a test that its
recorded fault time matches the process's death on the driver's clock to within 100 ms; the plan check has tests that
refuse a plan of 351 minutes and one beyond the disk budget; the architecture rule has a negative fixture per refusal.
