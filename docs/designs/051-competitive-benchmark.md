# A published benchmark against Temporal and Windmill

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

The owner's aim for release 0.3.0 is that the platform surpass Temporal and Windmill, and the owner asked for a
published benchmark of the same workflow shapes on the three products (Owner decision 2026-11-24-1, whose record quotes
the owner: "so that we surpass Windmill and Temporal"). This design is that benchmark, crash-recovery time included. It
changes no platform code.

One harness runs the same eight ladder variants (chained activities, fan-out and fan-in, signals, timers, sagas, scripts)
and one timer-burst procedure on this platform, on Temporal (self-hosted, PostgreSQL persistence) and on Windmill
(self-hosted). The load is open loop. The three products run in turn on the same declared machine, each with its own
PostgreSQL server of the same build, restored from an archive that already holds a history. The harness measures
throughput, the latency of a whole workflow, the time to recover from three kinds of crash, the work lost, stuck,
failed or repeated, and the processor time per transition. A report of a fixed format is published with its raw data,
its configuration and the command that reproduces it. A result where this platform loses is published as a loss.

Release 0.3.0 runs it on GitHub's free standard runner only (Owner decision 2026-11-25-1, "Nada pago"): a 4-vCPU
machine, a single PostgreSQL node per product, a history of 100,000 workflows, five repetitions (six for crashes). The
report is comparable (the three products on the same machine, shapes and load) and small, and says so: a figure from
it is a figure of that runner, never a product's limit, and no document claims to surpass a product beyond what a
published campaign at the release's commit shows (Requirement 051-017). Larger tiers are described in one section,
"Later tiers (unscheduled)", for the owner to decide on.

The hot-path remedies that the first draft of this design also carried are design 052. This harness's driver, this
platform's adapter and its history loader are also used by designs 049 and 053 (Requirement 051-014).

**Terms used below.**

- A *run* is one execution of a workflow; a *step* is one node of it, and an *activity* is a step that a worker
  executes. A *transition* is one step that reached its final state (completed, failed permanently or compensated),
  recorded durably (Requirement 051-008). A *shape* is a workflow defined once in product-neutral terms and implemented
  for each product; a *variant* is one of the eight shapes run as a ladder.
- A *rung* is one target arrival rate of the load, measured over a fixed window; a *ladder* is the sequence of rungs
  that finds a product's maximum sustained rate for a variant. *Open loop* means that starts are sent at their scheduled
  times whatever the system does; *coordinated omission* is the error of measuring latency from when a request was sent
  instead of when it was due. A *censored* latency is one not observed because the workflow had not ended, or failed.
- A *benchmark job* is one GitHub Actions job on a fresh runner; a product's *turn* is the part of a job in which it
  runs alone; a *campaign* is the set of jobs that produces one dated report section; a *pilot* is a job that chooses a
  product's configuration before a campaign; a *method version* is a numbered revision of `shapes.md` and the fixed
  constraints, recorded in every section.
- A *history* is the set of completed workflows a product's database holds before a turn. The *Temporal sidecar* is the
  Go process through which the Rust driver reaches Temporal's Go SDK. A product's *set* is the hardware threads and
  memory its containers may use. An *engine replica* is one execution engine process of this platform. *Work sent to a
  person* is a step this platform puts in review (`StepNeedsReview`); Temporal and Windmill have no such state, and the
  report shows that column for this platform only.

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
  in `tests/performance/competitive/shapes.md` (steps, payload sizes, expected outcome, transitions per workflow, the
  completion record per product, timeouts and retry budgets per product, idempotency mechanism per product, and the
  latency ceiling of Requirement 051-006) and implemented for each product in its own idiom: a chain of three activities
  and a chain of ten; a fan-out (one activity, then the same activity 100 times in parallel, then a join activity: 102
  transitions); a signal shape (start, wait for a named event sent 1 s after the start is acknowledged, one activity); a
  timer shape (a durable 1 s sleep, one activity); a saga (three activities, the third failing permanently, the first
  two compensated in reverse; expected outcome compensated, five transitions: three steps and two compensations, each compensation a node of its own); a
  script step in TypeScript and one in Python; and the timer-burst procedure (Requirement 051-016). Every activity's
  body is the same: it echoes its input and sleeps 20 ms. On this platform the signal shape waits with a correlated
  event wait with a lookback (`WaitEvent`), its own idiom for an event that may arrive before the wait, so the driver
  sends once; Temporal and Windmill wait for a signal and a resume as they document it, and `shapes.md` states each
  product's behaviour for an early arrival; where a product refuses one, the driver retries every 100 ms for at most 10 s
  and the refused sends per workflow are published. Temporal has no native script step: its two script variants are reported as
  "not run natively" (Requirement 051-013) and are not built. `shapes.md` carries the method version and the grammar of
  the execution records (Requirement 051-008), and every ceiling with its reasoning, all committed before any pilot runs.
