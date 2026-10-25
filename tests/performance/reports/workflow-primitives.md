# Workflow primitive capacity measurements

Measurements for T014-042 on one shared laptop, written 2026-10-17. They are **observations, not
production ceilings and not latency objectives**. The first section says what the numbers do and do not
support; the raw summaries follow, failures and anomalies included.

## Latest diagnostic update — bounded worker paths

A frozen `025ce8d4` debug build completed 10,000 items at scope widths 1, 16 and 256 with zero
scheduler errors and 10,000 durable checkpoints in each case. This is an owner-role diagnostic,
not the final runtime-role capacity gate. See the raw update at the end; the historical results
and failures below remain unchanged.

## What this report concludes

- **Proven by assertion in the test (not only reported):** a suspended approval or timer owns no worker,
  thread or database connection (11,000 waits pending: 4 threads, 8 connections = the pool, 0 idle in a
  transaction, 0 worker sessions, no lease on any wait step); a tenant's collection slot limit was never
  exceeded in 279 scheduler rounds of two tenants with 48 slots of demand against 8, checked against two
  independent figures; the `maxConcurrency` of a collection was never exceeded in any run.
- **Update 2026-10-17: G461 is fixed for the scheduler transition, not for the worker report. See "After G461" at the end of this file; the numbers below are the measurements before the fix and are kept.**
- **A real defect was found and is not fixed here (as measured):** the work of one collection or loop transition grows
  linearly with the progress of the scope (G461), so a scope costs quadratic database work. Two red tests
  pin it. **The 10,000-item collection did not complete in a 10-minute budget at maxConcurrency 1, 16 or
  256.** A full-length run at maxConcurrency 256 then completed correctly in **6,815 s (1 h 54 min)**: 47
  times the 1,000-item run (144 s) for ten times the items, with the p50 of a transition rising from 44 ms
  in the first tenth to 678 ms in the last and the rows read per transition from 15,000 to 70,000. The
  10,000-iteration loop did not complete in 15 minutes. Collections and loops of up to 1,000 completed,
  with every bound respected, in 2 to 3 minutes.
- **Waiting is free of workers but not of scheduler work:** an idle scheduler tick costs 26 ms with no
  waits and 186 ms with 10,000 approvals pending (G468).
- **Chains** cost a flat 50 ms per hand-off to 1,000 segments; a conditional chain cannot exceed 127
  segments (G462).
- **No production ceiling is accepted by this report.** See "Ceilings".

## Environment

| Item | Value |
| --- | --- |
| Machine | Apple Mac15,9, Apple M3 Max, 16 logical cores (12 performance, 4 efficiency), 48 GiB RAM |
| OS | macOS 26.3 (25D125), Darwin 25.3.0 arm64 |
| Container runtime | Colima profile `sagat-remaining`, aarch64 VM with 4 CPUs, 6 GiB RAM, 30 GiB disk; Docker server 29.5.2. The VM also ran other agents' containers (Grafana, Jaeger, a kind cluster and other PostgreSQL servers) throughout |
| PostgreSQL | `postgres:16-alpine` (`sha256:721873c3...6080ea`), PostgreSQL 16.15 on aarch64-unknown-linux-musl. Container `sagat-014-perf-pg`, no CPU or memory limit, started with `-c shared_preload_libraries=pg_stat_statements -c pg_stat_statements.track=all -c pg_stat_statements.max=10000`, host port 55433 |
| PostgreSQL settings | `shared_buffers` 128MB, `max_connections` 100, `work_mem` 4MB, `effective_cache_size` 4GB, `synchronous_commit` on, `fsync` on, `max_wal_size` 1GB, `wal_level` replica, `random_page_cost` 4. Defaults of the image except the three `-c` options. The default test container (testcontainers, used only for the control run) starts the same image with `fsync` off |
| Build | `cargo test --release`, rustc 1.98.0 / cargo 1.98.0, `CARGO_PROFILE_DEV_DEBUG=0`, `CARGO_BUILD_INCREMENTAL=false`, `--locked` |
| Process | One test binary. The engine library, the scheduler (`batch` 200, `concurrency` 8, fair), the worker service and the PostgreSQL store run in it; there is no HTTP or gRPC hop, no TLS, no client network latency. The SQLx pool has 8 connections. `#[tokio::test]` uses a single-threaded runtime for the driver; database concurrency comes from the loader (32 callers) and from the scheduler's own `concurrency` |
| Commits | Series 1 (collections, loops, chains, tenants, first wait run): built from `e3e3979`. Series 2 (wait reruns, scaling, control, chain depth, A/B, idle-probe): built from the working tree that was committed unchanged as `d8b9d2e`, so its fragments print `Commit: e3e3979`; it differs from series 1 only in the wait workload (idle tick control points, scheduler lease TTL) and in how unfinished runs print their rates. The full-length run and the later commits add only the end-of-run load label. Baseline of the A/B: `42375f3`, built in a separate worktree |
| Concurrent load | Not controlled. Other agents ran builds, test suites and containers on the same host the whole time; the host load average at the start of each run is in the table below and in each fragment |

Host load average (1 minute) at the start of each run: waits (first) 40.8, tenants 17.0, chains 13.2 to
18.8, loops 10.7, collections 22.9 / 18.4, loop 10,000 11.0, collection 10,000 9.6, waits (dedicated rerun)
25.6, waits (control) 11.3, scaling runs 11 to 14. Single runs under that load have wide spread; the p99 of
anything here is the tail of one run.

## Commands

```sh
export CARGO_TARGET_DIR=/private/tmp/claude-501/targets/014-perf CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_INCREMENTAL=false
export DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock
docker run -d --name sagat-014-perf-pg -p 127.0.0.1:55433:5432 -e POSTGRES_PASSWORD=postgres postgres:16-alpine \
  -c shared_preload_libraries=pg_stat_statements -c pg_stat_statements.track=all -c pg_stat_statements.max=10000
docker exec sagat-014-perf-pg psql -U postgres -d template1 -c "CREATE EXTENSION pg_stat_statements"
cargo test -p sagat-tests --test perf_workflow_primitives --locked --release --no-run
export SAGAT_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:55433/postgres
export SAGAT_PERF_FRAGMENTS=<directory for the raw fragments>
BIN=$CARGO_TARGET_DIR/release/deps/perf_workflow_primitives-<hash>
$BIN --ignored --nocapture measure_durable_waits
$BIN --ignored --nocapture measure_cross_run
SAGAT_PERF_CHAIN_SEGMENTS=10,64,120 $BIN --ignored --nocapture measure_chain
SAGAT_PERF_CHAIN_SHAPE=direct SAGAT_PERF_CHAIN_SEGMENTS=1000 SAGAT_PERF_BUDGET_SECS=900 $BIN --ignored --nocapture measure_chain
SAGAT_PERF_LOOP_ITERATIONS=100,1000 $BIN --ignored --nocapture measure_sequential_loops
SAGAT_PERF_LOOP_ITERATIONS=10000 SAGAT_PERF_BUDGET_SECS=900 $BIN --ignored --nocapture measure_sequential_loops
SAGAT_PERF_COLLECTION_ITEMS=100 SAGAT_PERF_COLLECTION_CONCURRENCY=1,16,100 $BIN --ignored --nocapture measure_collection_fanout
SAGAT_PERF_COLLECTION_ITEMS=1000 SAGAT_PERF_COLLECTION_CONCURRENCY=1,16,256 $BIN --ignored --nocapture measure_collection_fanout
SAGAT_PERF_COLLECTION_ITEMS=10000 SAGAT_PERF_COLLECTION_CONCURRENCY=1,16,256 SAGAT_PERF_BUDGET_SECS=600 $BIN --ignored --nocapture measure_collection_fanout
$BIN --ignored --nocapture red_            # the two red tests: both fail, by design
SAGAT_PRIMITIVES_APPROVALS=500 SAGAT_PRIMITIVES_TIMERS=50 $BIN --ignored --nocapture measure_durable_waits   # also 2000/200, 5000/500
```

The databases of a run are dropped before the next one starts. Without `SAGAT_TEST_DATABASE_URL` the suite
starts its own container (the control run) and the statement tables print "unavailable".



## Results

### 1. Suspended waits: zero dedicated workers, threads and connections

The claim under test (SC-014-006) is that a suspended approval, timer or delay owns no worker, thread or
database connection. 10,000 approvals are started through the public publish/start path, opened by the
production scheduler, and 1,000 one-hour timers are added; the process is then observed. The assertions
are in the test, not only in this report: the process may not gain more than four threads, PostgreSQL
connections may not exceed the pool of eight plus the probe, no backend may be idle in a transaction, no
worker session may exist and every wait step must have no lease token, lease expiry or worker identity.
All of them held.

How to read the figures. The engine, scheduler and worker service run inside the test process, so its
thread count and resident memory are the engine's. PostgreSQL connections are the backends of the
benchmark database (`pg_stat_activity`); the process holds a pool of eight, so the count is bounded by the
pool and cannot by itself rise to 11,000. The evidence that nothing is held per wait is that the count
does not follow the number of waits (2 connections with no waits, 8 once the loading callers have filled
the pool, 8 with 11,000 waits pending and idle), that none is idle in a transaction, that threads stay at
four, and that the worker session and lease columns are empty.

**Waiting is not free for the scheduler.** The idle tick (a full scheduler tick that finds no work) grows
with the number of pending approvals. The 0-wait control row is the same scheduler over an empty database.

| Pending approvals | Idle tick, no waits (control), p50 / p95 | Idle tick with the approvals pending, p50 / p95 |
|---:|---:|---:|
| 500 | 27.4 / 42.6 ms | 41.8 / 46.6 ms |
| 2000 | 25.7 / 40.0 ms | 75.1 / 82.0 ms |
| 5000 | 26.9 / 44.1 ms | 142.1 / 146.1 ms |
| 10000 | 25.6 / 42.6 ms | 185.9 / 194.6 ms |

Populations other than 10,000 were run with `SAGAT_PRIMITIVES_APPROVALS` and `SAGAT_PRIMITIVES_TIMERS`
(timers at one tenth); each is a separate run, 2 to 3 minutes apart, on the same container. The curve is
close to 16 to 30 microseconds per pending approval per tick. If it stays linear (not measured beyond
10,000), a single scheduler replica with the default 500 ms idle pause would spend more than the pause on
each tick at roughly 20,000 to 30,000 pending approvals. That is an extrapolation, not a measurement (G468).
The approval-opening pass itself opened 270 to 410 approvals per second at 2,000 to 10,000 pending.

**Did the start path get slower?** The first wait run of this work (host load 17 to 41, dedicated
container) created approval runs at 65 per second with a p99 start transaction of 3.0 s; the reruns on the
same container at load 11 to 25 created them at 176 per second (p99 224 ms), and a control on the default
test container at load 12 at 160 per second. The earlier report of this file (commit `3545df0`, load
11.87) measured 257 per second. To separate host from code, the harness commit `42375f3` (135 commits
earlier) and this branch were run alternately on the same container with the same populations:

| Run | Build | Approval run creation | Approval opening | Approval start p50 | Host load (1 min) when the run started | Idle tick over 10,000 approvals + 1,000 timers, p50 |
|---|---|---|---|---|---|---|
| A1 | `42375f3` (the commit that introduced this harness) | 51.6 s, 194 runs/s | 36.5 s, 274 approvals/s | 154.9 ms | 25.5 | 118.7 ms |
| B1 | `d8b9d2e` (this branch) | 52.3 s, 191 runs/s | 34.8 s, 287 approvals/s | 164.8 ms | 11.3 | 198.0 ms |
| A2 | `42375f3` (the commit that introduced this harness) | 49.3 s, 203 runs/s | 36.3 s, 275 approvals/s | 156.0 ms | 8.1 | 118.1 ms |
| B2 | `d8b9d2e` (this branch) | 52.4 s, 191 runs/s | 34.6 s, 289 approvals/s | 165.4 ms | 7.6 | 197.6 ms |

See "Defects and anomalies" for the reading.

The line "approval scheduling delay (run created to request persisted)" in the table below is **not** a
scheduler latency. The harness starts all runs before it runs the scheduler, so the figure is the length
of the load phase. It is kept because it was in the earlier report; do not use it.


