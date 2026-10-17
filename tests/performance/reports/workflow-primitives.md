# Workflow primitive capacity measurements

These are observations from one controlled local run, not production ceilings or latency SLOs. The correctness assertions are the configured tenant slot bound and the absence of worker ownership while approvals and timers wait.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average at start: 11.87 13.57 10.96
- Build profile: release
- Commit: 3545df0
- PostgreSQL: Docker container on the same host
- SQLx pool: 8 connections
- Services: in-process; no client network latency

## Configuration

- 10000 approval runs, loaded with 32 concurrent callers
- 1000 one-hour timer runs, loaded with 32 concurrent callers
- scheduler batch 200, scheduler concurrency 8, 50 idle ticks
- 64 collection runs contending for 8 tenant slots with 5 concurrent callers
- tenant usage row held for 100 ms to make lock contention observable

## Latency observations

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| approval scheduling delay (run created to request persisted) | 10000 | 34066.5 ms | 38632.7 ms | 38971.8 ms | 39300.4 ms |
| timer scheduling delay (run created to wait persisted) | 1000 | 0.0 ms | 0.0 ms | 0.0 ms | 0.0 ms |
| scheduler tick with all waits idle | 50 | 117.1 ms | 125.0 ms | 129.7 ms | 129.7 ms |
| collection admission accepted under tenant-slot contention | 8 | 116.6 ms | 143.5 ms | 143.5 ms | 143.5 ms |
| collection admission rejected at tenant capacity | 56 | 2.4 ms | 15.0 ms | 26.9 ms | 26.9 ms |

## Throughput and storage

- Approval run creation: 38.9 s, 257 runs/s.
- Approval opening: 27.0 s, 370 approvals/s across 50 scheduler ticks.
- Timer run creation and wait persistence: 4.0 s, 248 runs/s.
- Observed relation growth: 4490 bytes per suspended approval and 2514 bytes per suspended timer. This is a delta over shared PostgreSQL relations and includes index allocation noise.
- Journal row bytes: 458 bytes per approval run and 394 bytes per timer run, measured with `pg_column_size` and excluding index/toast overhead.

## Correctness observations

- 8/64 collection admissions committed; 56 were rejected with the typed tenant-capacity error. Durable tenant usage and persisted iterations both equal 8.
- PostgreSQL exposed 5 lock waiters while the tenant usage row was deliberately held. Admission latencies therefore include a known 100 ms contention interval plus connection-pool and serialization delay.
- 11000 approval/timer steps are durably waiting with no lease token, lease expiry or worker identity. Active worker sessions: 0.

## Limits and failures

- The first default-population attempt was rejected after reaching the platform's default 10,000 active-run namespace quota. The benchmark now grants and configures explicit headroom before loading; this preserves production admission behavior while isolating the wait measurement.
- The successful run did not capture process or PostgreSQL RSS. Relation growth is durable storage, not memory, so no memory-per-wait result is claimed. The benchmark now contains RSS capture for the next isolated run.
- Public start transaction percentiles were not retained in this successful run; only total load time and throughput were retained. The collection admission rows include the complete adapter call, connection-pool wait and the controlled tenant-row lock interval. The benchmark now retains public start samples for the next isolated run.
- The benchmark performs no client retries: every start and collection admission is submitted once. The successful run did not retain the scheduler's skipped/fenced/failed counters, so no scheduler retry/race rate is claimed. The benchmark now records those counters.
- The collection adapter is still a private runtime boundary. This run measures its real PostgreSQL transaction and persisted tenant policy; it does not claim public collection throughput.
- Approval and timer runs use the supported publish/start/scheduler paths. Waiting consumes database rows and scheduler index scans; it does not allocate one process, connection, worker session or broker consumer per wait.
- The shared development host, local container, eight-connection pool and in-process services are not representative of a production cluster. Repeat on target hardware before sizing.
- The controlled slot test measures one tenant and one database. It proves exact enforcement under cross-run contention; it does not establish a cluster-wide throughput floor.
- The 10,000-item collection workload at concurrency 1/16/256 was not run because public collection publication remains closed.
- Nested loops reaching the 100,000-instance ceiling were not run because nested primitive admission remains closed. No memory, latency, journal or retry result is claimed for either missing workload.
- A later repeat intended to add RSS and scheduler counters was invalidated by host pressure: 35 scheduler actions failed while another PostgreSQL suite reported an unexpected EOF, and the filesystem had only 187 MiB free. It did not replace the successful run above.
- No timeout, queue-delay or capacity ceiling is inferred beyond the populations and concurrency recorded above.

## Reproduction

```sh
SAGAT_PERF_REPORT=tests/performance/reports/workflow-primitives.md \
  cargo test -p sagat-tests --test perf_workflow_primitives --release -- --ignored --nocapture
```

`SAGAT_PRIMITIVES_APPROVALS` and `SAGAT_PRIMITIVES_TIMERS` may reduce populations for diagnostics. A reduced run must not replace the default report.