- **Requirement 051-002 Products, versions, deployments and profiles.** The three products MUST run self-hosted from
  images pinned by digest, at the latest stable release of each on the day of Task 051-1's commit, frozen for the method
  version: this platform at the commit under test, its images built once per campaign from the release Dockerfile in release
  profile (Requirement 051-015); Temporal
  server with PostgreSQL persistence and SQL visibility; Windmill community edition. Each product runs its documented
  production profile: no development token issuer, audit on, telemetry exported to a collector inside its set, logs at
  information level, and TLS off on every client-to-server hop of all three alike (stated; a pilot may not turn off a
  product's telemetry or audit; the start probe of each turn reads each product's effective issuer, audit sink, log level and
  telemetry exporter and fails the turn otherwise, and `check_competitive_benchmark` refuses a compose file that sets a
  development issuer). Throughput and timer-burst jobs run each product's documented single-node deployment
  (its server processes as its documentation starts them on one host, one worker process); crash jobs run two server
  processes and two worker processes (Temporal's services each in their own process, its frontend behind the load
  balancer). The concurrency of one worker process (Temporal's activity slots, Windmill's workers per container, this
  platform's worker slots) is fixed per product by the pilot and published. The versions, digests, configurations
  (credentials as placeholders) and the tasks of designs 045, 048, 052 and 054 in this platform's commit are part of
  the report.
- **Requirement 051-003 One machine, shared fairly.** In one benchmark job the three products MUST run in turn on the
  same machine, each with its own PostgreSQL server of the same build, started from its own archive (Requirement
  051-015) and stopped with the product after its turn. The machine is GitHub's free standard runner for a public
  repository (`ubuntu-24.04`, Linux x64, 4 vCPU, 16 GB of memory, 14 GB of disk as GitHub documents it; Task 051-1
  records the page and its date); the benchmark workflow refuses to start when the repository is private
  (`github.event.repository.private`). The report records the runner image's version, and for every rung the CPU model
  and the processor steal time (`/proc/stat`), and a fixed calibration loop runs on a thread of the product's set between
  rungs (the product idle), measured against the job's first calibration. A throughput rung whose steal or calibration
  deviation exceeds 5%
  is run once more, at most once per job, and is marked if it exceeds it again; other job kinds mark such a rung and do
  not re-run it. The four vCPUs are two cores
  with two hardware threads each, so the sets are not by physical core: the driver and metrics collection get one
  thread, and the product with its database server the other three and 12 GB, split between them as the pilot chooses
  (one or two threads to the database), published; the Temporal sidecar runs in Temporal's set and is charged to it,
  since it is Temporal's client path, as a deviation line. CPU sets do not isolate the disk or the shared caches, which
  the report states.
- **Requirement 051-004 Best configuration within fixed constraints.** Each product MUST run in one configuration, the
  best the pilots find across the shapes (a per-shape change only where a pilot shows it matters), within the fixed
  constraints of Technical Plan, "Fixed constraints"; each constraint and each excluded feature is a deviation line, and
  the report publishes each product's tuning budget (the configurations tried and the runner minutes spent). Durability
  is never changed: at the start of each turn `pg_settings` shows the fixed constraints' values and
  `pg_db_role_setting` holds no setting for the product's roles and database and `pg_proc.proconfig` none on any
  function, both read at the start and at the end of each turn; each product's connection options are published, and
  `check_competitive_benchmark` refuses `options`, `-c` and `PGOPTIONS` in the compose files, the products'
  configurations and their connection options, and a `pg_service.conf` that carries `options`; at each turn's start and
  after each rung `run.sh` reads `/proc/<pid>/environ` of every process in each product container's control group
  and refuses `PGOPTIONS`, `PGSERVICE` and `PGSERVICEFILE` (an entrypoint's exports and a vendor image's own
  environment included), with tracing off and printing only the container's and the variable's names, never a value,
  since the job log is public (a fixture test confirms a refused value does not reach the output);
  the start probe refuses any connection startup parameter outside an allowlist (`application_name`, `search_path`,
  `client_encoding`, `TimeZone`, `DateStyle`, from `pg_stat_activity` and each product's published options); the
  products' roles are not superusers and hold no `SET` grant on `commit_delay`, checked through SQL (`synchronous_commit`
  has user context, so any role may set it, which only the scan below catches); and, with `pg_stat_statements.track =
  all` so that statements inside functions are seen, a scan of `pg_stat_statements` runs immediately before every
  reset and at the turn's end, so that every interval since the restore is covered (the start-up of connection pools and
  the waits between rungs included), and fails the turn, as it does when `pg_stat_statements_info.dealloc` changed
  during it, on a `SET` of
  `synchronous_commit` or `commit_delay` and on any `set_config` statement outside a per-product list that the pilot
  fixes and publishes, since `pg_stat_statements` stores the setting's name of a `set_config` call as a placeholder;
  each entry is a normalized statement text admitted only when every call site of the pinned vendor source (and of this
  platform's commit) that produces that text, all of them listed, issues it with a constant setting name that is not
  about durability, so that the list trusts only source no pilot can change (`pg_stat_statements_reset()` runs after
  each restore and at each rung's start, so the statement latency of Requirement 051-007 covers that rung alone). This platform runs with `SAGAT_DB_REQUIRE_SYNCHRONOUS_STANDBY=false` when design 048's
  task that adds the setting is in the commit, as a single node requires, stated.

### The load and the measurement

- **Requirement 051-005 Open-loop load.** The driver MUST start workflows on a precomputed schedule of exponentially
  distributed inter-arrival times at a target rate, from a seed fixed per (variant, rung, repetition) and shared by the
  three products of a job. An *arrival* is a scheduled start; the latency samples are the acknowledged arrivals
  scheduled within the rung's window. A start request times out after 5 s (this platform's adapter sets its client's
  timeout to 5 s and its retries to one attempt, the driver retrying); a start refused or timed out is retried with the
  same idempotency key under the product's documented advice until 10 s after its scheduled time; only an
  acknowledgement or an "already exists" answer makes a start acknowledged; an arrival not acknowledged by then is a
  failed arrival, and a workflow it may have created is an *orphan*, counted apart. The driver records every arrival's
  send lag (actual send minus scheduled time); a rung whose 99th percentile send lag exceeds 5 ms is marked.
- **Requirement 051-006 Rungs and the maximum sustained rate.** Each product's ladder for a variant MUST start at the
  rate the pilots chose for that product and variant and double at most five times, then bisect three times at the
  geometric midpoint between the last sustained and the first unsustained rung; if the first rung is not sustained the
  rate halves, at most three times, in place of doubling rungs; then comes the rung at half the maximum: at most
  1 + 5 + 3 + 1 = 10 rungs. A rung is *sustained* when the backlog of its own arrivals (acknowledged, not yet ended,
  counted by the driver) at the end of its window is not larger than at its start by more than 1% of the window's
  arrivals, at least 99% of the window's arrivals ended with the shape's expected outcome by the end of the drain, the
  99th percentile latency (censored values counted as infinite) is under the variant's ceiling, and the rung is not
  *worker-bound*: a rung whose arrival rate times the activity-seconds of one workflow (`shapes.md`; 2.04 s for the
  fan-out) reaches 80% of the product's worker slots ends the ladder as "worker-bound at" that rate, not as
  unsustained, and the pilot sizes the slots so that the expected maximum stays below it. Between rungs the driver
  waits, at most 3 minutes, until no run of the previous rung is open; a run still open when the wait ends is counted as
  stuck and excluded from later waits; the waits of a turn are capped at 12 minutes in all, and a rung that starts after
  the cap, or with work still open, is marked. A ladder that sustains every doubling rung reports "at least" its
  last rate. A product that sustains no rung reports "no rate sustained under the ceiling" with its lowest rung's
  latency. The *common rate* of a variant is half the lowest median maximum among the products that sustained a rung in
  the campaign's throughput jobs (whatever the CPU models, their mix stated), half the median maximum of the one product
  that sustained a rung when only one did, or, when none did, the lowest starting rate among the three products'
  ladders divided by 8, stated; a "worker-bound at" result counts as its last sustained rate; it is measured, for the chain of three (whose common
  rate also sets the crash rate) and the fan-out, in later jobs of its own, the other variants reporting their latency
  at half each product's maximum (Technical Plan, "A campaign").
- **Requirement 051-007 What is measured.** For each product, variant and rung: arrivals, failed arrivals, orphans,
  completions by outcome, transitions per second (transitions of workflows that ended within the window, over 300 s),
  the median and 99th percentile of workflow latency from the scheduled start (nearest rank; failed and open workflows
  counted as infinite, and the 99th percentile reported as "more than" the shortest censoring interval among the
  censored samples when censored values exceed 1%); the signal-to-resume latency (from the later of the event's
  acceptance and the workflow's wait, taken for Temporal as the completion time of the workflow task that blocked, a
  stand-in, stated, to the activity's start record); the processor seconds and peak memory (anonymous and shared, not
  page cache) of the product's processes and of its database server, its background processes included, over the same
  300 s; the WAL bytes of the turn (`pg_wal_lsn_diff` over the window, the server being the product's alone) and its
  commits per transition; the database connections used; the run table's rows and bytes at the rung's start; the CPU
  model, steal time, send lag and free disk space. For this platform's runs, also the time from a step's completion to
  the next step's lease (the *transition latency* design 053 uses, from this platform's journal) and the statement latency per table from
  `pg_stat_statements` (what design 049 uses). Every percentile carries its sample count; one from fewer than 2,000
  samples is marked.
- **Requirement 051-008 One unit of work, counted by the driver.** A *transition* is one step of the shape that reached
  its final state; a workflow's transitions are its shape's constant count (`shapes.md`), counted for each workflow that
  ended with the expected outcome, once. Every activity MUST append a record to an append-only file in its worker's set
  (one line per `O_APPEND` write: the product-neutral workflow identifier the driver assigned and passed in the input,
  the step, the iteration, the attempt, `begin`, `end` or `reconcile` (a reconciliation's record, which is
  neither a repeat nor a completion), and `CLOCK_REALTIME` in nanoseconds) before its body runs and
  after it ends, so that a `kill -9` of the worker leaves its records; reconciliations (Requirement 051-009) write
  records and are not transitions. A *repeat* is an execution of a (workflow, step, iteration) beyond the shape's
  expected count. The driver counts completions during a rung from each workflow's completion record (`shapes.md`: the
  last activity's end record, all of a fan-out's), read from those files as they grow, and never polls the products
  during a rung; script steps, whose containers have no network, are counted from the product's own records after the
  rung, so their drain runs its full 60 s and their backlog is computed afterwards. A workflow's end, for latency, is
  the commit time of the database transaction that recorded it (`track_commit_timestamp` on for all three,
  `pg_xact_commit_timestamp(xmin)` of the product's end row: this platform's run row, Temporal's execution row in its
  persistence, Windmill's completed job row), read after the drain and again at the end of the turn (a changed `xmin`, the pilot first confirming per product that it never rewrites its end row or naming another end
  record, fails
  the rung), one clock for all three; the report gives the
  distribution of that end minus the completion record, and a negative value fails the rung as a defect of the reader.
  Product internal units MUST NOT be compared.
