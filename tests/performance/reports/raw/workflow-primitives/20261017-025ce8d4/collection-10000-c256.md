- CPU: Apple M3 Max (16 logical cores), 48 GiB RAM
- OS: Darwin 25.3.0 arm64
- Load average at start: 24.91 17.61 18.28
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

### 10000 items, maxConcurrency 256

- Outcome: run status Succeeded, completed; wall 1163.4 s, 20002 scheduler transitions, 8.6 items/s (items checkpointed per second), 17.2 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 255; configured maxConcurrency 256; tenant policy slots 256. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 172 MiB RSS, 4 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 121 MiB RSS, 5 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 5 (sampled every 256 transitions).
- Durable size: 2334 journal bytes per item (`pg_column_size`, no index/toast), 20002 step rows, 10000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 19.6, 21.5, 22.3, 21.4, 21.4, 19.7, 22.1, 17.9, 17.8, 31.7

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 10000 | 20.6 ms | 58.0 ms | 169.4 ms | 812.4 ms |
| scheduler: checkpoint one iteration | 10000 | 21.3 ms | 57.2 ms | 155.6 ms | 827.3 ms |
| scheduler: enter (snapshot the items) | 1 | 3937.7 ms | 3937.7 ms | 3937.7 ms | 3937.7 ms |
| scheduler: finish the collection | 1 | 918.2 ms | 918.2 ms | 918.2 ms | 918.2 ms |
| scheduler: nothing to advance (waiting for a worker) | 40 | 10.5 ms | 20.0 ms | 67.6 ms | 67.6 ms |
| worker: complete one scoped task | 10000 | 17.5 ms | 40.4 ms | 79.9 ms | 399.4 ms |
| worker: poll that leased tasks | 157 | 1366.0 ms | 3007.9 ms | 4137.3 ms | 9676.7 ms |
| worker: poll with nothing ready | 40 | 5.7 ms | 9.3 ms | 12.7 ms | 12.7 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 1666 | 20.9 ms | 57 | 303 | 926 |
| checkpoint | 3332 | 29.7 ms | 49 | 797 | 2859 |
| checkpoint | 4998 | 26.4 ms | 49 | 407 | 1719 |
| admit | 6664 | 19.6 ms | 57 | 59 | 409 |
| admit | 8330 | 28.2 ms | 57 | 319 | 1059 |
| checkpoint | 9996 | 27.1 ms | 49 | 773 | 2946 |
| checkpoint | 11662 | 18.3 ms | 49 | 383 | 1478 |
| admit | 13328 | 17.9 ms | 57 | 75 | 419 |
| admit | 14994 | 16.5 ms | 57 | 335 | 1162 |
| checkpoint | 16660 | 23.4 ms | 49 | 749 | 2617 |
| checkpoint | 18326 | 23.5 ms | 49 | 359 | 1442 |
| checkpoint | 19992 | 15.9 ms | 49 | 65 | 356 |