#### Durable waits and the tenant slot lock

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average at start: 11.63 14.23 11.49
- Build profile: release
- Commit: e3e3979
- PostgreSQL: Docker container on the same host
- SQLx pool: 8 connections
- Services: in-process; no client network latency

##### Configuration

- 10000 approval runs, loaded with 32 concurrent callers
- 1000 one-hour timer runs, loaded with 32 concurrent callers
- scheduler batch 200, scheduler concurrency 8, 50 idle ticks
- 64 collection runs contending for 8 tenant slots with 5 concurrent callers
- tenant usage row held for 100 ms to make lock contention observable

##### Latency observations

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| approval start transaction | 10000 | 178.8 ms | 203.5 ms | 223.6 ms | 463.7 ms |
| timer start transaction | 1000 | 194.7 ms | 228.3 ms | 270.3 ms | 343.7 ms |
| approval scheduling delay (run created to request persisted) | 10000 | 47047.1 ms | 56062.5 ms | 56610.3 ms | 57068.0 ms |
| timer scheduling delay (run created to wait persisted) | 1000 | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms |
| scheduler tick, no waits exist (control) | 10 | 25.6 ms | 42.6 ms | 42.6 ms | 42.6 ms |
| scheduler tick, 10000 approvals pending | 10 | 185.9 ms | 194.6 ms | 194.6 ms | 194.6 ms |
| scheduler tick, 10000 approvals + 1000 timers pending | 50 | 198.8 ms | 214.1 ms | 222.4 ms | 222.4 ms |
| collection admission accepted under tenant-slot contention | 8 | 122.9 ms | 163.1 ms | 163.1 ms | 163.1 ms |
| collection admission rejected at tenant capacity | 56 | 4.0 ms | 25.8 ms | 46.5 ms | 46.5 ms |

##### Throughput and storage

- Approval run creation: 56.7 s, 176 runs/s.
- Approval opening: 35.4 s, 282 approvals/s across 50 scheduler ticks.
- Timer run creation and wait persistence: 6.2 s, 161 runs/s.
- Observed relation growth: 4461 bytes per suspended approval and 2539 bytes per suspended timer. This is a delta over shared PostgreSQL relations and includes index allocation noise.
- Journal row bytes: 458 bytes per approval run and 394 bytes per timer run, measured with `pg_column_size` and excluding index/toast overhead.
- Test-process RSS: 15024128 bytes before loading and 26148864 bytes after measurement, a signed change of 11124736 bytes. PostgreSQL container RSS was not isolated in this run.

##### Correctness observations

- Resources while the waits are pending (the engine, scheduler and worker service run in this process; PostgreSQL connections are every backend of the benchmark database):
  - no waits: 4 threads, 14 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals started, none opened yet: 4 threads, 24 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals pending: 4 threads, 25 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals pending + 1000 timers waiting: 4 threads, 25 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - after 50 idle ticks over all waits: 4 threads, 25 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
- 8/64 collection admissions committed; 56 were rejected with the typed tenant-capacity error. Durable tenant usage and persisted iterations both equal 8.
- PostgreSQL exposed 5 lock waiters while the tenant usage row was deliberately held. Admission latencies therefore include a known 100 ms contention interval plus connection-pool and serialization delay.
- 11000 approval/timer steps are durably waiting with no lease token, lease expiry or worker identity. Active worker sessions: 0.
- Scheduler outcomes while opening and observing waits: 0 skipped, 0 fenced and 0 failed. Explicit client transaction retries: 0; every start and admission was submitted once.

##### Notes

- The first default-population attempt was rejected after reaching the platform's default 10,000 active-run namespace quota. The benchmark now grants and configures explicit headroom before loading; this preserves production admission behavior while isolating the wait measurement.
- Approval and timer runs use the supported publish/start/scheduler paths. Waiting consumes database rows and scheduler index scans; it does not allocate one process, connection, worker session or broker consumer per wait.
- The shared development host, local container, eight-connection pool and in-process services are not representative of a production cluster. Repeat on target hardware before sizing.
- The controlled slot test measures one tenant and one database. It proves exact enforcement under cross-run contention; it does not establish a cluster-wide throughput floor.
- No timeout, queue-delay or capacity ceiling is inferred beyond the populations and concurrency recorded above.


The three complete runs of the 10,000-approval workload, side by side:

| Run | PostgreSQL | Host load (1 min) when the run started | Approval run creation | Start transaction p50 / p99 | Idle tick, approvals pending | Idle tick, approvals + timers |
|---|---|---|---|---|---|---|
| First run (series 1) | dedicated, fsync on | 40.8 | 154.0 s, 65 runs/s | 312.9 ms / 3037.3 ms | n/a | 211.0 ms |
| Rerun (series 2, the table above) | dedicated, fsync on | 25.6 | 56.7 s, 176 runs/s | 178.8 ms / 223.6 ms | 185.9 ms | 198.8 ms |
| Control | default test container | 11.3 | 62.5 s, 160 runs/s | 198.7 ms / 248.1 ms | 182.0 ms | 198.3 ms |



### 2. Cross-run tenant concurrency bound

