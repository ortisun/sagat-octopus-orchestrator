- CPU: Apple M3 Max (16 logical cores), 48 GiB RAM
- OS: Darwin 25.3.0 arm64
- Load average at start: 6.16 8.18 11.97
- Docker VM: 16 CPUs, 8318709760 bytes
- Build profile: debug
- Commit: 025ce8d4
- PostgreSQL: PostgreSQL 16.15 on aarch64-unknown-linux-musl, compiled by gcc (Alpine 15.2.0) 15.2.0, 64-bit
  - `shared_buffers` = `128MB`
  - `max_connections` = `100`
  - `work_mem` = `4MB`
  - `effective_cache_size` = `4GB`
  - `synchronous_commit` = `on`
  - `fsync` = `on`
  - `max_wal_size` = `1GB`
  - `wal_level` = `replica`
  - `random_page_cost` = `4`

### 10000 items, maxConcurrency 1

- Outcome: run status Succeeded, completed; wall 1580.7 s, 20002 scheduler transitions, 6.3 items/s (items checkpointed per second), 12.7 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 1; configured maxConcurrency 1; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 34 MiB RSS, 4 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 118 MiB RSS, 3 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 5 (sampled every 256 transitions).
- Durable size: 2334 journal bytes per item (`pg_column_size`, no index/toast), 20002 step rows, 10000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 13.3, 14.3, 12.2, 24.0, 37.4, 25.1, 17.2, 12.3, 14.2, 21.9

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 10000 | 16.1 ms | 81.3 ms | 176.8 ms | 2550.9 ms |
| scheduler: checkpoint one iteration | 10000 | 15.7 ms | 78.6 ms | 179.9 ms | 963.3 ms |
| scheduler: enter (snapshot the items) | 1 | 3253.7 ms | 3253.7 ms | 3253.7 ms | 3253.7 ms |
| scheduler: finish the collection | 1 | 962.8 ms | 962.8 ms | 962.8 ms | 962.8 ms |
| scheduler: nothing to advance (waiting for a worker) | 10000 | 4.3 ms | 21.9 ms | 54.0 ms | 263.3 ms |
| worker: complete one scoped task | 10000 | 18.2 ms | 82.1 ms | 189.7 ms | 1009.5 ms |
| worker: poll that leased tasks | 10000 | 25.8 ms | 106.9 ms | 227.0 ms | 802.6 ms |
| worker: poll with nothing ready | 10000 | 5.4 ms | 17.4 ms | 35.2 ms | 294.6 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| checkpoint | 1666 | 14.3 ms | 49 | 41 | 236 |
| checkpoint | 3332 | 13.2 ms | 49 | 41 | 238 |
| checkpoint | 4998 | 13.4 ms | 49 | 41 | 277 |
| checkpoint | 6664 | 19.4 ms | 49 | 41 | 231 |
| checkpoint | 8330 | 17.4 ms | 49 | 41 | 253 |
| checkpoint | 9996 | 54.1 ms | 49 | 41 | 243 |
| checkpoint | 11662 | 45.4 ms | 49 | 41 | 248 |
| checkpoint | 13328 | 12.3 ms | 49 | 41 | 282 |
| checkpoint | 14994 | 11.0 ms | 49 | 41 | 253 |
| checkpoint | 16660 | 15.9 ms | 49 | 41 | 243 |
| checkpoint | 18326 | 19.6 ms | 49 | 41 | 230 |
| checkpoint | 19992 | 14.8 ms | 49 | 41 | 253 |