- **Requirement 051-009 Crash recovery.** The harness MUST inject three faults into each product while it runs the chain
  of three at its common rate (one absolute rate for the three products; the crash jobs run after the throughput jobs
  that fix it): (a) `kill -9` of one of two server processes (this platform's engine replica; Temporal's history-service
  process, its frontend behind the load balancer staying up; Windmill's server process, "API only"); (b) `kill -9` of
  one of two worker processes, fired when the target worker's execution file shows at least four activities in flight,
  or after 30 s without, marked; (c) `docker kill -s KILL` of the database server's container and `docker start` of it
  on the same volume, a `CHECKPOINT` having run 60 s before the fault, the WAL replayed reported (the insert LSN read just before the kill
  minus the redo LSN of the last checkpoint, both through SQL). The killed process of
  faults (a) and (b) is restarted after a fixed 5 s. This platform's activities declare `reconcileWith` a reconciler
  registered by its worker whose verdict is "not applied", so an activity whose outcome is unknown is run again (a
  repeat) instead of going to a person. During its own pre-fault period the crash deployment MUST sustain the common
  rate by Requirement 051-006's rule, or the crash job fails, stated; a crash confirmation job runs each product's crash
  deployment at the common rate before the crash stage, and if any fails to sustain it, the crash stage runs at half the
  common rate for all three, stated as a deviation. The recovery is measured against a baseline of 20
  windows of 200 samples each (of 20 s each when the common rate is below 10 workflows per second): a window *passes*
  when its median and 90th percentile latency are within the baseline windows' worst value of the same statistic; the
  *recovery time* is from the fault to the start of the first three consecutive passing windows, capped at 10 minutes
  (its confirming windows counted inside the cap) and reported as "more than" the cap; every crash job also measures a null fault (the same procedure with nothing
  injected, before the real fault) and publishes its "recovery time" beside each fault's. For each fault the harness
  also measures the time until the last workflow in flight at the fault ends; for fault (c), the database's own time to
  accept connections again; and, after a 10-minute drain, each acknowledged start in one class only (ended, failed,
  stuck in review, stuck otherwise, lost), from the product's own record, as fractions of the acknowledged starts, with
  repeats as a fraction of activity executions, and the failed arrivals and orphans during the fault's recovery as
  fractions of its arrivals. The timeouts and client retry budgets that decide recovery are fixed per
  product in `shapes.md`, equal where the products can express them and a deviation line where they cannot (this
  platform's lease time to live, its shard lease, and its Go worker's report attempts, a constant of five).
- **Requirement 051-010 Repetitions, order and ranking.** Every throughput measurement MUST be repeated in five benchmark
  jobs on fresh runners, every common-rate measurement and timer burst in five, and every crash measurement in six.
  Within a job each product runs as one contiguous turn, the products' order rotating by repetition (the five throughput
  repetitions use the orders ABC, BCA, CAB, ACB and BAC; the six crash repetitions every order once). Products are ranked
  on the within-job ratio of each pair (one product's maximum sustained rate over the other's in the same job): a pair is
  tied unless all its ratios exceed 1.1, where the first leads, or all fall below 1/1.1, where the second leads; a pair
  is ranked only on the jobs where both maxima are numbers; a "worker-bound at" result ranks as "at least" its last sustained rate; a product with "at least" a rate is ranked
  only against products below that rate, and one with "no rate sustained" below every product that sustained one; when
  pairwise results do not order the three (one ties another that a third leads), the report states the pairs and no
  highest. Latency at the common rate is tied when the ranges of the two products' five 99th percentiles overlap, a "more than"
  value overlapping every value above its bound. The
  report shows each product's result by its position in the order and says that five or six repetitions cannot
  establish an order effect.
- **Requirement 051-011** is withdrawn (the first draft's cost per million transitions on a priced runner, which no
  0.3.0 run uses; "Later tiers (unscheduled)").

### The report and the rules around it

- **Requirement 051-012 Report format and reproduction.** Every job records the SHA-256s of `shapes.md`,
  `campaign.json`, `images.lock` and the driver's lock; the aggregation command refuses jobs whose digests differ from
  their method version's, unless its number changed, tested with a fixture, and refuses committed raw data above 50 MB
  per campaign. Each campaign MUST publish, through a pull request, a dated
  section of `tests/performance/reports/competitive-benchmark.md` in the format of Technical Plan, "The report", built by
  the driver's aggregation command from the jobs' artifacts, its charts as SVG files beside it; the per-job summaries
  (HdrHistogram logs, per-second counters, configurations with placeholders) are committed under
  `tests/performance/reports/raw/competitive-benchmark/<date>-<commit>/` with a manifest of SHA-256s, only the
  regenerable archives staying as expiring artifacts. One command, `tests/performance/competitive/run.sh`, reproduces a
  job on a fork. The method and the configurations MUST be published in the repository before the first campaign runs.
  A reproduction agrees when the median of its repetitions of each maximum sustained rate falls within the published range widened by one bisection step (a
  factor of 2^(1/8), about 1.09)
  on the same CPU model (on every CPU model when fewer than three repetitions share one), or each pair's within-job
  ratios fall within the published ratio range. Every later campaign of 0.3.0 states the ranking of Success criterion
  051-002 with its ties.
