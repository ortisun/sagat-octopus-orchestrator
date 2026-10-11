# Baseline benchmark: durable execution (feature 002)

Measured by `tests/performance/durable-execution/main.rs`. Hypotheses, not promises: figures are valid only for this environment.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average when the run began: 41.76 40.92 39.17 (other work on the machine adds noise)
- Build profile of the test process and services: release
- Commit: db77f57
- PostgreSQL 16 in a Docker container on the same machine; database pool of 8 connections
- Services run in-process (no network between them and the client)

## Workload

- 10000 runs of `timer(1h) -> activity`, started with 16 concurrent callers; every one is suspended on a timer
- 100 scheduler ticks with nothing due, then 500 timers made due at once and drained by repeated ticks
- 8 workers claiming (16 at a time) and completing the 500 resulting tasks while the other 9500 runs stay suspended

## Results

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| start a run (create, reserve the timer) | 10000 | 29.5 ms | 59.5 ms | 89.0 ms | 172.8 ms |
| scheduler tick, nothing due, 1000 suspended runs | 100 | 12.2 ms | 46.6 ms | 67.7 ms | 70.9 ms |
| scheduler tick, nothing due, 10000 suspended runs | 100 | 14.3 ms | 37.8 ms | 55.6 ms | 65.1 ms |
| scheduler tick while 500 timers fall due | 3 | 1333.5 ms | 1414.9 ms | 1414.9 ms | 1414.9 ms |
| claim one task (poll of up to 16, 8 workers, 10k suspended runs) | 45 | 16.6 ms | 52.8 ms | 311.5 ms | 311.5 ms |
| complete a step (commit, route, finish) | 500 | 9.5 ms | 19.4 ms | 37.4 ms | 71.0 ms |

- Loading 10000 runs took 22.6 s (443 runs/s).
- Database footprint of a suspended run (runs, steps, journal, payloads, outbox, dedupe, audit with their indexes): **2459 bytes** per run.
- 500 due timers were fired in 3.45 s (145 timers/s, 3 ticks); their 500 tasks were claimed and completed in 1.98 s (252 tasks/s).
- Plan of the due-timer query with 10000 waiting steps: `Limit  (cost=0.29..32.86 rows=200 width=46) /   ->  Index Scan using steps_timer_idx on steps  (cost=0.29..543.21 rows=3333 width=46) /         Filter: (due_at <= clock_timestamp())`.

## Hypotheses

| Hypothesis | Result |
|---|---|
| scheduler tick with nothing due, p95 < 50 ms at 10k suspended runs | 37.8 ms: **met** |
| ten times the runs slows an idle tick (p50) by less than 3x | 12.2 ms to 14.3 ms (1.2x): **met** |
| claiming one task, p50 < 50 ms with 10k suspended runs (the p95 is in the table) | 16.6 ms: **met** |
| completing a step, p95 < 100 ms | 19.4 ms: **met** |

A suspended run holds no process, no connection and no broker consumer: it is the rows counted above. Larger populations (100k, 1M) are later experiments (spec, NFR-002-PERF).

## Reading the numbers

- The machine is shared with unrelated workloads (see the load average above), so absolute figures vary a lot between runs; the claims that matter are structural: the scheduler's queries are index scans whose cost does not grow with the number of suspended runs, and a suspended run costs about 2.4 KiB of rows.
- Timers fall due and fire one transaction at a time under their run's lock, about 100 per second per scheduler tick loop. A burst of 100k timers due at the same instant would therefore take about 17 minutes to fire with one engine; more engines do not multiply it as things stand (they read the same oldest candidates and the losers skip what the winner already fired, which is safe but wasteful), so firing with bounded concurrency inside a tick and giving each engine disjoint candidates are the next steps (feature 010, scale and HA). Nothing is lost while it drains: due timers wait in an index.
- The tail (p99) of claiming grows with the number of workers polling at once, because they all look at the oldest ready runs first and each loser skips the runs the winner holds. Spreading candidates (for example by hashing the worker) is a known optimisation, deliberately not made before it is measured on a quiet machine.
- A poll leases up to 16 steps, each in its own transaction, so the figure above is per claimed task.
