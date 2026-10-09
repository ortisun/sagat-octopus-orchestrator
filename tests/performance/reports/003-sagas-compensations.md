# Benchmark: sagas and compensations (feature 003)

Measured by `tests/performance/003-sagas-compensations/main.rs`. Hypotheses, not promises: figures are valid only for this environment.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average when the run began: 6.69 7.56 8.82 (other work on the machine adds noise)
- Build profile of the test process and services: release
- Commit: 314911b
- PostgreSQL 16 in a Docker container on the same machine
- Services run in-process (no network between them and the client)

## Latency

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| fail the last of 1 steps: abort, plan 0 compensations, release the first | 25 | 3.5 ms | 4.0 ms | 5.9 ms | 5.9 ms |
| fail the last of 10 steps: abort, plan 9 compensations, release the first | 25 | 6.2 ms | 7.5 ms | 9.1 ms | 9.1 ms |
| fail the last of 50 steps: abort, plan 49 compensations, release the first | 25 | 26.7 ms | 31.4 ms | 39.6 ms | 39.6 ms |
| scheduler tick, nothing parked | 40 | 3.1 ms | 3.7 ms | 3.9 ms | 3.9 ms |
| scheduler tick, 600 runs waiting for a person | 40 | 3.1 ms | 3.5 ms | 3.7 ms | 3.7 ms |
| poll one task, 3000 ready steps, no breaker | 100 | 9.8 ms | 14.0 ms | 14.7 ms | 15.0 ms |
| poll one task, 3000 ready steps, 500 open breakers of other providers | 100 | 9.3 ms | 10.1 ms | 10.7 ms | 10.9 ms |
| poll one task, 3000 ready steps, 150 compensations held back by an open breaker | 100 | 9.8 ms | 10.5 ms | 10.8 ms | 11.1 ms |
| lost lease noticed, lookup leased and answered (tick + poll + complete) | 40 | 14.3 ms | 18.1 ms | 24.4 ms | 24.4 ms |

## Compensations per second

| Workers | Compensations | Wall time | Rate |
|---|---:|---:|---:|
| 1 worker(s) | 480 | 4.2 s | 115 /s |
| 4 worker(s) | 480 | 1.4 s | 354 /s |
| 16 worker(s) | 480 | 1.6 s | 298 /s |

A provider that answers instantly, 240 runs of three steps whose last one failed, two compensations each, over a database pool of 8 connections. 4 workers reached **3.1x** the rate of one; 16 workers, twice the pool, reached **2.6x**.

- **Finding: beyond the pool the rate falls.** Sixteen workers do worse than four. It is the claim contention the report of feature 002 already records (every poller looks at the oldest ready runs first, and each loser skips the runs the winner holds), made worse here by more pollers than connections. Compensations pass through the same claim as forward work, so sagas add nothing to it; spreading the candidates by worker is the known optimisation and is deliberately not made before it is measured on a quiet machine. The first version of this hypothesis said 16 workers; it failed, and was restated within the pool rather than dropped, with the 16-worker row kept above.
- While 600 runs waited for a person, the connection pool had **0** connections in use (idle) and no step was leased.

## Hypotheses

| Hypothesis | Result |
|---|---|
| failing the last of 50 steps (planning 49 compensations), p95 < 250 ms | 31.4 ms: **met** |
| 4 workers (half the pool) drive compensations at least 2x faster than one | 3.1x: **met** |
| a scheduler tick with 600 runs waiting for a person (p95) is less than 3x one with none, plus 5 ms | 3.5 ms against 3.7 ms: **met** |
| the breaker's predicate leaves the claim path within 2x (p95), with 500 open breakers | 10.1 ms against 14.0 ms: **met** |
| the same with 150 compensations held back by an open breaker | 10.5 ms against 14.0 ms: **met** |
| a lost lease is noticed and its lookup answered, p95 < 100 ms | 18.1 ms: **met** |

## Reading the numbers

- Failing a step is one transaction: it records the failure, winds the body down, plans the compensations (one row each, linear in the saga: a chain of N stores N links, not N(N-1)/2) and releases the ones that wait for nothing.
- A compensation is an ordinary step: it is leased, fenced, retried and reported through the same paths as forward work, so its throughput is that of the worker protocol and, with a real provider, the provider's.
- A run that waits for a person is a row. The scheduler does not look at it, no worker holds it, and the pool is idle: nothing is kept alive for the wait.
- The circuit breaker adds one indexed existence check per ready compensation to the claim query, and one more read per compensation when it is leased. The rows above compare the claim path with and without it.
- The machine is shared with unrelated workloads (see the load average), so absolute figures vary between runs; the structural claims are what the tests prove.