Two tenants share one scheduler, one database and one PostgreSQL instance. Each has a slot limit of 8
concurrent iterations (`tenant_collection_policies`). One tenant bursts six collection runs of 40 items,
each allowed all 8 slots, so demand is 48 against 8; the other runs one collection of 40 items. A worker
per tenant reports each leased task eight scheduler rounds after leasing it (the activity's duration), which
is what makes slots fill; a worker that answers instantly would never hold more than one slot per run. The
quiet tenant is also run alone as the control.

The bound is asserted after every scheduler round for both tenants against two independent figures: the
usage counter (`tenant_iteration_usage.active_iterations`) and the number of iterations that really hold a
slot (`collection_iterations.slot_held`). They must be at most the limit and equal each other.

The 64-run lock test in section 1 is the other half: 64 runs contend for 8 slots under a deliberately held
tenant usage row; exactly 8 are admitted and 56 get the typed capacity error.


#### Two tenants, one bursting

- Configuration: slot limit 8 per tenant; bursting tenant 6 runs x 40 items, each run `maxConcurrency` 8 (demand 48 against 8 slots); quiet tenant 1 run x 40 items; one worker per tenant that reports each leased task 8 rounds after leasing it; scheduler `tick()` rounds (batch 200, fair, concurrency 8).
- Slot bound asserted after every round for both tenants against the usage counter and against the count of iterations that really hold a slot (`slot_held`): peak counter 8 / holding 8 (bursting), 8 / 8 (quiet), limit 8. The two figures agreed in every round.
- Quiet tenant: finished in 87 rounds beside the burst, 87 alone (ratio 1.00); bound asserted: at most 4x.
- Bursting tenant: finished in 279 rounds; total wall 31.6 s. Scheduler actions skipped for a typed reason (tenant at capacity or revision conflict, no client retry involved): 1083.

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick while both tenants are active | 279 | 70.9 ms | 104.6 ms | 147.1 ms | 177.0 ms |
| scheduler tick, quiet tenant alone (control) | 87 | 42.6 ms | 69.3 ms | 89.0 ms | 89.0 ms |



### 3. Bounded collections

A `forEach` over `input.items` with a body of one activity and a `return`, published and started through
the public path, then driven by `advance_collection` (one durable scheduler transition per call, the same
operation the automatic pass uses) and a protocol 1.5 worker (poll up to 64, complete each task). Every
transition is timed. The tenant slot limit is set to the larger of 16 and `maxConcurrency` (collection
admission is closed until an operator records a tenant limit, so this is configuration). Items are
integers; worker outputs are `{"v": index}`. Peak active iterations are sampled every 8 transitions and
asserted against `maxConcurrency` and the tenant limit each time.

"Rows returned/affected" and "Buffer accesses" in the sampled-transition tables come from
`pg_stat_statements`: the difference between two reads around one transition, with the benchmark's own
query excluded. Rows and statements do not depend on host load; latency does. `ANALYZE` runs every 50
transitions to stand in for autovacuum (buffer counts were meaningless without it: the planner kept plans
for empty tables).

Findings, in the order they matter:

1. **Rows read per transition grow linearly with progress, so total database work is quadratic (G461).**
   Statement counts are constant (60 for a checkpoint, 65 for an admission) while rows returned go from
   1,536 at transition 166 to 7,014 at 1,992 in the 1,000-item run, and from 15,038 at 1,666 to 25,276 at
   4,998 in the 10,000-item runs. Two red tests pin it: `red_collection_transition_rows_do_not_grow_with_progress`
   and `red_loop_transition_rows_do_not_grow_with_progress` (both `#[ignore]`d, both fail today).
2. **The 10,000-item workload did not complete in 10 minutes at maxConcurrency 1, 16 or 256.** Each run was
   stopped by its budget, after 2,302 (concurrency 1), 3,120 (16) and 3,072 (256) items. The p50 of a
   transition went from about 32 ms in the first tenth to 86 ms (concurrency 1) and 115 to 118 ms (16 and
   256) in the last tenth reached. A full-length run
   at maxConcurrency 256 and no budget completed in 6,815 s; its fragment is the last of this section.
3. **Per-run progress is one transition per scheduler tick.** At 1,000 items the run took 125 to 188 s
   (5 to 8 items/s); concurrency 16 and 256 are 1.3 to 1.5 times faster than 1 and not 16 or 256 times,
   because the work is serialized in the transition, not in the activities.
4. **Concurrency bounds held.** Observed peaks: 1, 15 or 255 (sampled) against limits of 1, 16 and 256.
5. `enter` snapshots the whole item list in one transaction: 207 to 240 ms at 1,000 items and 3.6 s at
   10,000 (G464).

The `items/s` printed in the three unfinished 10,000-item fragments divides 10,000 by the wall time (the
harness did that when the run was unfinished; fixed since). The correct figures are checkpoints divided by
600 s: 2,302 / 600 = 3.8 (concurrency 1), 3,120 / 600 = 5.2 (16) and 3,072 / 600 = 5.1 (256) items/s. The
journal bytes per item of those three fragments are likewise divided by 10,000 and are meaningless; the
completed runs' 2,335 bytes per item is the figure to use.


#### 100 items, maxConcurrency 1

- Outcome: run status Succeeded, completed; wall 11.1 s, 202 scheduler transitions, 9.0 items/s, 18.3 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 1; configured maxConcurrency 1; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 22 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 24 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 2348 journal bytes per item (`pg_column_size`, no index/toast), 202 step rows, 100 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 28.1, 23.9, 21.8, 20.8, 20.9, 22.2, 21.6, 24.3, 24.6, 23.6

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 100 | 23.4 ms | 31.4 ms | 49.7 ms | 52.8 ms |
| scheduler: checkpoint one iteration | 100 | 22.8 ms | 28.9 ms | 40.2 ms | 47.2 ms |
| scheduler: enter (snapshot the items) | 1 | 49.6 ms | 49.6 ms | 49.6 ms | 49.6 ms |
| scheduler: finish the collection | 1 | 42.6 ms | 42.6 ms | 42.6 ms | 42.6 ms |
| scheduler: nothing to advance (waiting for a worker) | 100 | 10.8 ms | 14.5 ms | 19.2 ms | 20.1 ms |
| worker: complete one scoped task | 100 | 14.9 ms | 20.9 ms | 23.4 ms | 45.0 ms |
| worker: poll that leased tasks | 100 | 19.6 ms | 27.2 ms | 31.8 ms | 39.8 ms |
| worker: poll with nothing ready | 100 | 4.0 ms | 6.1 ms | 7.7 ms | 9.3 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| checkpoint | 16 | 21.0 ms | 60 | 186 | 185 |
| checkpoint | 32 | 26.2 ms | 60 | 234 | 214 |
| checkpoint | 48 | 23.6 ms | 60 | 282 | 296 |
| checkpoint | 64 | 20.7 ms | 60 | 330 | 217 |
| checkpoint | 80 | 23.4 ms | 60 | 378 | 235 |
| checkpoint | 96 | 20.8 ms | 60 | 426 | 258 |
| checkpoint | 112 | 23.0 ms | 60 | 474 | 254 |
| checkpoint | 128 | 22.2 ms | 60 | 522 | 278 |
| checkpoint | 144 | 27.2 ms | 60 | 570 | 324 |
| checkpoint | 160 | 24.3 ms | 60 | 618 | 304 |
| checkpoint | 176 | 23.0 ms | 60 | 666 | 311 |
| checkpoint | 192 | 23.6 ms | 60 | 714 | 333 |


#### 100 items, maxConcurrency 16

- Outcome: run status Succeeded, completed; wall 8.8 s, 202 scheduler transitions, 11.3 items/s, 22.9 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 15; configured maxConcurrency 16; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 25 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 25 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 2348 journal bytes per item (`pg_column_size`, no index/toast), 202 step rows, 100 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 24.4, 21.3, 25.9, 23.8, 27.4, 24.6, 22.4, 22.0, 23.5, 28.8

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 100 | 24.4 ms | 43.5 ms | 51.9 ms | 53.0 ms |
| scheduler: checkpoint one iteration | 100 | 25.1 ms | 33.8 ms | 47.9 ms | 61.2 ms |
| scheduler: enter (snapshot the items) | 1 | 45.9 ms | 45.9 ms | 45.9 ms | 45.9 ms |
| scheduler: finish the collection | 1 | 47.2 ms | 47.2 ms | 47.2 ms | 47.2 ms |
| scheduler: nothing to advance (waiting for a worker) | 7 | 11.2 ms | 16.9 ms | 16.9 ms | 16.9 ms |
| worker: complete one scoped task | 100 | 15.3 ms | 25.8 ms | 37.1 ms | 45.8 ms |
| worker: poll that leased tasks | 7 | 99.5 ms | 215.8 ms | 215.8 ms | 215.8 ms |
| worker: poll with nothing ready | 7 | 6.9 ms | 8.1 ms | 8.1 ms | 8.1 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 16 | 25.4 ms | 65 | 188 | 202 |
| checkpoint | 32 | 18.6 ms | 60 | 234 | 220 |
| admit | 48 | 22.5 ms | 65 | 284 | 299 |
| checkpoint | 64 | 25.0 ms | 60 | 330 | 212 |
| admit | 80 | 30.6 ms | 65 | 380 | 206 |
| checkpoint | 96 | 27.4 ms | 60 | 426 | 225 |
| admit | 112 | 34.6 ms | 65 | 476 | 228 |
| checkpoint | 128 | 19.2 ms | 60 | 522 | 256 |
| admit | 144 | 22.0 ms | 65 | 572 | 258 |
| checkpoint | 160 | 20.7 ms | 60 | 618 | 282 |
| admit | 176 | 26.4 ms | 65 | 668 | 275 |
| checkpoint | 192 | 47.9 ms | 60 | 714 | 303 |


#### 100 items, maxConcurrency 100

- Outcome: run status Succeeded, completed; wall 7.7 s, 202 scheduler transitions, 13.0 items/s, 26.3 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 97; configured maxConcurrency 100; tenant policy slots 100. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 25 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 25 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 2348 journal bytes per item (`pg_column_size`, no index/toast), 202 step rows, 100 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 21.9, 21.1, 21.6, 21.8, 21.2, 23.1, 23.3, 23.6, 21.9, 21.1

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 100 | 21.5 ms | 27.5 ms | 39.0 ms | 41.1 ms |
| scheduler: checkpoint one iteration | 100 | 22.8 ms | 36.2 ms | 45.1 ms | 49.8 ms |
| scheduler: enter (snapshot the items) | 1 | 48.1 ms | 48.1 ms | 48.1 ms | 48.1 ms |
| scheduler: finish the collection | 1 | 34.8 ms | 34.8 ms | 34.8 ms | 34.8 ms |
| scheduler: nothing to advance (waiting for a worker) | 1 | 12.1 ms | 12.1 ms | 12.1 ms | 12.1 ms |
| worker: complete one scoped task | 100 | 14.6 ms | 17.0 ms | 17.9 ms | 24.5 ms |
| worker: poll that leased tasks | 2 | 201.2 ms | 344.8 ms | 344.8 ms | 344.8 ms |
| worker: poll with nothing ready | 1 | 5.1 ms | 5.1 ms | 5.1 ms | 5.1 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 16 | 21.8 ms | 65 | 188 | 202 |
| admit | 32 | 22.5 ms | 65 | 236 | 238 |
| admit | 48 | 19.3 ms | 65 | 284 | 412 |
| admit | 64 | 23.3 ms | 65 | 332 | 189 |
| admit | 80 | 21.9 ms | 65 | 380 | 210 |
| admit | 96 | 21.9 ms | 65 | 428 | 210 |
| checkpoint | 112 | 22.9 ms | 60 | 650 | 291 |
| checkpoint | 128 | 23.7 ms | 60 | 666 | 292 |
| checkpoint | 144 | 24.1 ms | 60 | 682 | 301 |
| checkpoint | 160 | 26.7 ms | 60 | 698 | 302 |
| checkpoint | 176 | 26.8 ms | 60 | 714 | 314 |
| checkpoint | 192 | 22.8 ms | 60 | 730 | 320 |


#### 1000 items, maxConcurrency 1

- Outcome: run status Succeeded, completed; wall 187.9 s, 2002 scheduler transitions, 5.3 items/s, 10.7 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 1; configured maxConcurrency 1; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 22 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 34 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 2335 journal bytes per item (`pg_column_size`, no index/toast), 2002 step rows, 1000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 25.0, 33.5, 37.9, 39.4, 39.4, 45.1, 45.0, 40.1, 48.9, 50.6

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 1000 | 42.2 ms | 58.1 ms | 80.0 ms | 136.0 ms |
| scheduler: checkpoint one iteration | 1000 | 40.9 ms | 57.3 ms | 83.5 ms | 132.8 ms |
| scheduler: enter (snapshot the items) | 1 | 206.5 ms | 206.5 ms | 206.5 ms | 206.5 ms |
| scheduler: finish the collection | 1 | 88.5 ms | 88.5 ms | 88.5 ms | 88.5 ms |
| scheduler: nothing to advance (waiting for a worker) | 1000 | 25.7 ms | 38.5 ms | 42.3 ms | 55.8 ms |
| worker: complete one scoped task | 1000 | 27.8 ms | 39.7 ms | 54.6 ms | 128.5 ms |
| worker: poll that leased tasks | 1000 | 34.5 ms | 48.7 ms | 106.7 ms | 171.6 ms |
| worker: poll with nothing ready | 1000 | 5.5 ms | 9.9 ms | 14.5 ms | 19.9 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| checkpoint | 166 | 30.6 ms | 60 | 1536 | 335 |
| checkpoint | 332 | 35.3 ms | 60 | 2034 | 394 |
| checkpoint | 498 | 45.4 ms | 60 | 2532 | 453 |
| checkpoint | 664 | 41.7 ms | 60 | 3030 | 575 |
| checkpoint | 830 | 40.2 ms | 60 | 3528 | 689 |
| checkpoint | 996 | 42.1 ms | 60 | 4026 | 707 |
| checkpoint | 1162 | 43.5 ms | 60 | 4524 | 691 |
| checkpoint | 1328 | 44.9 ms | 60 | 5022 | 752 |
| checkpoint | 1494 | 41.9 ms | 60 | 5520 | 825 |
| checkpoint | 1660 | 56.5 ms | 60 | 6018 | 814 |
| checkpoint | 1826 | 48.6 ms | 60 | 6516 | 907 |
| checkpoint | 1992 | 46.3 ms | 60 | 7014 | 1009 |


#### 1000 items, maxConcurrency 16

- Outcome: run status Succeeded, completed; wall 125.5 s, 2002 scheduler transitions, 8.0 items/s, 16.0 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 15; configured maxConcurrency 16; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 38 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 39 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 2335 journal bytes per item (`pg_column_size`, no index/toast), 2002 step rows, 1000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 26.0, 29.1, 35.3, 38.3, 34.4, 37.4, 44.3, 49.7, 50.1, 51.3

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 1000 | 39.8 ms | 59.0 ms | 86.8 ms | 145.1 ms |
| scheduler: checkpoint one iteration | 1000 | 39.6 ms | 58.7 ms | 75.2 ms | 129.0 ms |
| scheduler: enter (snapshot the items) | 1 | 212.7 ms | 212.7 ms | 212.7 ms | 212.7 ms |
| scheduler: finish the collection | 1 | 81.1 ms | 81.1 ms | 81.1 ms | 81.1 ms |
| scheduler: nothing to advance (waiting for a worker) | 63 | 24.4 ms | 38.6 ms | 40.3 ms | 40.3 ms |
| worker: complete one scoped task | 1000 | 26.9 ms | 40.1 ms | 61.2 ms | 89.6 ms |
| worker: poll that leased tasks | 63 | 123.9 ms | 154.3 ms | 241.5 ms | 241.5 ms |
| worker: poll with nothing ready | 63 | 6.0 ms | 10.0 ms | 14.6 ms | 14.6 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 166 | 24.1 ms | 65 | 1538 | 327 |
| admit | 332 | 33.1 ms | 65 | 2036 | 438 |
| checkpoint | 498 | 41.3 ms | 60 | 2560 | 555 |
| checkpoint | 664 | 43.2 ms | 60 | 3046 | 638 |
| checkpoint | 830 | 34.8 ms | 60 | 3532 | 739 |
| admit | 996 | 34.9 ms | 65 | 4028 | 855 |
| admit | 1162 | 47.4 ms | 65 | 4526 | 929 |
| admit | 1328 | 46.7 ms | 65 | 5024 | 911 |
| checkpoint | 1494 | 47.3 ms | 60 | 5540 | 909 |
| checkpoint | 1660 | 48.4 ms | 60 | 6026 | 900 |
| admit | 1826 | 55.1 ms | 65 | 6518 | 1000 |
| admit | 1992 | 52.7 ms | 65 | 7016 | 972 |


#### 1000 items, maxConcurrency 256

- Outcome: run status Succeeded, completed; wall 143.6 s, 2002 scheduler transitions, 7.0 items/s, 13.9 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 255; configured maxConcurrency 256; tenant policy slots 256. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 39 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 33 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 2335 journal bytes per item (`pg_column_size`, no index/toast), 2002 step rows, 1000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 28.6, 28.1, 75.3, 36.8, 51.5, 36.8, 46.7, 45.3, 48.7, 42.5

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 1000 | 38.7 ms | 71.2 ms | 105.8 ms | 233.9 ms |
| scheduler: checkpoint one iteration | 1000 | 44.8 ms | 148.8 ms | 220.7 ms | 334.7 ms |
| scheduler: enter (snapshot the items) | 1 | 240.6 ms | 240.6 ms | 240.6 ms | 240.6 ms |
| scheduler: finish the collection | 1 | 80.6 ms | 80.6 ms | 80.6 ms | 80.6 ms |
| scheduler: nothing to advance (waiting for a worker) | 4 | 24.5 ms | 34.5 ms | 34.5 ms | 34.5 ms |
| worker: complete one scoped task | 1000 | 26.3 ms | 49.4 ms | 115.5 ms | 255.5 ms |
| worker: poll that leased tasks | 16 | 353.4 ms | 473.8 ms | 473.8 ms | 473.8 ms |
| worker: poll with nothing ready | 4 | 5.4 ms | 17.3 ms | 17.3 ms | 17.3 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 166 | 25.8 ms | 65 | 1538 | 289 |
| checkpoint | 332 | 29.2 ms | 60 | 2394 | 444 |
| checkpoint | 498 | 123.1 ms | 60 | 2560 | 485 |
| admit | 664 | 33.2 ms | 65 | 3032 | 535 |
| checkpoint | 830 | 89.3 ms | 60 | 3916 | 689 |
| checkpoint | 996 | 179.0 ms | 60 | 4082 | 719 |
| admit | 1162 | 37.9 ms | 65 | 4526 | 668 |
| checkpoint | 1328 | 43.3 ms | 60 | 5438 | 749 |
| checkpoint | 1494 | 46.9 ms | 60 | 5604 | 811 |
| admit | 1660 | 46.9 ms | 65 | 6020 | 880 |
| checkpoint | 1826 | 39.3 ms | 60 | 6864 | 925 |
| checkpoint | 1992 | 52.0 ms | 60 | 7030 | 954 |


#### 10000 items, maxConcurrency 1 (BUDGET EXHAUSTED, NOT COMPLETED)

- Outcome: run status Running, NOT completed within the time budget; wall 600.1 s, 4606 scheduler transitions, 16.7 items/s, 7.7 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 1; configured maxConcurrency 1; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 22 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 43 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 537 journal bytes per item (`pg_column_size`, no index/toast), 4607 step rows, 2302 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 31.6, 36.6, 41.4, 46.8, 52.7, 56.4, 65.2, 73.4, 79.9, 86.3

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 2303 | 54.9 ms | 115.8 ms | 194.5 ms | 593.6 ms |
| scheduler: checkpoint one iteration | 2302 | 54.9 ms | 114.8 ms | 199.2 ms | 444.4 ms |
| scheduler: enter (snapshot the items) | 1 | 3601.0 ms | 3601.0 ms | 3601.0 ms | 3601.0 ms |
| scheduler: nothing to advance (waiting for a worker) | 2303 | 43.0 ms | 84.1 ms | 136.3 ms | 400.5 ms |
| worker: complete one scoped task | 2303 | 30.6 ms | 65.3 ms | 109.6 ms | 219.6 ms |
| worker: poll that leased tasks | 2303 | 36.4 ms | 76.4 ms | 142.8 ms | 324.2 ms |
| worker: poll with nothing ready | 2303 | 4.3 ms | 8.3 ms | 15.3 ms | 33.5 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| checkpoint | 1666 | 49.4 ms | 60 | 15036 | 1474 |
| checkpoint | 3332 | 73.2 ms | 60 | 20034 | 1984 |


#### 10000 items, maxConcurrency 16 (BUDGET EXHAUSTED, NOT COMPLETED)

- Outcome: run status Running, NOT completed within the time budget; wall 600.0 s, 6250 scheduler transitions, 16.7 items/s, 10.4 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 15; configured maxConcurrency 16; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 44 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 55 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 729 journal bytes per item (`pg_column_size`, no index/toast), 6250 step rows, 3120 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 32.4, 39.7, 45.9, 54.2, 61.4, 69.5, 80.4, 90.2, 101.5, 114.8

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 3129 | 65.8 ms | 115.2 ms | 122.1 ms | 132.5 ms |
| scheduler: checkpoint one iteration | 3120 | 65.6 ms | 115.0 ms | 121.3 ms | 140.7 ms |
| scheduler: enter (snapshot the items) | 1 | 3254.7 ms | 3254.7 ms | 3254.7 ms | 3254.7 ms |
| scheduler: nothing to advance (waiting for a worker) | 195 | 52.8 ms | 102.1 ms | 107.0 ms | 111.1 ms |
| worker: complete one scoped task | 3120 | 35.0 ms | 51.8 ms | 55.6 ms | 61.9 ms |
| worker: poll that leased tasks | 195 | 114.6 ms | 130.4 ms | 136.5 ms | 139.3 ms |
| worker: poll with nothing ready | 195 | 4.6 ms | 6.4 ms | 8.3 ms | 8.8 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 1666 | 52.2 ms | 65 | 15038 | 1476 |
| admit | 3332 | 71.6 ms | 65 | 20036 | 2024 |
| admit | 4998 | 97.1 ms | 65 | 25034 | 2583 |


#### 10000 items, maxConcurrency 256 (BUDGET EXHAUSTED, NOT COMPLETED)

- Outcome: run status Running, NOT completed within the time budget; wall 600.1 s, 6348 scheduler transitions, 16.7 items/s, 10.6 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 255; configured maxConcurrency 256; tenant policy slots 256. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 55 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 60 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 730 journal bytes per item (`pg_column_size`, no index/toast), 6348 step rows, 3072 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 31.7, 39.6, 45.7, 54.3, 62.7, 71.9, 82.8, 94.9, 105.1, 118.5

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 3275 | 67.1 ms | 121.0 ms | 130.3 ms | 190.6 ms |
| scheduler: checkpoint one iteration | 3072 | 67.9 ms | 117.4 ms | 123.0 ms | 142.5 ms |
| scheduler: enter (snapshot the items) | 1 | 3241.0 ms | 3241.0 ms | 3241.0 ms | 3241.0 ms |
| scheduler: nothing to advance (waiting for a worker) | 12 | 48.8 ms | 104.6 ms | 104.6 ms | 104.6 ms |
| worker: complete one scoped task | 3072 | 35.7 ms | 53.6 ms | 57.8 ms | 81.5 ms |
| worker: poll that leased tasks | 48 | 344.4 ms | 404.2 ms | 431.9 ms | 431.9 ms |
| worker: poll with nothing ready | 12 | 4.2 ms | 7.9 ms | 7.9 ms | 7.9 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 1666 | 45.0 ms | 65 | 15038 | 1409 |
| checkpoint | 3332 | 80.6 ms | 60 | 20538 | 1933 |
| checkpoint | 4998 | 96.2 ms | 60 | 25276 | 2339 |


#### Full-length run: 10,000 items, maxConcurrency 256, no budget

Started 07:42:54 at host load 6.45 and ended 09:36:32. During the first three minutes the formatting, lint and test gates of this task ran on the same host, and a 45 s idle-tick probe ran at about 08:00; host load stayed between 5 and 13 otherwise. The run used the `e3e3979` build with the later label and formula fixes (the harness binary of series 2). Its time-per-tenth series is the cleanest picture of G461.


#### 10000 items, maxConcurrency 256

- Outcome: run status Succeeded, completed; wall 6814.7 s, 20002 scheduler transitions, 1.5 items/s (items checkpointed per second), 2.9 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 255; configured maxConcurrency 256; tenant policy slots 256. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 22 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 118 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3 (sampled every 256 transitions).
- Durable size: 2334 journal bytes per item (`pg_column_size`, no index/toast), 20002 step rows, 10000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 44.4, 64.5, 96.9, 139.1, 208.7, 256.7, 333.0, 402.9, 530.7, 678.2

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 10000 | 234.4 ms | 724.7 ms | 970.4 ms | 5987.1 ms |
| scheduler: checkpoint one iteration | 10000 | 237.1 ms | 688.9 ms | 887.8 ms | 2253.9 ms |
| scheduler: enter (snapshot the items) | 1 | 3198.4 ms | 3198.4 ms | 3198.4 ms | 3198.4 ms |
| scheduler: finish the collection | 1 | 750.4 ms | 750.4 ms | 750.4 ms | 750.4 ms |
| scheduler: nothing to advance (waiting for a worker) | 40 | 229.6 ms | 674.4 ms | 851.3 ms | 851.3 ms |
| worker: complete one scoped task | 10000 | 82.4 ms | 155.1 ms | 264.1 ms | 1160.4 ms |
| worker: poll that leased tasks | 157 | 418.3 ms | 849.1 ms | 2246.3 ms | 6023.0 ms |
| worker: poll with nothing ready | 40 | 4.5 ms | 8.3 ms | 14.5 ms | 14.5 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 1666 | 62.6 ms | 65 | 15038 | 1379 |
| checkpoint | 3332 | 69.8 ms | 60 | 20538 | 1968 |
| checkpoint | 4998 | 92.5 ms | 60 | 25276 | 2502 |
| admit | 6664 | 121.8 ms | 65 | 30032 | 3001 |
| admit | 8330 | 185.9 ms | 65 | 35030 | 3641 |
| checkpoint | 9996 | 249.0 ms | 60 | 40514 | 4174 |
| checkpoint | 11662 | 276.6 ms | 60 | 45252 | 4684 |
| admit | 13328 | 330.4 ms | 65 | 50024 | 5047 |
| admit | 14994 | 392.8 ms | 65 | 55022 | 5025 |
| checkpoint | 16660 | 523.7 ms | 60 | 60490 | 6265 |
| checkpoint | 18326 | 640.6 ms | 60 | 65228 | 6690 |
| checkpoint | 19992 | 698.7 ms | 60 | 70030 | 7103 |



### 4. Sequential loops

A `while` loop with `carry < N`, `maxIterations` N, a body of one activity and a `return`, driven by
`advance_loop` and a protocol 1.5 worker whose output is the iteration index plus one. The same
statement/row accounting is used. The loop shows the same shape as the collection: constant statements
(83 per transition) and rows returned growing by five per completed iteration (561 at iteration 100, 5,067
at 1,000). 100 and 1,000 iterations completed in 8.7 s and 129 s; 10,000 did not complete in its 900 s
budget (3,940 iterations; p50 of a transition 34 ms in the first tenth, 72 to 78 ms in the later ones).
The 10,000-iteration fragment states `7.8` and `11.1` iterations per second computed over all iterations;
the correct figure is the iterations done per second, 3,940 / 900 = 4.4.


#### 100 sequential iterations

- Outcome: run status Succeeded, completed; wall 8.7 s, 101 scheduler transitions, 11.5 iterations/s.
- Resources before: 4 threads, 21 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 23 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3.
- Durable size: 2410 journal bytes per iteration (`pg_column_size`, no index/toast), 202 step rows.
- p50 (ms) of checkpoint/admit transitions by tenth of the run, first to last: 24.6, 25.2, 21.6, 22.5, 24.0, 26.5, 26.2, 27.5, 25.8, 28.6

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: checkpoint an iteration and admit the next | 100 | 25.5 ms | 41.4 ms | 46.1 ms | 50.5 ms |
| scheduler: enter and admit the first body | 1 | 49.6 ms | 49.6 ms | 49.6 ms | 49.6 ms |
| scheduler: nothing to advance (waiting for a worker) | 100 | 8.8 ms | 12.9 ms | 18.2 ms | 25.6 ms |
| worker: complete one scoped task | 100 | 15.7 ms | 22.7 ms | 26.9 ms | 38.7 ms |
| worker: poll that leased a task | 100 | 20.8 ms | 28.4 ms | 41.5 ms | 42.7 ms |
| worker: poll with nothing ready | 100 | 4.1 ms | 6.6 ms | 8.7 ms | 9.8 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| checkpoint and admit | 10 | 28.7 ms | 83 | 111 | 272 |
| checkpoint and admit | 20 | 25.2 ms | 83 | 161 | 344 |
| checkpoint and admit | 30 | 28.3 ms | 83 | 211 | 469 |
| checkpoint and admit | 40 | 24.2 ms | 83 | 261 | 602 |
| checkpoint and admit | 50 | 25.0 ms | 83 | 311 | 778 |
| checkpoint and admit | 60 | 27.4 ms | 83 | 361 | 324 |
| checkpoint and admit | 70 | 26.2 ms | 83 | 411 | 348 |
| checkpoint and admit | 80 | 23.4 ms | 83 | 461 | 354 |
| checkpoint and admit | 90 | 44.2 ms | 83 | 511 | 390 |
| checkpoint and admit | 100 | 42.2 ms | 92 | 567 | 524 |


#### 1000 sequential iterations

- Outcome: run status Succeeded, completed; wall 128.7 s, 1001 scheduler transitions, 7.8 iterations/s.
- Resources before: 4 threads, 24 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 26 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3.
- Durable size: 2399 journal bytes per iteration (`pg_column_size`, no index/toast), 2002 step rows.
- p50 (ms) of checkpoint/admit transitions by tenth of the run, first to last: 25.0, 28.3, 30.7, 30.1, 33.9, 33.0, 41.2, 38.2, 38.0, 38.5

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: checkpoint an iteration and admit the next | 1000 | 34.1 ms | 68.4 ms | 158.0 ms | 463.2 ms |
| scheduler: enter and admit the first body | 1 | 48.4 ms | 48.4 ms | 48.4 ms | 48.4 ms |
| scheduler: nothing to advance (waiting for a worker) | 1000 | 15.7 ms | 31.2 ms | 71.1 ms | 198.3 ms |
| worker: complete one scoped task | 1000 | 24.3 ms | 49.5 ms | 105.0 ms | 375.0 ms |
| worker: poll that leased a task | 1000 | 29.4 ms | 61.1 ms | 135.6 ms | 231.8 ms |
| worker: poll with nothing ready | 1000 | 4.6 ms | 8.3 ms | 17.4 ms | 63.5 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| checkpoint and admit | 100 | 35.0 ms | 83 | 561 | 420 |
| checkpoint and admit | 200 | 38.6 ms | 83 | 1061 | 572 |
| checkpoint and admit | 300 | 31.7 ms | 83 | 1561 | 529 |
| checkpoint and admit | 400 | 36.9 ms | 83 | 2061 | 566 |
| checkpoint and admit | 500 | 31.1 ms | 83 | 2561 | 662 |
| checkpoint and admit | 600 | 31.1 ms | 83 | 3061 | 772 |
| checkpoint and admit | 700 | 49.3 ms | 83 | 3561 | 868 |
| checkpoint and admit | 800 | 37.6 ms | 83 | 4061 | 833 |
| checkpoint and admit | 900 | 33.2 ms | 83 | 4561 | 856 |
| checkpoint and admit | 1000 | 39.9 ms | 92 | 5067 | 965 |


#### 10000 sequential iterations (BUDGET EXHAUSTED, NOT COMPLETED)

- Outcome: run status Running, NOT completed within the time budget; wall 900.1 s, 3941 scheduler transitions, 11.1 iterations/s.
- Resources before: 4 threads, 22 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 50 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3.
- Durable size: 945 journal bytes per iteration (`pg_column_size`, no index/toast), 7883 step rows.
- p50 (ms) of checkpoint/admit transitions by tenth of the run, first to last: 34.2, 43.6, 51.6, 52.5, 70.1, 54.7, 72.3, 68.1, 78.1, 71.9

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: checkpoint an iteration and admit the next | 3940 | 62.5 ms | 97.8 ms | 151.5 ms | 279.5 ms |
| scheduler: enter and admit the first body | 1 | 55.4 ms | 55.4 ms | 55.4 ms | 55.4 ms |
| scheduler: nothing to advance (waiting for a worker) | 3941 | 41.5 ms | 65.6 ms | 79.3 ms | 148.4 ms |
| worker: complete one scoped task | 3941 | 50.3 ms | 81.8 ms | 119.0 ms | 216.0 ms |
| worker: poll that leased a task | 3941 | 56.8 ms | 93.0 ms | 153.6 ms | 281.2 ms |
| worker: poll with nothing ready | 3941 | 5.6 ms | 11.1 ms | 17.9 ms | 46.6 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| checkpoint and admit | 1000 | 121.5 ms | 83 | 5061 | 914 |
| checkpoint and admit | 2000 | 51.1 ms | 83 | 10061 | 1387 |
| checkpoint and admit | 3000 | 74.7 ms | 83 | 15061 | 1816 |



### 5. Chains with many segments

A published continuation: an activity, a condition and `continueAsNew`, with `maxSegments` equal to the
number of segments, driven by the production tick and a worker. Hand-off cost is flat: 50 ms p50 per
hand-off from the first to the last tenth of a 1,000-segment chain, and the PostgreSQL connection count did
not change.

The accepted expression subset cannot construct an object, so a conditional chain carries the rest of its
segments as a nested `next` in the first input, and the run input is parsed as a JSON object with a
recursion limit of 128. **127 segments run, 128 are refused at start** with
`the run input must be a JSON object` (G462). The 1,000-segment run uses an unconditional hand-off that
repeats the same input; it runs until `maxSegments` and ends `FAILED`, which is the segment limit being
enforced and is what that fragment's "final outcome FAILED" means.


#### 10 segments

- Start request with the 243-byte nested input: 33.4 ms. Outcome: chain reached final outcome SUCCEEDED; wall 1.0 s, 9 hand-offs, 8.7 segments/s; `segment_count` 10, 10 segment rows.
- Resources before: 4 threads, 21 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 23 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 2.
- p50 (ms) of hand-off ticks by tenth of the chain, first to last: n/a

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick that handed a chain to its successor | 9 | 56.9 ms | 84.9 ms | 84.9 ms | 84.9 ms |
| scheduler tick with no hand-off | 1 | 30.4 ms | 30.4 ms | 30.4 ms | 30.4 ms |
| worker: complete a segment activity | 10 | 17.1 ms | 32.4 ms | 32.4 ms | 32.4 ms |


#### 64 segments

- Start request with the 1593-byte nested input: 37.0 ms. Outcome: chain reached final outcome SUCCEEDED; wall 6.7 s, 63 hand-offs, 9.5 segments/s; `segment_count` 64, 64 segment rows.
- Resources before: 4 threads, 23 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 24 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3.
- p50 (ms) of hand-off ticks by tenth of the chain, first to last: 63.4, 55.1, 45.6, 52.7, 56.6, 60.4, 55.8, 54.9, 48.7, 57.3

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick that handed a chain to its successor | 63 | 55.8 ms | 87.4 ms | 127.1 ms | 127.1 ms |
| scheduler tick with no hand-off | 1 | 14.9 ms | 14.9 ms | 14.9 ms | 14.9 ms |
| worker: complete a segment activity | 64 | 19.3 ms | 29.4 ms | 51.5 ms | 51.5 ms |


#### 120 segments

- Start request with the 2993-byte nested input: 49.1 ms. Outcome: chain reached final outcome SUCCEEDED; wall 15.2 s, 119 hand-offs, 7.8 segments/s; `segment_count` 120, 120 segment rows.
- Resources before: 4 threads, 24 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 24 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3.
- p50 (ms) of hand-off ticks by tenth of the chain, first to last: 92.8, 65.9, 61.1, 60.5, 65.8, 72.3, 83.3, 70.4, 64.4, 52.0

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick that handed a chain to its successor | 119 | 65.6 ms | 120.2 ms | 161.9 ms | 248.8 ms |
| scheduler tick with no hand-off | 1 | 19.7 ms | 19.7 ms | 19.7 ms | 19.7 ms |
| worker: complete a segment activity | 120 | 22.0 ms | 38.2 ms | 49.1 ms | 59.7 ms |


#### 126 segments

- Start request with the 3143-byte nested input: 36.6 ms. Outcome: chain reached final outcome SUCCEEDED; wall 10.6 s, 125 hand-offs, 11.8 segments/s; `segment_count` 126, 126 segment rows.
- Resources before: 4 threads, 21 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 23 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3.
- p50 (ms) of hand-off ticks by tenth of the chain, first to last: 53.9, 37.7, 47.4, 44.6, 45.9, 46.1, 47.3, 47.7, 50.8, 53.5

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick that handed a chain to its successor | 125 | 48.8 ms | 58.8 ms | 79.1 ms | 84.6 ms |
| scheduler tick with no hand-off | 1 | 16.6 ms | 16.6 ms | 16.6 ms | 16.6 ms |
| worker: complete a segment activity | 126 | 14.4 ms | 19.4 ms | 22.6 ms | 26.3 ms |


#### 127 segments

- Start request with the 3168-byte nested input: 37.0 ms. Outcome: chain reached final outcome SUCCEEDED; wall 10.5 s, 126 hand-offs, 12.0 segments/s; `segment_count` 127, 127 segment rows.
- Resources before: 4 threads, 21 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 23 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 3.
- p50 (ms) of hand-off ticks by tenth of the chain, first to last: 57.1, 39.9, 39.9, 40.7, 51.8, 50.5, 45.5, 45.9, 46.0, 46.8

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick that handed a chain to its successor | 126 | 46.5 ms | 57.6 ms | 82.6 ms | 85.1 ms |
| scheduler tick with no hand-off | 1 | 15.0 ms | 15.0 ms | 15.0 ms | 15.0 ms |
| worker: complete a segment activity | 127 | 14.6 ms | 19.9 ms | 22.9 ms | 27.6 ms |


#### 128 segments

- Start REFUSED (3193 byte nested input): NotAllowed("the run input must be a JSON object"). No chain was run.


#### 129 segments

- Start REFUSED (3218 byte nested input): NotAllowed("the run input must be a JSON object"). No chain was run.


#### 1000 segments

- Start request with the 17-byte nested input: 34.4 ms. Outcome: chain reached final outcome FAILED; wall 90.2 s, 1000 hand-offs, 11.1 segments/s; `segment_count` 1000, 1000 segment rows.
- Resources before: 4 threads, 21 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 23 MiB RSS, 4 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 4.
- p50 (ms) of hand-off ticks by tenth of the chain, first to last: 55.6, 50.3, 49.2, 47.7, 48.9, 46.3, 50.1, 54.8, 48.8, 50.0

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick that handed a chain to its successor | 1000 | 50.1 ms | 74.1 ms | 95.0 ms | 148.8 ms |
| worker: complete a segment activity | 1000 | 13.9 ms | 23.2 ms | 37.2 ms | 68.0 ms |



## Defects and anomalies

1. **G461 (defect, not fixed): one collection or loop transition reads O(progress) rows.** Statement
   count per transition is constant; rows returned grow by about 3 per transition index in a collection and
   5 per iteration in a loop. The queries are the restore of every step of the run, the load of every item
   payload, every iteration row, every step output and every checkpoint, and
   `advance_collection_in_view` iterates `0..next_admission` on each call. Red tests (both `#[ignore]`d,
   run in series 1 against the pg_stat_statements container, both failed as designed):
   `red_collection_transition_rows_do_not_grow_with_progress` ("rows per transition grew from 218 (first
   ten) to 1146 (last ten) over 160 items") and `red_loop_transition_rows_do_not_grow_with_progress` ("from
   91 to 841 over 160 iterations"). Consequence: the 10,000-item collection took 1 h 54 min at maxConcurrency 256
   and did not finish in 10 minutes at 1, 16 or 256; the 10,000-iteration loop did not finish in 15
   minutes (sections 3 and 4). Not tuned, not fixed.
2. **G468 (anomaly): the cost of an idle scheduler tick grows with the number of pending waits and grew
   between the harness commit and this branch.** 26 ms with nothing pending, 186 ms with 10,000 approvals
   pending (section 1 table). A/B on the same container and populations: the harness commit `42375f3`
   measured 118.7 ms and 118.1 ms for the idle tick over 10,000 approvals and 1,000 timers; this branch
   measured 198.0 ms and 197.6 ms, 67 percent more. Run creation was the same within noise (A: 194 and 203
   runs/s, B: 191 and 191). The new passes of feature 014 (collections, loops, continuations) each add a
   discovery read. Under `pg_stat_statements` an idle tick over a 2,000-approval population (separate run,
   same container, 80 ticks) spent 14.8 ms per call in the collection discovery query and 14.7 ms in the loop
   discovery query, both returning zero rows, against 7.3 ms for the approval-opening query: those two new
   passes are about half of the 59 to 75 ms tick at that size. Both scan the pending root steps of every run
   and read each run's stored document as JSON to look for a `forEach`; the 10,000-approval population was
   not repeated under statement accounting.
3. **The start path did not regress.** The earlier report of this file measured 257 approval runs/s on the
   default test container at load 11.87. The A/B in section 1, with that same harness commit built in
   release on the dedicated container, gives 194 and 203 runs/s, against 191 and 191 for this branch, so the
   earlier figure is not reproducible by either build here (the container differs: `fsync` was off in the
   earlier one). The first wait run of this work (65 runs/s, p99 start 3.0 s) started at host load 41 and
   is kept (Appendix A) as evidence of how much a loaded host moves the result; no conclusion rests on it.
4. **G462: a conditional chain cannot exceed 127 segments** (JSON recursion limit 128 on the run input).
5. **G463/G464: progress is one transition per tick per run; `enter` is one 3.6 s transaction at 10,000
   items.**
6. **The tenant slot figures needed a slow worker to be exercised.** With a worker that answers within one
   scheduler round, three runs never held more than three slots and the first version of the test failed its
   own "the burst saturated its slots" assertion. The report's configuration (eight rounds per task) is what
   makes the bound meaningful.
7. **The harness had defects, found by running it.** Fixed before the numbers in this file: the scheduler's
   shard lease of 60 s expired during the load phase (the opening loop reported a stall at 0/10,000), and
   the wait fragments printed the end-of-run load average under the label "at start" (the table headers
   here use the load recorded when each run started). Fixed after the series 1 runs and therefore still
   present in their fragments: a throughput and a journal-bytes-per-item line of an unfinished run divided
   by all items, in the three unfinished 10,000-item collections and the 10,000-iteration loop; sections 3
   and 4 give the corrected figures.
8. **`approval scheduling delay`** in the section 1 table is the length of the load phase, not a scheduler
   latency (see there).
9. **G460: a reader that cannot parse a pinned definition fails the whole worker poll with `Internal`**
   (found by the compatibility tests, recorded in `docs/sdk-compatibility.md`).

## Ceilings

What these numbers support, on one shared laptop, for one scheduler replica, one PostgreSQL instance and
the dedicated container described above. "Support" means the workload ran to the stated size with every
asserted bound holding; it is not a promise at that size.

| Ceiling under consideration | Supported by | Not supported |
| --- | --- | --- |
| Suspended approvals and timers hold no worker, thread or connection | Asserted at 10,000 approvals and 1,000 timers: 4 threads, 8 connections (the pool), 0 idle in a transaction, 0 worker sessions, no lease on a wait step; storage 4.5 KB per approval and 2.5 KB per timer | Any number of waits above 11,000; the scheduler's idle cost at that size is 186 to 199 ms per tick and grows (G468), so "free" is true of workers and false of scheduler work |
| Tenant concurrency bound across runs | Never exceeded in 279 rounds with 48 slots of demand against 8, counter equal to real holders every round; 8 of 64 admitted under a held lock, 56 typed refusals; the quiet tenant needed as many rounds beside the burst as alone | A cluster-wide bound (one database only), fairness at other ratios, other primitives sharing the same tenant policy |
| Collection of up to 1,000 items | Completed at maxConcurrency 1, 16 and 256 in 125 to 188 s (5 to 8 items/s), bounds respected, 2,335 journal bytes per item | A throughput or latency objective (one run per cell under load 7 to 23) |
| Collection of 10,000 items | One full run at maxConcurrency 256 completed correctly with every bound held: 6,815 s, 20,002 transitions, 1.5 items/s, 2,334 journal bytes per item, process RSS 22 to 118 MiB | A usable `maxItems` of 10,000: the same run took 47 times the 1,000-item run, and the 600 s budgets of maxConcurrency 1, 16 and 256 all ended unfinished. Other concurrencies were not run to completion. With G461 open the recommendation (O460) is to publish at most the largest size that finishes in minutes, 1,000 |
| Loop of up to 1,000 iterations | Completed in 129 s | 10,000 iterations (3,940 done in 900 s), nested loops, the 100,000-instance ceiling |
| Chain of many segments | Flat 50 ms per hand-off to 1,000 segments (unconditional shape) and 127 (conditional shape) | More than 127 conditional segments (refused) |
| Memory | Process RSS 15 to 60 MiB over every run except the full-length collection (118 MiB at its end); the PostgreSQL container used 164 MiB RSS at the end of the series | A memory ceiling: RSS of the benchmark process only, PostgreSQL memory not isolated per workload |
| Latency percentiles | Recorded as observed | Any p95/p99 objective: single runs, shared host, in-process, no network |

## Not measured

- Loops parked in a durable delay as a population (timers were measured; a loop delay has its own path).
- Nested loops reaching the 100,000-instance ceiling (nested admission is closed).
- Approval decision throughput and the approval-expiry race under load.
- More than one scheduler replica, replica failover under load, a network between the services and the
  database, TLS, the HTTP and gRPC layers, authentication.
- Target hardware, a production-sized PostgreSQL, a PostgreSQL with a replica.
- Per-pass attribution of the idle tick at 10,000 approvals (it was measured at 2,000 only).
- Run output size limits at 10,000 items (the completed runs used no `outputExpr`).

## Reproduction

The commands are in "Commands". The workloads are the `#[ignore]`d tests of
`tests/performance/workflow-primitives/` (`main.rs` waits and the 64-run slot lock, `collections.rs`,
`loops.rs`, `chains.rs`, `tenants.rs`, shared code in `common.rs`). Each writes its fragment to
`SAGAT_PERF_FRAGMENTS`. The numbers in this file were produced from those fragments without editing,
except that the corrections named in "Defects and anomalies" 7 are stated next to the figures they
correct. A reduced run (smaller populations) must not replace this report.


## Appendix A: first wait run (host load 41 at the start, 17 at the end)

Kept as raw evidence of how a loaded host moves the result; not used for conclusions.


#### Durable waits and the tenant slot lock

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average at start: 16.98 26.64 30.06
- Build profile: release
- Commit: e3e3979
- PostgreSQL: Docker container on the same host
- SQLx pool: 8 connections
- Services: in-process; no client network latency

##### Configuration

- 10000 approval runs, loaded with 32 concurrent callers
- 1000 one-hour timer runs, loaded with 32 concurrent callers
- scheduler batch 200, scheduler concurrency 8, 50 idle ticks
- 64 collection runs contending for 8 tenant slots with 5 concurrent callers
- tenant usage row held for 100 ms to make lock contention observable

##### Latency observations

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| approval start transaction | 10000 | 312.9 ms | 1554.9 ms | 3037.3 ms | 6939.2 ms |
| timer start transaction | 1000 | 254.1 ms | 378.6 ms | 466.7 ms | 606.2 ms |
| approval scheduling delay (run created to request persisted) | 10000 | 88983.0 ms | 147164.1 ms | 151697.7 ms | 154241.1 ms |
| timer scheduling delay (run created to wait persisted) | 1000 | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms |
| scheduler tick with all waits idle | 50 | 211.0 ms | 237.8 ms | 254.7 ms | 254.7 ms |
| collection admission accepted under tenant-slot contention | 8 | 118.2 ms | 168.9 ms | 168.9 ms | 168.9 ms |
| collection admission rejected at tenant capacity | 56 | 5.1 ms | 33.9 ms | 57.1 ms | 57.1 ms |

##### Throughput and storage

- Approval run creation: 154.0 s, 65 runs/s.
- Approval opening: 43.5 s, 230 approvals/s across 50 scheduler ticks.
- Timer run creation and wait persistence: 8.5 s, 117 runs/s.
- Observed relation growth: 4460 bytes per suspended approval and 2482 bytes per suspended timer. This is a delta over shared PostgreSQL relations and includes index allocation noise.
- Journal row bytes: 458 bytes per approval run and 394 bytes per timer run, measured with `pg_column_size` and excluding index/toast overhead.
- Test-process RSS: 15253504 bytes before loading and 26624000 bytes after measurement, a signed change of 11370496 bytes. PostgreSQL container RSS was not isolated in this run.

##### Correctness observations

- Resources while the waits are pending (the engine, scheduler and worker service run in this process; PostgreSQL connections are every backend of the benchmark database):
  - no waits: 4 threads, 15 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals started, none opened yet: 4 threads, 24 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals pending: 4 threads, 25 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals pending + 1000 timers waiting: 4 threads, 25 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - after 50 idle ticks over all waits: 4 threads, 25 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
- 8/64 collection admissions committed; 56 were rejected with the typed tenant-capacity error. Durable tenant usage and persisted iterations both equal 8.
- PostgreSQL exposed 5 lock waiters while the tenant usage row was deliberately held. Admission latencies therefore include a known 100 ms contention interval plus connection-pool and serialization delay.
- 11000 approval/timer steps are durably waiting with no lease token, lease expiry or worker identity. Active worker sessions: 0.
- Scheduler outcomes while opening and observing waits: 0 skipped, 0 fenced and 0 failed. Explicit client transaction retries: 0; every start and admission was submitted once.

##### Notes

- The first default-population attempt was rejected after reaching the platform's default 10,000 active-run namespace quota. The benchmark now grants and configures explicit headroom before loading; this preserves production admission behavior while isolating the wait measurement.
- Approval and timer runs use the supported publish/start/scheduler paths. Waiting consumes database rows and scheduler index scans; it does not allocate one process, connection, worker session or broker consumer per wait.
- The shared development host, local container, eight-connection pool and in-process services are not representative of a production cluster. Repeat on target hardware before sizing.
- The controlled slot test measures one tenant and one database. It proves exact enforcement under cross-run contention; it does not establish a cluster-wide throughput floor.
- No timeout, queue-delay or capacity ceiling is inferred beyond the populations and concurrency recorded above.



## Appendix B: wait control run on the default test container


#### Durable waits and the tenant slot lock

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average at start: 12.56 13.50 11.55
- Build profile: release
- Commit: e3e3979
- PostgreSQL: Docker container on the same host
- SQLx pool: 8 connections
- Services: in-process; no client network latency

##### Configuration

- 10000 approval runs, loaded with 32 concurrent callers
- 1000 one-hour timer runs, loaded with 32 concurrent callers
- scheduler batch 200, scheduler concurrency 8, 50 idle ticks
- 64 collection runs contending for 8 tenant slots with 5 concurrent callers
- tenant usage row held for 100 ms to make lock contention observable

##### Latency observations

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| approval start transaction | 10000 | 198.7 ms | 220.3 ms | 248.1 ms | 453.9 ms |
| timer start transaction | 1000 | 218.1 ms | 247.7 ms | 286.3 ms | 364.7 ms |
| approval scheduling delay (run created to request persisted) | 10000 | 50531.2 ms | 61554.0 ms | 62368.7 ms | 62880.7 ms |
| timer scheduling delay (run created to wait persisted) | 1000 | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms |
| scheduler tick, no waits exist (control) | 10 | 25.1 ms | 54.7 ms | 54.7 ms | 54.7 ms |
| scheduler tick, 10000 approvals pending | 10 | 182.0 ms | 204.4 ms | 204.4 ms | 204.4 ms |
| scheduler tick, 10000 approvals + 1000 timers pending | 50 | 198.3 ms | 217.4 ms | 232.2 ms | 232.2 ms |
| collection admission accepted under tenant-slot contention | 8 | 122.5 ms | 174.3 ms | 174.3 ms | 174.3 ms |
| collection admission rejected at tenant capacity | 56 | 4.1 ms | 27.5 ms | 58.3 ms | 58.3 ms |

##### Throughput and storage

- Approval run creation: 62.5 s, 160 runs/s.
- Approval opening: 37.0 s, 270 approvals/s across 50 scheduler ticks.
- Timer run creation and wait persistence: 6.9 s, 145 runs/s.
- Observed relation growth: 4476 bytes per suspended approval and 2408 bytes per suspended timer. This is a delta over shared PostgreSQL relations and includes index allocation noise.
- Journal row bytes: 458 bytes per approval run and 394 bytes per timer run, measured with `pg_column_size` and excluding index/toast overhead.
- Test-process RSS: 19136512 bytes before loading and 28459008 bytes after measurement, a signed change of 9322496 bytes. PostgreSQL container RSS was not isolated in this run.

##### Correctness observations

- Resources while the waits are pending (the engine, scheduler and worker service run in this process; PostgreSQL connections are every backend of the benchmark database):
  - no waits: 6 threads, 18 MiB RSS, 2 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals started, none opened yet: 5 threads, 26 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals pending: 5 threads, 27 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - 10000 approvals pending + 1000 timers waiting: 5 threads, 27 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
  - after 50 idle ticks over all waits: 5 threads, 27 MiB RSS, 8 PostgreSQL connections (1 active, 0 idle in transaction)
- 8/64 collection admissions committed; 56 were rejected with the typed tenant-capacity error. Durable tenant usage and persisted iterations both equal 8.
- PostgreSQL exposed 5 lock waiters while the tenant usage row was deliberately held. Admission latencies therefore include a known 100 ms contention interval plus connection-pool and serialization delay.
- 11000 approval/timer steps are durably waiting with no lease token, lease expiry or worker identity. Active worker sessions: 0.
- Scheduler outcomes while opening and observing waits: 0 skipped, 0 fenced and 0 failed. Explicit client transaction retries: 0; every start and admission was submitted once.

##### Notes

- The first default-population attempt was rejected after reaching the platform's default 10,000 active-run namespace quota. The benchmark now grants and configures explicit headroom before loading; this preserves production admission behavior while isolating the wait measurement.
- Approval and timer runs use the supported publish/start/scheduler paths. Waiting consumes database rows and scheduler index scans; it does not allocate one process, connection, worker session or broker consumer per wait.
- The shared development host, local container, eight-connection pool and in-process services are not representative of a production cluster. Repeat on target hardware before sizing.
- The controlled slot test measures one tenant and one database. It proves exact enforcement under cross-run contention; it does not establish a cluster-wide throughput floor.
- No timeout, queue-delay or capacity ceiling is inferred beyond the populations and concurrency recorded above.



## Appendix C: idle-tick scaling runs

Separate runs of the same test with `SAGAT_PRIMITIVES_APPROVALS` and `SAGAT_PRIMITIVES_TIMERS` reduced; only the idle-tick rows are quoted, the full fragments are in the run directory.


### 500 approvals and 50 timers

| approval start transaction | 500 | 184.8 ms | 338.9 ms | 356.0 ms | 391.7 ms |
| scheduler tick, no waits exist (control) | 10 | 27.4 ms | 42.6 ms | 42.6 ms | 42.6 ms |
| scheduler tick, 500 approvals pending | 10 | 41.8 ms | 46.6 ms | 46.6 ms | 46.6 ms |
| scheduler tick, 500 approvals + 50 timers pending | 50 | 35.2 ms | 44.1 ms | 60.0 ms | 60.0 ms |
- Approval run creation: 3.2 s, 158 runs/s.
- Approval opening: 6.2 s, 80 approvals/s across 3 scheduler ticks.


### 2000 approvals and 200 timers

| approval start transaction | 2000 | 187.6 ms | 212.4 ms | 323.2 ms | 482.4 ms |
| scheduler tick, no waits exist (control) | 10 | 25.7 ms | 40.0 ms | 40.0 ms | 40.0 ms |
| scheduler tick, 2000 approvals pending | 10 | 75.1 ms | 82.0 ms | 82.0 ms | 82.0 ms |
| scheduler tick, 2000 approvals + 200 timers pending | 50 | 58.9 ms | 73.5 ms | 96.0 ms | 96.0 ms |
- Approval run creation: 11.9 s, 168 runs/s.
- Approval opening: 4.8 s, 413 approvals/s across 10 scheduler ticks.


### 5000 approvals and 500 timers

| approval start transaction | 5000 | 184.7 ms | 215.1 ms | 291.9 ms | 424.7 ms |
| scheduler tick, no waits exist (control) | 10 | 26.9 ms | 44.1 ms | 44.1 ms | 44.1 ms |
| scheduler tick, 5000 approvals pending | 10 | 142.1 ms | 146.1 ms | 146.1 ms | 146.1 ms |
| scheduler tick, 5000 approvals + 500 timers pending | 50 | 109.1 ms | 124.3 ms | 151.8 ms | 151.8 ms |
- Approval run creation: 29.4 s, 170 runs/s.
- Approval opening: 15.2 s, 328 approvals/s across 25 scheduler ticks.


## After G461

Written 2026-10-17 on branch `feature/014-scale`, after the scheduler decides a collection or loop
transition from a bounded frontier instead of the whole restored run (`research.md`, D520-D525). The
sections above are the measurements before the change and are unchanged.

### What changed, and what did not

- A collection or loop transition now reads the scope's counters, the iterations that hold a slot and
  the steps of those iterations (`ScopeFrontierStore`), not every step, iteration, checkpoint and output
  of the run. Entering a scope, the last checkpoint of a collection, the finish and scoped compensation
  still read the whole run, once per scope.
- Two partial indexes (`steps_run_open_idx`, `steps_run_waiting_idx`, migration `20261122000000`) make
  "is any step of this run still open" and the collection and loop discovery queries independent of the
  number of finished or waiting steps.
- **Not changed: the worker's `complete` and `poll` still restore the whole run** (G520). They are now
  most of the wall time of a large scope; the 10,000-item collection is faster but is still not linear.

### Environment

Same host as above (Apple M3 Max, 16 logical cores, 48 GiB), PostgreSQL 16.15 in the colima VM (4 CPUs,
5.8 GiB), container `sagat-014-scale-pg`, `postgres:16-alpine` with the flags of the earlier container plus
`max_connections=300`, port 55443; release build of `perf_workflow_primitives` at commit `e8ccca8b`
(the code of the change; the later commits add tests and the G460 skip). Load average (1 minute) at the
start of each run is in its row. The host was shared with other sessions' builds and test runs the whole
time; wall times carry that noise, rows and buffers do not.

```sh
export CARGO_TARGET_DIR=/private/tmp/claude-501/targets/014-scale CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_INCREMENTAL=false
export DOCKER_HOST=unix:///Users/marcelo/.colima/sagat-remaining/docker.sock
docker run -d --name sagat-014-scale-pg -p 127.0.0.1:55443:5432 -e POSTGRES_PASSWORD=postgres postgres:16-alpine \
  -c shared_preload_libraries=pg_stat_statements -c pg_stat_statements.track=all -c pg_stat_statements.max=10000 -c max_connections=300
docker exec sagat-014-scale-pg psql -U postgres -d template1 -c "CREATE EXTENSION pg_stat_statements"
cargo test -p sagat-tests --test perf_workflow_primitives --locked --release --no-run
export SAGAT_TEST_DATABASE_URL=postgres://postgres:postgres@127.0.0.1:55443/postgres
BIN=$CARGO_TARGET_DIR/release/deps/perf_workflow_primitives-<hash>
$BIN --nocapture red_                                  # the two tests of G461 (now passing, no longer ignored)
$BIN --nocapture flat_across                           # 160 / 320 / 640, collection and loop
SAGAT_PERF_COLLECTION_ITEMS=100 SAGAT_PERF_COLLECTION_CONCURRENCY=1,16,100 $BIN --ignored --nocapture measure_collection_fanout
SAGAT_PERF_COLLECTION_ITEMS=1000 SAGAT_PERF_COLLECTION_CONCURRENCY=1,16,256 $BIN --ignored --nocapture measure_collection_fanout
SAGAT_PERF_COLLECTION_ITEMS=10000 SAGAT_PERF_COLLECTION_CONCURRENCY=256 SAGAT_PERF_BUDGET_SECS=1800 $BIN --ignored --nocapture measure_collection_fanout
SAGAT_PERF_LOOP_ITERATIONS=100,1000 $BIN --ignored --nocapture measure_sequential_loops
SAGAT_PERF_LOOP_ITERATIONS=10000 SAGAT_PERF_BUDGET_SECS=1800 $BIN --ignored --nocapture measure_sequential_loops
```

### Rows and buffers of one transition, by size of the scope

Median of the transitions between two thirds and 95 percent of the run (`late_cost`), concurrency 2,
`pg_stat_statements` around each transition. The sizes were run in the same process, one after the other.

| Scope | Size | Rows returned/affected | Buffer accesses | Note |
|---|---:|---:|---:|---|
| collection | 160 | 44 | 210 | before the open-step indexes |
| collection | 320 | 44 | 237 | |
| collection | 640 | 44 | 262 | buffers still grew with size: the `finish_version` probe walked the finished steps |
| collection | 160 | 44 | 225 | with the open-step indexes |
| collection | 320 | 44 | 247 | |
| collection | 640 | 44 | 217 | flat |
| loop | 160 | 65 | 317 | before the open-step indexes |
| loop | 320 | 65 | 367 | |
| loop | 640 | 65 | 381 | |

Before the change, from the red tests of the sections above: a collection of 160 read 57 rows in the
first ten transitions and 829 in the last ten (this run, original binary); the loop 91 to 841. The 1,000-item
runs read 1,536 rows at transition 166 and 7,014 at 1,992; the loop 561 at iteration 100 and 5,067 at 1,000.
After the change the 10,000-item loop reads 65 rows at iterations 1,000 to 7,000 (buffers 345 to 381) and
the 1,000-item collection at concurrency 256 reads between 65 and 665 rows depending on how many of its 256
iterations are in flight (the O(`maxConcurrency`) term), 49 to 55 statements, with no trend over the run.

The control (`SAGAT_PERF_REFERENCE_PATH=1` drives the whole-run path that is kept as the reference) is
reported under "Control" below.

### Wall time and the p50 of a transition

Before (sections 3 and 4) against after. The p50 of a transition is the admit/checkpoint transition by
tenth of the run, first / sixth / last.

| Workload | Wall before | Wall after | p50 of a transition before (first / 6th / last) | after (first / 6th / last) | Load at start (after) |
|---|---:|---:|---|---|---:|
| collection 100 items, c1 | 11.1 s | 8.4 s | 28.1 / 22.2 / 23.6 ms | 15.5 / 16.1 / 15.4 ms | 13.5 |
| collection 100 items, c16 | 8.8 s | 5.9 s | 24.4 / 24.6 / 28.8 ms | 14.5 / 18.5 / 11.1 ms | 12.8 |
| collection 100 items, c100 | 7.7 s | 5.0 s | 21.9 / 23.1 / 21.1 ms | 12.1 / 11.3 / 10.0 ms | 13.8 |
| collection 1,000 items, c1 | 187.9 s | 94.5 s | 25.0 / 45.1 / 50.6 ms | 12.5 / 14.9 / 13.2 ms | 13.9 |
| collection 1,000 items, c16 | 125.5 s | 61.7 s | 26.0 / 37.4 / 51.3 ms | 14.6 / 12.0 / 11.5 ms | 9.8 |
| collection 1,000 items, c256 | 143.6 s | 53.8 s | 28.6 / 36.8 / 42.5 ms | 12.8 / 13.2 / 13.0 ms | 9.8 |
| **collection 10,000 items, c256** | **6,814.7 s (1 h 54 min)** | **1,404.9 s (23 min 25 s)** | **44.4 / 256.7 / 678.2 ms** | **15.1 / 15.5 / 18.8 ms** | 9.8 |
| loop 100 iterations | 8.7 s | 5.4 s | 24.6 / 26.5 / 28.6 ms | 14.8 / 14.4 / 12.9 ms | 8.7 |
| loop 1,000 iterations | 128.7 s | 98.2 s | 25.0 / 33.0 / 38.5 ms | 13.5 / 18.8 / 16.3 ms | 8.3 |
| loop 10,000 iterations | not completed in 900 s (3,941 iterations) | not completed in 1,800 s (8,806 iterations) | 34.2 / 54.7 / 71.9 ms | 17.6 / 25.6 / 18.7 ms | 9.7 |

p50 / p95 of the 10,000-item collection transition after the change: admit 17.3 / 28.6 ms, checkpoint
16.8 / 28.4 ms (before: 234.4 / 724.7 and 237.1 / 688.9 ms). `enter` of 10,000 items: 3,343.9 ms (before
3,198.4 ms; by design one transaction, G464). The finish: 218.0 ms (before 750.4 ms). Journal bytes per item
unchanged (2,334).

**The scheduler's share is now flat; the wall time is not, because the worker side is not.** In the same
10,000-item run the worker's `complete` has a p50 of 78.4 ms (p95 144.8, max 1,506.8 ms) against 82.4 ms
before, and the `poll` that leases 64 tasks 384.3 ms against 418.3 ms: they read the whole run (G520).
They are 10,000 x 78.4 ms = 784 s of the 1,405 s. At 1,000 items the same `complete` is 18.5 to 21.4 ms, at
100 items 11.0 to 14.3 ms. In the 10,000-iteration loop `complete` is 74.3 ms and `poll` 78.9 ms against
50.3 and 56.8 ms in the first 3,941 iterations before. The loop's wall time per iteration is the sum of
these round trips, which is why it still did not finish in 30 minutes.

### Idle scheduler tick (G468)

The collection and loop discovery queries (`collection_candidates`, `loop_candidates`) scanned the PENDING
root steps of every shard and read each run's stored document as JSON. With the open-step indexes they
read the few open steps. Measured on one database (2,264 waiting steps, 8 ready, 64 shards), the same
queries with `EXPLAIN (ANALYZE)`, three runs each:

| Query | Without the indexes | With the indexes |
|---|---:|---:|
| collection discovery | 16.9, 17.6, 17.3 ms | 3.1, 1.3, 1.3 ms |
| loop discovery | 16.7, 16.3, 16.3 ms | 0.67, 0.68, 0.59 ms |

The wait harness (`measure_durable_waits`, the same row names as above):

| Population | Row | Before (section 1 and Appendix C) | After |
|---|---|---:|---:|
| 2,000 approvals + 200 timers | 50 idle ticks, p50 | 58.9 ms | 18.8 ms |
| 2,000 approvals | 10 ticks, statistics as the load left them, p50 | 75.1 ms | 36.2 ms |
| 10,000 approvals + 1,000 timers | 50 idle ticks, p50 | 118.7 and 118.1 ms (A/B baseline), 197.6 to 198.0 ms (this branch before) | 11.2 ms |
| 10,000 approvals | 10 ticks, statistics as the load left them, p50 | 185.9 ms | **320.8 ms (worse, see below)** |
| none (control) | 10 ticks, p50 | 25.6 ms | 26.1 ms |

The 10-tick rows are taken right after approvals are opened and before the tables have statistics
(autovacuum has not run). At 10,000 approvals that row went from 185.9 to 320.8 ms in one run each, on a
host at load 10 to 40; it is not explained here (G522). The 50-tick row is measured after `ANALYZE`, the
state of a database that has been running for a minute. The harness now takes the 10 ticks twice (before
and after `ANALYZE`) so the two states are separated in later runs.

### Control: the whole-run path on the same binary

`SAGAT_PERF_REFERENCE_PATH=1` drives the scheduler through `advance_collection_from_run` /
`advance_loop_from_run`, the whole-run path that the change keeps for entering, finishing and compensating
a scope (it is the previous implementation: same restore, same statements). Same container, same binary,
11:40 to 11:47 at host load 12 to 27, `flat_across` and `red_`:

| Scope | Size | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|
| collection | 160 | 689 | 277 |
| collection | 320 | 1,337 | 359 |
| collection | 640 | 2,629 | 491 |
| loop | 160 | 711 | 404 |
| loop | 320 | 1,356 | 449 |
| loop | 640 | 2,651 | 574 |

Rows double with the size, as in the sections above. Both `red_*` tests fail on this path with the numbers
of the first report: "rows per transition grew from 57 (first ten) to 829 (last ten) over 160 items" and
"from 91 to 841 over 160 iterations". Both `flat_across` tests fail on "rows per transition grew with the
size: 689 at 160, 1337 at 320" and "711 at 160, 1356 at 320". On the frontier path the same four tests pass
(44 rows for a collection and 65 for a loop at every size).


### Combined-profile migration name

The historical measurements above use the scale branch's 20261122000000 open-step indexes,
which develop merged under that name (PR #37). Feature 017's OpenBao migration is renumbered
20261122000100 in the combined profile (ADR-0036), so the index migration keeps its name and
no applied migration is renamed. Historical source measurements are
retained as reported; the combined runtime still requires its own large-workload release gate.

## Integrated profile: projected instance count (2026-10-17)

Product source: `5ba1c2bc` (the performance binary was built before the subsequent bounded
reconciliation work). Actual command:

```sh
SAGAT_TEST_DATABASE_URL=postgres://postgres:<test-only>@127.0.0.1:<owned-port>/postgres \
CARGO_TARGET_DIR=/Users/marcelo/Documents/Projects/sagat-octopus-database-connections/target \
CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=3 RUST_MIN_STACK=16777216 \
cargo test --locked -p sagat-tests --test perf_workflow_primitives \
  -- flat_across_sizes --nocapture --test-threads=1
```

Both checks passed, no ignored tests, 434.37 seconds:
`/tmp/sagat-completion-charge-flat-cost.log`. Apple M3 Max, 16 logical CPUs, 48 GiB host RAM;
Docker exposed 16 CPUs and 8,318,709,760 bytes RAM. Rust 1.98.0, debug test profile, PostgreSQL
16-alpine with pg_stat_statements. The host was shared with other workloads and builds.

| Scope population | Collection median rows / buffers | Loop median rows / buffers |
| --- | --- | --- |
| 160 | 44 / 247 | 67 / 370 |
| 320 | 44 / 241 | 67 / 389 |
| 640 | 44 / 266 | 67 / 391 |

These are the existing tests' late checkpoint-transition samples (between two thirds and 95%
completion), measured through pg_stat_statements in each workload's database. They cover the
scheduler checkpoint path after replacing its historical step count with the transactional
current-instance projection. They exclude final aggregate/finalization transitions and ordinary
worker polling/reporting, which still use full restoration. They are not a measurement of
10,000-item collections, 100,000 nested instances, full-run memory bounds or production capacity.
Those accepted release criteria remain open; no ceiling or latency promise is inferred here.


## 2026-10-17 — Frozen bounded-worker diagnostic, not final capacity acceptance

Source is detached frozen `025ce8d4`; raw hardware/PostgreSQL settings, latencies, row/buffer work,
resources and source identity are retained in [the manifest](raw/workflow-primitives/20261017-025ce8d4/manifest.json).
The debug executable ran all three cases successfully in 3,864.6 command seconds. Each case
completed 10,000 members, wrote 10,000 checkpoints/20,002 step rows and returned zero scheduler
errors. Fsync and synchronous commit were on; default query/connection settings were retained,
with pg_stat_statements preloaded and an explicit 1 GiB shared-memory budget.

| Scope width | Workload wall seconds | Sampled active peak | Raw fragment |
| --- | ---: | ---: | --- |
| 1 | 1580.7 | 1 | [collection-10000-c1.md](raw/workflow-primitives/20261017-025ce8d4/collection-10000-c1.md) |
| 16 | 980.4 | 15 | [collection-10000-c16.md](raw/workflow-primitives/20261017-025ce8d4/collection-10000-c16.md) |
| 256 | 1163.4 | 255 | [collection-10000-c256.md](raw/workflow-primitives/20261017-025ce8d4/collection-10000-c256.md) |

The active bounds were asserted at each observation, not inferred from elapsed time. At width
256 the observer sampled 255, so these samples do not establish an observed peak of 256. A single
in-process driver polls 64 tasks and completes them serially; these are logical scope widths,
not 256 parallel external worker processes. The host also ran builds and functional suites.

The fixture explicitly constructed execution stores from the owner pool: setting
`SAGAT_TEST_AS_RUNTIME_ROLE=1` alone did not change those stores. This is recorded as an owner-role
profile, not evidence of runtime-role performance. The even probe interval sampled only checkpoints;
for example width1 checkpoint samples returned 41 rows with 49 statements, but no admission SQL-cost
claim follows from them. RSS is before/after, not a measured peak, and queue delay is absent.
The server was a dedicated manually started diagnostic profile; later acceptance uses the
repository-declared disposable fixture. These limitations remain acceptance work, along with
nested/low-capacity admission, 100k nested instances, waits and fairness. No production ceilings
or complete T014-042 acceptance are claimed by these three diagnostic runs.

### Measurement-driver verification

The corrected driver records each leased member's database `ready_at`→`updated_at` queue delay
before completion, observes active counts after every admission, and separates admission and
checkpoint cost comparisons. Missing timestamps or resource observations fail the measurement.
RSS sampled before/after and every 256 transitions is explicitly distinct from the OS process peak.
Three measurement tests passed (6.33s), including an actual journal-writer runtime-role guard.
A self-contained 32-item/width16 smoke passed (4.65s): observed active peak16, 32 queue observations,
both transition families and verified `sagat_app` execution. This small dirty-source diagnostic is
not a frozen 10k acceptance run; raw logs are under `/tmp/sagat-completion-perf-profile-smoke/`.
On Darwin the command was additionally wrapped with `/usr/bin/time -l -o process-resources.log`
to retain native maximum-resident-set/peak-footprint fields for the entire process. Retain this
native output alongside future per-width fragments; do not label sampled RSS an OS peak.

### Runtime-role cross-tenant contention

The first runtime-role execution failed at registration (`NotBound`): the synthetic second worker
had a namespace binding but no persisted current tenant membership. The fixture now persists its
ordinary Worker membership before registration, preserving production authorization. The corrected
default workload passed1/1,46.10s: 6×40 bursting members versus1×40 quiet members, quota8 per tenant.
Persisted holders and usage counters agreed in every round and both peaked at8. The quiet tenant
finished in87 rounds alone and87 beside its neighbour (ratio1.00); the burst finished in279 rounds
(32.9s measured workload wall). This proves the stated case, not an unmeasured throughput SLO.
[Raw evidence and reproducible source](raw/workflow-primitives/20261017-runtime-fairness/manifest.json)
retain the failed registration, successful output, fixture patch, settings and shared-load limitations.

### Wait resource-profile correction

The frozen b1855cbe 10,000-approval/10,000-timer run failed after286.78s: its stale assertion allowed
10 connections while the fixture's existing runtime16 and observer8 pools exposed18. This is a
failed run, not proof of zero worker ownership; later no-worker/slot assertions were not reached.
[Failure and native process resources](raw/workflow-primitives/20261017-waits-profile-red/manifest.json)
retain the source and failure. The correction reads both actual configured pool maxima and verifies
the runtime role, without increasing a pool. The contention store now also shares the execution
pool rather than using the owner observer pool. The report states the two budgets explicitly.
A 20-approval/20-timer check passed1/1,8.13s: 40 waits without leases/worker identities, zero worker
sessions, zero scheduler failures, all resource observations inside the existing combined bound24,
and exact8/64 slot admissions. Full-size corrected evidence is recorded below.


### Runtime-role 20,000 durable waits

The corrected frozen `60dfc824` workload passed1/1 (213.73s harness,214.70s command):
10,000 approvals plus10,000 one-hour timers, using the repository disposable PostgreSQL fixture
and verified `sagat_app` execution. All20,000 waits had no lease/token/expiry/worker identity;
there were zero worker sessions and zero skipped, fenced or failed scheduler outcomes.
Runtime/observer pools retained their existing16/8 maxima. Observed connections peaked at18,
with no idle transactions. Separate runtime contention accepted8 of64 attempts and returned56
typed quota refusals; persisted holders and counters both measured8. No client retry was added.
Native Darwin process resources are retained without relabelling sampled RSS as a peak;
PostgreSQL container RSS was not isolated and the shared host also ran SDK/build/flat workloads.
[Source, command and result](raw/workflow-primitives/20261017-waits-runtime/result.json),
[workload details](raw/workflow-primitives/20261017-waits-runtime/waits.md), and
[native process resources](raw/workflow-primitives/20261017-waits-runtime/process-resources.txt)
are reproducible evidence for this workload, not full T014-042 acceptance or a latency SLO.


### Frozen runtime-role 10,000-item flat collections

All three separate executions passed using frozen clean source`b1855cbe` and the copied binary
SHA-256`372e91c1ee31bd4ce5a85e5f244423de44a1f73be629aed14bdcb4e3d8350ed2`.
Each uses verified`sagat_app` worker/scheduler stores and the repository disposable PostgreSQL16
fixture. Each retained10,000 checkpoints and20,002 step rows, returned zero scheduler errors,
added no client retries and recorded10,000 actual ready-to-lease queue samples.

| Logical concurrency | Observed active peak | Workload wall(s) | Queue p50/p95/p99(ms) | Sampled peak RSS(MiB) |
| --- | ---: | ---: | --- | ---: |
| 1 | 1 | 1238.8 | 28.9 / 96.8 / 208.8 | 121.2 |
| 16 | 16 | 860.1 | 293.5 / 729.0 / 989.2 | 120.6 |
| 256 | 256 | 1387.1 | 7966.6 / 43926.5 / 190292.8 | 120.4 |

Actual holders were observed after every admission and every8 transitions; both configured scope
and tenant bounds were asserted at every observation. The one in-process worker driver polls at
most64 leases and completes them serially. Concurrency256 therefore describes logical iteration
width, not256 parallel external workers. The shared host also ran builds and functional suites;
these observed queue delays establish no latency SLO. Command walls, including setup/inspection,
were1282.68/899.14/1436.18s, distinct from the workload walls above.
Admission/checkpoint SQL work is sampled and reported as separate families. Sampled process RSS
is not an OS peak; unmodified Darwin`time -l` maximum-resident/footprint fields are retained for
each process, without guessing a unit conversion. PostgreSQL container RSS is not isolated.
[Source and exact commands](raw/workflow-primitives/20261017-runtime-flat/manifest.json),
[width1](raw/workflow-primitives/20261017-runtime-flat/collection-10000-c1.md),
[width16](raw/workflow-primitives/20261017-runtime-flat/collection-10000-c16.md), and
[width256](raw/workflow-primitives/20261017-runtime-flat/collection-10000-c256.md) retain full
settings, operation quantiles, journal sizes and raw process evidence. Nested/low-quota progress,
100k nested instances and release gates remain open; this does not close all of T014-042.