- **Requirement 051-013 Failures are reported.** A run that fails, a rung that did not sustain or was marked, a product
  that could not run a shape natively, and every deviation from this design MUST be stated in the report. A number that
  was not measured is never filled in.
- **Requirement 051-014 Isolation and reuse.** The competitor images, their adapters and the benchmark's scripts MUST
  live only under `tests/performance/competitive/` (the workflow itself in `.github/workflows/competitive-benchmark.yml`)
  and run only by hand. The driver (crate `sagat-competitive-driver`, outside the Cargo workspace, a path dependency on
  `sdks/rust`, the competitor adapters behind Cargo features so that this platform's adapter builds alone, without Go)
  offers its consumers the subcommands `capacity` (a bounded probe of the maximum sustained rate of the chain of three
  against an endpoint) and `load --rate --duration --endpoint` (open-loop load against any deployment), with a JSON
  output (a `schema_version` field, changed only by adding fields, pinned by a golden test) carrying the per-second
  completions, refused starts, terminal counts by outcome and run identifiers, the transition latency and the statement
  latency per table. A consumer deploys this design's Go worker for the chain of three with its record directory
  mounted where the driver reads it; `capacity` runs at most 10 rungs of 7 minutes, checked by the plan check. Its consumers are design 049 (its Tasks
  049-2, 049-8 and 049-14: the shapes, the driver, this platform's adapter and its history loader; its dated histories
  are design 049's own fixture) and design 053 (its Task 053-6, driving N-1 with the SDK of N-1's tree and N with N's);
  designs 048 and 052 use their own tools (the drill of `tests/performance/scalability-ha/dr.rs` and the open-loop mode
  of the same directory, whose figures never enter this report). The driver's lock and the Temporal sidecar's modules
  are audited in CI (`cargo audit`, and `govulncheck` at a pinned version), the driver's lock moving with `sdks/rust` when the CI step requires it, its digest part of the method
  version; no
  production crate depends on any of it.
- **Requirement 051-015 Histories and images, made once per campaign.** After the pilots, a campaign's first job MUST
  build this platform's images at the commit under test and load each product's history through the product itself, at
  a bounded concurrency in a closed loop, checked to hold exactly 100,000 completed chains of three, with the settings
  the pilots chose (those an archive freezes, such as Temporal's number of history shards, listed); it stores this
  platform's images (`docker save`) and each product's data directory as compressed archives, pinned by their SHA-256s,
  passed to later jobs only as the history job's outputs (`needs.<history job>.outputs`), as workflow artifacts. Every
  later job downloads them by exact name, loads the images and restores each archive before the product's turn,
  refusing an archive whose SHA-256 is not that output, any member outside the data
  directory and any symbolic link, and overwriting `postgresql.conf`, `postgresql.auto.conf` and `pg_hba.conf` from the
  repository before the server starts; then a `VACUUM (ANALYZE)` and a `CHECKPOINT` run. Every credential a product or
  its database uses is generated per campaign, never a development default; the archives are public, and the report
  says so. Such a history may fit in the database's memory, which the report states; and the ladder's own runs grow each
  product's tables with its own throughput, faster products more, which the report states as a deviation beside the run
  table's rows at each product's maximum.
- **Requirement 051-016 The timer-burst procedure.** For each product, 10,000 workflows are started over 5 minutes at a
  steady rate, each sleeping until one absolute instant 10 minutes after the first start; the procedure measures, from
  that instant, both the product's resume record and the next activity's start record of each workflow, their lateness
  percentiles, the time until every one has fired, and the timers not fired 10 minutes after the instant, reported as
  unfired; beside it the floor the worker slots set (10,000 × 20 ms over the slots). On this platform a graph timer
  sleeps a constant delay, so the absolute instant is its run's start delay, stated as a deviation ("a start delay, not
  a durable sleep").
- **Requirement 051-017 No claim beyond a comparable run.** A report, a release note or any other document MUST state that
  this platform leads, ties or trails a product only for the machine, shapes and rates a published campaign measured,
  citing its dated section, which counts only when run from `develop`, `main`, a `release/` branch or a `v` tag (its ref
  and commit recorded), and a release note only a campaign whose commit is an ancestor of the release's commit with only `docs/` and
  `tests/performance/reports/` changed between them (`git diff --name-only`); it MUST NOT
  claim to surpass Temporal or Windmill on throughput, latency or recovery time otherwise (Owner decision 2026-11-25-1).
  A free runner's figure is a figure of that runner, never a product's limit. `scripts/check-competitor-claims.py`, with
  a fixture for each refusal and each exemption, refuses a statement of fact (leads, ties, trails, surpasses, faster,
  slower) in `docs/`, `README.md` or `docs/releases/` that names Temporal or Windmill and cites no dated section, or, in
  a release note, cites a section whose commit fails the ancestry rule above; goals and quotes in `docs/designs/` and
  their reviews, and `docs/roadmap-assessment-2026-11-11.md`'s vendor figures, are exempt; it runs where the
  repository's other checkers run and in `release.yml`'s `version` job beside its notes check, so that no release
  publishes past it.
- **Requirement 051-018 Fair accounting.** This platform's per-run script containers MUST run under its set: during its
  turn the container daemon's default control-group parent (`/etc/docker/daemon.json`) is a slice whose CPU set is the
  product's, the daemon restarted between turns, and the processor time of the daemon and its shims during the turn is
  read by `run.sh` from the host's `/proc` (so that no metrics container needs a privilege) and charged to this platform (Windmill's sandbox runs inside its worker); the report gives the combined
  processor seconds of the product and its database server per thousand transitions beside each maximum.
- **Requirement 051-019 The disk is budgeted.** The pilot MUST measure the bytes each product writes per workflow of each
  variant; `run.sh` frees the runner's preinstalled tool caches at start and records the free space at every rung, and
  the driver's plan check refuses a plan whose projected disk use (the downloaded and decompressed archives and the
  loaded images included) exceeds the free space less 2 GB, as it refuses one
  whose worst-case minutes exceed 340.
- **Requirement 051-020 Nothing on the runner can reach the repository.** The benchmark workflow MUST have exactly the
  top-level block `permissions: contents: read` and no job-level `permissions:`, no `secrets: inherit`, no
  `${{ github.token }}`, no cache (the release workflow's cache rule below), no `${{ secrets.` reference at all, `persist-credentials: false` on its
  checkout, every `uses:` pinned to a
  40-character commit, no `${{ inputs. }}` or `${{ github.event. }}` inside a `run:` (inputs pass through `env:`), and
  refuse any ref but `develop`, `main`, `release/**` and `v*`. Every compose file under `tests/performance/competitive/`
  MUST have every service on networks declared `internal: true` (this platform's included), and none may be
  `privileged`, mount the container engine's socket, use the host's network, process or IPC namespace, add
  capabilities or devices, or run unconfined (Windmill's script variants then run with `nsjail` off if it needs any of
  these, stated), with one named exception: this platform's sandbox-runner service may mount the socket, its image
  built at the commit, `read_only`, `cap_drop: [ALL]` and `no-new-privileges`, and its per-run containers run with
  `--network none`, as a non-root user, from images of `images.lock`, which a check after each turn confirms from
  `docker inspect` of every run container; every third-party program, the Temporal sidecar and `omes`
  included, runs as a compose service under these rules, and the rule refuses a third-party binary started from
  `run.sh` on the host and a `docker run` or `docker create` in `run.sh` of any image not built at the commit, and
  applies the compose rules' list as flags to the rest (no `--privileged`, engine socket mount, host network, process or IPC
  namespace, added capability or device, or unconfined profile); the driver, built at the commit under test, runs its load as a container on each product's
  internal network, started by `run.sh` once per turn in the order `driver plan` prints, it and the metrics containers
  with an explicit `cgroup_parent` pinned to the driver's thread (their effective CPU sets confirmed from `docker
  inspect` at each turn's start, so that this platform's turn's default parent does not capture them), while its fault injector runs
  on the host in `run.sh`'s process (it needs the engine to kill and start containers), writing each fault's time as
  `CLOCK_REALTIME` into a host-only directory that no container can write (mounted read-only into the driver), where
  `run.sh` also gathers every file it uploads; the injector opens each execution record from a handle on the record root with
  `openat2` and `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS` (so that no part of the path is a link) and `O_NONBLOCK`,
  confirms with `fstat` on that open file that it is a regular file and reads at most 1 MB of it; the start probe checks that no container resolves an external name or reaches a public IPv4 or IPv6
  address, the pilots included (a Windmill step language that needs packages at run time is pre-resolved into an image
  built from pinned inputs, or excluded). Every image is named in one manifest (`images.lock`: name, digest taken from the vendor's
  registry, and the vendor's signature checked where the vendor signs), read by the compose files and by `run.sh`, every
  `SAGAT_RUNNER_IMAGE_*` of this platform set from it (this platform's own images exempt as "built at the commit under
  test"); after each turn `docker image ls --digests` is compared with it and an image not in it fails the job. `omes`
  is pinned to a commit with its `go.sum` and runs unprivileged. The raw data uploaded are an allowlist of regular files copied into the host-only directory,
  each opened the same way, checked by `fstat` on the open file, bounded in size and copied from that same open file (a link anywhere in
  the path or any other kind of file refused; no
  environment dumps, `docker inspect`, `docker compose config` or logs with statements; `log_statement` stays `none`).
  The release workflow (`.github/workflows/release.yml`) downloads its artifacts by name and refuses a duplicate file
  name, and no job of it restores a cache (`recovery`'s `rust-cache` removed; `cache: false` on `setup-go` and `setup-dotnet`, no
  `cache:` input on `setup-node`, `setup-python` or `setup-java` and `package-manager-cache: false` on `setup-node`; no
  `actions/cache`, `rust-cache` or `type=gha` anywhere), the one cache rule the benchmark workflow obeys too, checked by `scripts/tests/test_ci_workflow.py` and by the competitive rule `run.sh`
  runs first, a change of its own pull request before the first dispatch (Task
  051-0), since code with root on a runner of the default branch could otherwise plant a cache entry a tag's build
  restores.

### Success criteria

- **Success criterion 051-001** A first campaign is published with all variants and the procedure on all three products
  (except the variants a product cannot run natively, reported as such), the crash-recovery table with its null faults,
  and every deviation stated.
- **Success criterion 051-002** The report of every later campaign of release 0.3.0 states, for the chain of three and the
  fan-out, which product has the highest maximum sustained rate, and which the lowest 99th percentile latency at the
  common rate, with ties and unordered results as Requirement 051-010 defines them. The goal is that this platform
  leads both for both shapes; the criterion is that the answer is published either way, and claimed only as
  Requirement 051-017 allows.
- **Success criterion 051-003** In the crash-recovery table, this platform loses no acknowledged workflow under any of
  the three process crashes, its column of work sent to a person being zero by its reconciler's configuration, which the
  report states (storage loss is not tested on this runner: a killed
  container keeps the operating system's page cache), and its recovery times are reported beside the null fault's.

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
  platform's crash results, and the report names which of their tasks the commit under test has.

## Open Questions

Each was decided under the delegation of Owner decisions 2026-11-20-6 and 2026-11-24-4, with its reasoning.

1. **Which machine for 0.3.0?** GitHub's free standard runner of a public repository (this repository is public), the
   only one 0.3.0 may use: reproducible by anyone with a fork, and free. Its scale is small, which every figure states.
   Delegate.
2. **Which PostgreSQL major?** The one the platform's CI uses (16 today), pinned by digest, for all three. Delegate.
3. **What load generator?** A driver of our own in Rust, outside the workspace, with one adapter per product over its
   official client: this platform's Rust SDK; Temporal's Go SDK through the sidecar, whose own hop is measured with a
   no-op adapter and reported beside Temporal's latencies, which include it; Windmill's REST API. Temporal's `omes` runs
   once in the pilots as a cross-check. Delegate.
4. **Which language for activities?** Go for this platform and Temporal; for Windmill, the step language the pilot finds
   fastest among those that run without network, every candidate published. The script variants use TypeScript and
   Python on this platform and Windmill. Delegate.
5. **Which competitor features are allowed?** Their defaults and documented performance features that run inside their
   own set: Temporal's eager activity dispatch if faster; not its eager workflow start (it runs the worker in the
   starting client, which is the driver's set), not local activities (different semantics). Windmill's dedicated
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
    took to end the workflows in flight and to recover beside its null fault, and the fractions of acknowledged
    workflows lost, stuck or failed and of activities run twice.

## Technical Plan (Architecture)

**Layout.** Everything lives under `tests/performance/competitive/`, but the workflow:

| Path | What it holds |
|---|---|
| `shapes.md` | The method version; each variant and the procedure (steps, payloads, the activity body, outcomes, transitions, completion records, ceilings with their reasoning, timeouts and retry budgets per product, idempotency per product, early arrivals); the execution records' grammar |
| `images.lock` | Every image's name, digest and signature check |
| `driver/` | The driver's own workspace (`sagat-competitive-driver`): the schedule, the ladder state machine, the rotation, the plan check, the adapters behind features, the execution-record reader, the recorded-end reader, the fault injector, the recovery estimator, the ranking, the aggregation command; subcommands `plan --job-kind --variant --repetition --seed` (prints the turns and their order, the worst-case minutes and disk), `turn --product --variant --seed`, `crash --product --fault`, `burst --product`, `capacity`, `load`, `report <artifacts> [--common-rate <variant>]` |
| `sagat/`, `temporal/`, `windmill/` | For each product: the compose files (single-node, and two of each for crashes), its load balancer, the configuration with placeholders, the code of each variant, the history loader |
| `postgres/` | The PostgreSQL configuration of the fixed constraints, `pg_hba.conf`, and the server's compose service |
| `run.sh` | Prepares the machine (tool caches freed, the daemon's control-group parent, the sets), runs `check-architecture.py --only competitive` first, starts and stops containers, and calls the driver's subcommands; its arguments (job kind, variant, repetition, seed) are the matrix fields |
| `campaign.json` | The campaign's stages and matrices, read by the workflow with `fromJSON`, so that tuning never edits the workflow |
| `.github/workflows/competitive-benchmark.yml` | `workflow_dispatch` only; Requirement 051-020's rules; `ubuntu-24.04`; `timeout-minutes` 355 per job and 345 for the `run.sh` step, so that the upload of partial raw data still runs; a `concurrency` group `competitive-benchmark` without cancellation; stages in order by `needs:` (the history job, throughput, one matrix of common-rate and timer-burst jobs, the crash confirmation, crashes), each a matrix with `max-parallel` at most 12 and never two at once, so that the free plan's 20 concurrent jobs keep room for CI |

`check_competitive_benchmark` in `scripts/check-architecture.py`, with a flag `--competitive-root DIR` for its fixtures
in `scripts/tests/test_check_architecture.py`, checks the workflow's rules key by key with a line match (no YAML
library), its `concurrency` group with `cancel-in-progress: false`, `campaign.json` with the standard library's `json`
(every stage's `max-parallel` at most 12, each stage needing the one before), the compose files' rules, the refusal of `options`, `-c` and `PGOPTIONS` in the compose files, each product's
configuration and its published connection options, any `pg_service.conf` with `options`, `run.sh`'s `docker run` and `docker create`
lines, the image manifest, the release workflow's caches, and that no workspace member depends on `sagat-competitive-driver`;
`run.sh` calls it first as `python3 scripts/check-architecture.py --only competitive`; `--only competitive` is a new flag that runs this rule alone, without `cargo metadata`, as `--capability-root`
does; the workflow is required once `campaign.json` exists. A `ci.yml` step builds and tests the driver (`cargo test --locked --manifest-path
tests/performance/competitive/driver/Cargo.toml`, the default features and all of them) and builds the Go modules, on a
`competitive_driver` output of `scripts/select-test-targets.py` set for changes of `tests/performance/competitive/`,
`sdks/rust/` or `sdks/go/`, and on every full run, with a selector test.

**The machine and its sets** (Requirement 051-003). Only the product under test and its database server run during its
turn; the others are stopped.

| Component | Hardware threads | Memory limit |
|---|---:|---:|
| The product under test and its database server (the split the pilot chooses), the Temporal sidecar in Temporal's turn | 3 | 12 GB |
| The driver and metrics collection | 1 | 2 GB |

3 + 1 = 4 threads; 12 + 2 = 14 GB of 16, leaving 2 GB to the system.

**Fixed constraints** (each a deviation line where it departs from a product's documentation):

- One PostgreSQL configuration for the three servers, by a neutral rule: the server's defaults, with `shared_buffers` a
  quarter and `effective_cache_size` a half of the database server's memory share, `max_wal_size` 1 GB,
  `max_connections` 200, `shared_preload_libraries = 'pg_stat_statements'`, `track_commit_timestamp = on`, durability
  untouched (Requirement 051-004).
- Temporal's visibility store is SQL on the same server; its outcomes and close times are read from its persistence,
  not from visibility.
- Isolation, stated per row: activities of all three run unsandboxed in their workers; for the script variants this
  platform runs a container per run under its own set, and Windmill its sandbox (`nsjail`) where Requirement 051-020
  allows it.
- Timeouts and retry budgets as `shapes.md` fixes them (this platform's lease time to live, shard lease, worker report
  attempts and, once design 045 is built, per-activity heartbeat timeout; Temporal's heartbeat, start-to-close and
  client retry; Windmill's zombie-job timeout); this platform's activities declare their "not applied" reconciler.

**Turns and timing.** A rung is 60 s of warm-up, a 300 s window and up to 60 s of drain: 7 minutes. A throughput turn is
the archive's restore (at most 10 minutes), a ladder of at most 10 rungs (70 minutes), the waits between rungs (capped
at 12 minutes), a per-rung overhead of at most 0.5 minutes (the calibration loop, the `pg_stat_statements` scan and the
reads of commit times: 5), a per-turn overhead of at most 4 (the start probe, the image and `docker inspect` checks, and
the container daemon's restarts around this platform's turn) and a final `CHECKPOINT` and stop (2): at most 103 minutes.
A throughput job runs one variant: 3 × 103 + 20 of preparation + 7 for at most one re-run rung = 336 minutes. A
common-rate job runs one variant's common-rate rung first after each restore: 3 × (10 + 7 + 0.5 + 4 + 2) + 20, at most
92 minutes (23.5 rounded up to 24 per product). A
crash job is, per product, a restore (10), a warm-up (1), the baseline (at most 20 windows of 20 s, 7 minutes), the
null fault's recovery (at most 10), the in-flight trigger (at most 0.5), the fault's recovery (at most 10), the drain
(10), the stop (2) and the per-turn overhead (4), at most 55 minutes (54.5 rounded up), so 3 × 55 + 20 = 185 minutes; the crash confirmation job is
one such job. A
timer-burst job is, per product, a restore (10), the 10 minutes in which the starts fall and the sleep runs, at most 10
of firing, the stop (2) and the per-turn overhead (4), so 3 × 36 + 20 = 128 minutes. Before starting, the driver's plan check computes the worst case of the
job's plan and refuses one above 340 minutes (the `run.sh` step's timeout is 345, the job's 355 and GitHub's limit for a
hosted job 360) or beyond the disk budget, and at
each throughput job's start the no-op adapter's ceiling, delayed like each product's start latency, must reach ten
times the highest rate the job's plan can reach (its starting rate × 2⁵), or the job stops.

**A campaign.** The pilots are a job kind of their own, dispatched by hand once per method version (6 jobs of at most
340 minutes, at most 2,040 runner minutes and about 340 of wall time, before the first campaign: on a small history,
each product's configuration, database split, worker concurrency, starting rates, tuning budget, bytes per workflow and
each pilot job builds this platform's images itself). Then, per campaign: the history job
(this platform's images built once, at most 30 minutes, measured in the pilot, the archive within the disk budget; per
product at most 5 of start, 45 of loading and checking, and 10 of `VACUUM (ANALYZE)`, `CHECKPOINT`, archiving and a
short confirmation of its configuration: 30 + 3 × 60 + 20 = 230 minutes); 8 variants × 5 repetitions = 40 throughput
jobs (40 × 336 = 13,440); 2 variants × 5 = 10 common-rate jobs (10 × 92 = 920) and 5 timer-burst jobs (5 × 128 =
640), one matrix of 15 jobs; the crash confirmation job (185); 3 faults × 6 repetitions = 18 crash jobs (18 × 185 =
3,330). A campaign is at most 230 + 13,440 + 920 + 640 + 185 + 3,330 = 18,745 runner minutes, at no cost; in order, at
12 jobs at once, its wall time is at most 230 + ⌈40 / 12⌉ × 336 + ⌈15 / 12⌉ × 128 + 185 + ⌈18 / 12⌉ × 185 = 230 +
1,344 + 256 + 185 + 370 = 2,385 minutes, about 39.8 hours (the matrix of common-rate and timer-burst jobs counted at its longer job),
and a campaign is not dispatched while the release-candidate CI run is pending, a manual step of the release
procedure (`docs/runbooks/release.md`); one campaign is dispatched at a time, since a third dispatch would cancel a
pending one under the `concurrency` group.

**Faults per product** (Requirement 051-009).

| Fault | This platform | Temporal | Windmill |
|---|---|---|---|
| (a) server process | one of two engine replicas | one of two history-service processes | one of two server processes ("API only") |
| (b) worker | one of two worker processes | one of two worker processes | one of two worker processes |
| (c) database | its database server's container killed and started again on its volume, for all three |

**The report.** A dated section of `tests/performance/reports/competitive-benchmark.md`: the environment (runner image
and version, CPU models with each one's share of the jobs, steal or calibration per rung, kernel, PostgreSQL version and
configuration, the sets and memory limits with each product's database split, date, ref and commit, this platform's
design 045, 048, 052 and 054 tasks, competitor versions and digests, each product's tuning budget, the method version);
one table per variant (maximum sustained rate with resolution, the pairs' within-job ratios and ties, median and 99th
percentile at half the maximum and at the common rate, the 99th percentile at the maximum, combined processor seconds
per thousand transitions, WAL bytes per transition, rows of the run table at each rung; each cell the median of the
repetitions with its range and sample counts; marked rungs flagged); one latency-against-rate chart per variant (SVG);
the commit time against the completion record; the timer-burst table with the slot floor; the crash-recovery table with
the null faults; the results by position in the order; deviations, fixed constraints, failures and the pilots; the
scale of the runner beside every headline; how to reproduce.

**Order of the build.** Task 051-0 (the release workflow's hardening) comes before the first dispatch. The driver core
(Task 051-2) and this platform's adapter for the chain of three with its history loader (the first part of Task 051-3)
come before any competitor work, since designs 049 and 053 use them.

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
| The comparison is unfair to a competitor and is dismissed | Pilots and tuning budgets published; one neutral database configuration, durability checked; one budget for each product and its database; one commit clock for every end; production profiles; the method published before the first campaign; a correction is a new dated section. Inviting the vendors to read the method is the owner's act |
| A hosted runner is a shared virtual machine whose speed varies | Fresh runners, five repetitions, ranking on within-job ratios, CPU model and steal or calibration recorded, a noisy rung run again |
| One small machine hides the network cost, shares disk and caches, and may not show a product's strengths | Stated beside every figure; no claim beyond it (Requirement 051-017); "Later tiers (unscheduled)" |
| A competitor's licence forbids publishing benchmarks | Temporal is MIT-licensed and Windmill's community edition AGPL-3.0; neither restricts publishing measurements; Task 051-1 rechecks the pinned versions |
| The harness measures itself | Its own thread; the workers' records written locally; the send lag recorded; a delayed no-op adapter measures its ceiling; the sidecar's hop measured and reported, charged to Temporal as its client path |
| A worker's slots, not the product, set a maximum | Worker-bound rungs marked and not counted |
| One rung or product contaminates the next | A server per product; earlier runs drained before the next rung; rows recorded per rung; the common rate measured first after a restore |
| Third-party code on the runner reaches the repository or a release | Requirement 051-020; Task 051-0 |
| A job is killed by its time limit or fills the disk | The plan check refuses a worst case above 340 minutes or beyond the disk budget; the step's own timeout keeps the upload |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | n/a | The design measures; it changes no state handling |
| II. Modular domain, ports and adapters | Pass | Test code outside the workspace; the architecture rule keeps it there |
| III. External effects are at-least-once; sagas compensate | Pass | Repeats, stuck, failed and lost work are counted (Requirement 051-009) |
| IV. Compatibility is versioned and pinned | Pass | Images pinned in one manifest and checked after every turn; the platform's commit and the method version named |
| V. Code, tenant boundary and secrets are isolated | Pass | Requirement 051-020: read-only token, no cache, unprivileged internal containers, refs allowlisted; credentials per campaign; raw data by allowlist; the release workflow hardened (Task 051-0) |
| VI. Observability and audit are part of the feature | Pass | Every product in its production profile, audit and telemetry on (Requirement 051-002); the report states the environment, the method and the failures |
| VII. Events are redeliverable; inbox and outbox are the contract | n/a | Not touched |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Crash recovery measured for every product beside a null fault; censored latencies never hidden; a loss is published as a loss |
| Delivery Constraints | Pass | "Targets are not promises": every figure states its environment and scale; no paid run in 0.3.0 (Owner decision 2026-11-25-1); the driver's step and the claims checker run only where the constitution's gates already allow |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A harness for three products | A claim of surpassing needs the same shapes on the same machine | Vendor figures: different hardware, shapes and units |
| A Temporal sidecar | The driver is Rust and Temporal has no official Rust SDK | `omes` for Temporal: a different load model |
| One machine split by sets | Reproducible on a fork, free, no cloud account | Waiting for three machines |
| Restored history archives | Empty tables are not a workload, and loading in every job would push it past its limit | Fresh databases; loading per job |
| A recovery estimator with a null fault | A recovery time means something only beside what no fault gives | One window threshold |

## Alternatives Considered

- **Each vendor's own benchmark tool.** Different loops and units; one driver, with `omes` as a cross-check.
- **The development laptop.** Not valid evidence.
- **Closed loop, peak throughput.** Hides queueing; open loop with latency from the scheduled start.
- **A doubling ladder alone.** A factor-two resolution cannot rank; bisection gives a factor of 2^(1/8), about 1.09, and ties are reported.
- **Each product's recorded end time.** Stamped before the commit, by a different amount per product; the database's
  commit time is one clock for all three.
- **Polling the products for completions during a rung.** It loads each product's database differently; the workers'
  own records count them.
- **Interleaving the products rung by rung.** Twenty-nine switches per job do not fit its time; contiguous turns rotated
  by repetition keep the order visible.
- **One shared database server.** A data directory restores a whole server, and one server's settings cannot differ by
  product; a server per product, each stopped with it.
- **A fixed database-to-product split.** It favours whichever architecture fits it; one budget per product, its split
  chosen by its pilot.
- **Specifying the larger tiers in full now** (this design's second draft). Stale by the time they are paid for, and
  untested paths in the scripts; a short section instead.

## Non-Functional Requirements

- **Non-functional requirement 051-001 Reproducibility.** A new campaign at the same commit on the same runner class
  agrees with the published one as Requirement 051-012 defines it, or the report explains the difference.
- **Non-functional requirement 051-002 Cost.** The campaign costs no money; each job stays within the plan check's 340
  minutes and its disk budget, and a campaign within 18,745 runner minutes.
- **Non-functional requirement 051-003 Security.** Requirement 051-020.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

- [ ] T051-0 In a pull request of its own, make `.github/workflows/release.yml`'s `publish` download its artifacts by name (`pattern`, `merge-multiple: false`) and refuse a duplicate file name, and no job of it restore a cache (`recovery`'s `rust-cache` removed; `cache: false` on `setup-go` and `setup-dotnet`; no `cache:` input on `setup-node`, `setup-python` or `setup-java` and `package-manager-cache: false` on `setup-node`; no `actions/cache`, `rust-cache` or `type=gha`), with the workflow test of `scripts/tests/test_ci_workflow.py` extended to check both.
  Refs: Requirement 051-020
- [ ] T051-1 Write `tests/performance/competitive/shapes.md` (the method version, the shapes, the activity body, outcomes, transitions, completion records, the execution records' grammar, the ceilings with their reasoning, timeouts and retry budgets, idempotency, early arrivals, the timer mechanisms), committed before any pilot; check the licences of the pinned competitor versions; write `images.lock` by the version rule with the vendors' signatures checked; and record GitHub's page of the standard runner's specification and its date.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-006, Requirement 051-009, Requirement 051-020
- [ ] T051-2 Build the driver in its own workspace: the schedule with its seeds and send lag, the ladder state machine with the waits between rungs and the worker-bound rule, the rotation, the plan check (minutes and disk) with tests that accept a 340-minute plan within the disk budget and refuse 341 minutes and one beyond it, the execution-record reader, this platform's recorded-end reader (the commit time), the recovery estimator with the null fault and its test against a simulated 30 s recovery, the ranking with its test of unordered results, the aggregation command with SVG charts, the `capacity` and `load` subcommands with their JSON output, the transition latency and the statement latency per table, the no-op adapter; exclude it in the root `Cargo.toml`; add its lock to CI's `cargo audit`; add the `ci.yml` step with its selector output and test; add `check_competitive_benchmark` with its fixtures.
  Refs: Requirement 051-005, Requirement 051-006, Requirement 051-007, Requirement 051-008, Requirement 051-009, Requirement 051-010, Requirement 051-014
- [ ] T051-3 Write this platform's adapter and its history loader, the chain of three first (for designs 049 and 053), then one workflow definition per variant with Go activities declaring their "not applied" reconciler, the correlated-event wait of the signal shape and the start delay of the timer burst, in its production profile.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-008, Requirement 051-009, Requirement 051-015, Requirement 051-016
- [ ] T051-4 Write Temporal's compose files (single-node, and each service in its own process with two of each for crashes, the frontend behind a load balancer; PostgreSQL persistence and SQL visibility), its Go worker and the sidecar in Temporal's set with the sidecar's measured hop, its recorded-end reader from persistence, the variants but the scripts, its history loader, and `govulncheck` over the sidecar's and worker's modules in CI.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-008, Requirement 051-014, Requirement 051-015
- [ ] T051-5 Write Windmill's compose files (single-node, and two server processes behind a load balancer with two workers for crashes; its sandbox where Requirement 051-020 allows it), its flows for the variants, its recorded-end reader, the step-language pilot under the network rule, and its history loader.
  Refs: Requirement 051-001, Requirement 051-002, Requirement 051-004, Requirement 051-008, Requirement 051-015, Requirement 051-020
- [ ] T051-6 Write `run.sh` and `campaign.json`: the tool caches freed, the sets and the daemon's control-group parent, the images loaded and checked against `images.lock`, the archives restored and checked, the durability checks (`pg_db_role_setting` and `pg_proc.proconfig` at each turn's start and end, `PGOPTIONS`, `PGSERVICE` and `PGSERVICEFILE` in every product process's environment from `/proc` with no value printed and its fixture test, the scan before every reset and at the turn's end with the `dealloc` check, the per-product `set_config` list with every call site of each entry), the daemon's and shims' processor time from `/proc`, steal or calibration, the free space, the competitive rule run first, the driver's subcommands per job kind, the allowlist of uploaded raw data.
  Refs: Requirement 051-003, Requirement 051-004, Requirement 051-012, Requirement 051-015, Requirement 051-018, Requirement 051-019, Requirement 051-020
- [ ] T051-7 Add the fault injector on the host (the three faults, the 5 s restart, the in-flight trigger with its 30 s limit, the checkpoint before fault (c), the fault times in the host-only directory, each execution record and each uploaded file opened beneath the record root with `openat2` and `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS`, checked by `fstat` on the open file, read bounded and copied from that open file, with a test that a link in a directory part of the path is refused), the crash deployments' pre-fault check, the first step of every common-rate, confirmation and crash job (download the same run's earlier stages' summaries by pattern and run `driver report --common-rate <variant>`, the crash jobs also reading the confirmation's verdict), and the crash jobs after the throughput jobs; test the injector's recorded fault time against a child process's death seen through `pidfd` on the driver's monotonic clock, within 100 ms.
  Refs: Requirement 051-009
- [ ] T051-8 Add the `competitive-benchmark` workflow (Requirement 051-020's rules, the stages and matrices of `campaign.json`, the `concurrency` group, the refusal of a private repository and of other refs), publish the method and configurations, run the pilots once for the method version and publish them.
  Refs: Requirement 051-004, Requirement 051-012, Requirement 051-014, Requirement 051-019, Requirement 051-020
- [ ] T051-9 Run the first campaign, crash-recovery table with its null faults included, commit the per-job summaries and publish the dated report section.
  Refs: Success criterion 051-001, Success criterion 051-003, Requirement 051-012, Requirement 051-013
- [ ] T051-10 Write the reader's guide to the report at `tests/performance/reports/competitive-benchmark-guide.md` and link it from `docs/README.md` and `docs/roadmap.md`; run a campaign at the release-candidate commit of 0.3.0 before its release note is written, apart from the release-candidate CI run.
  Refs: User story 051-01, User story 051-02, Success criterion 051-002, Requirement 051-017
- [ ] T051-11 Write `scripts/check-competitor-claims.py` with its negative fixtures, run where the repository's other checkers run, add it as a step of `docs/runbooks/release.md`, and a row in `docs/engineering/architecture-principles.md`.
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
Requirement 051-012 with every variant a product runs natively completing on it before the first campaign runs. The
driver's ladder, bisection, waits, worker-bound rule, ties and unordered results, rotation, censoring, nearest-rank
percentiles, commit-time end and recovery estimator have unit tests against a simulated product whose saturation point
and recovery are known; the fault injector, the plan check and the architecture rule have the tests their tasks name;
the claims checker has a negative fixture per refusal.
