- CPU: Apple M3 Max (16 logical cores), 48 GiB RAM
- OS: Darwin 25.3.0 arm64
- Load average at start: 25.90 25.84 23.39
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

### 10000 items, maxConcurrency 16

- Outcome: run status Succeeded, completed; wall 980.4 s, 20002 scheduler transitions, 10.2 items/s (items checkpointed per second), 20.4 transitions/s.
- Peak active iterations observed (sampled every 8 transitions): 15; configured maxConcurrency 16; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 4 threads, 138 MiB RSS, 4 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 4 threads, 120 MiB RSS, 5 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 4, peak PostgreSQL connections 5 (sampled every 256 transitions).
- Durable size: 2334 journal bytes per item (`pg_column_size`, no index/toast), 20002 step rows, 10000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 20.5, 17.0, 13.2, 13.6, 14.1, 13.6, 13.1, 14.7, 14.8, 20.0

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 10000 | 15.4 ms | 36.7 ms | 97.7 ms | 2041.3 ms |
| scheduler: checkpoint one iteration | 10000 | 14.3 ms | 33.2 ms | 94.4 ms | 758.3 ms |
| scheduler: enter (snapshot the items) | 1 | 3863.1 ms | 3863.1 ms | 3863.1 ms | 3863.1 ms |
| scheduler: finish the collection | 1 | 863.1 ms | 863.1 ms | 863.1 ms | 863.1 ms |
| scheduler: nothing to advance (waiting for a worker) | 625 | 4.3 ms | 10.1 ms | 22.9 ms | 134.4 ms |
| worker: complete one scoped task | 10000 | 16.4 ms | 34.7 ms | 92.3 ms | 1108.1 ms |
| worker: poll that leased tasks | 625 | 310.3 ms | 587.5 ms | 1916.9 ms | 3387.6 ms |
| worker: poll with nothing ready | 625 | 5.4 ms | 9.8 ms | 25.3 ms | 291.4 ms |

What the database did for single sampled transitions (load-independent):

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 1666 | 23.2 ms | 57 | 47 | 365 |
| admit | 3332 | 13.0 ms | 57 | 51 | 284 |
| admit | 4998 | 12.6 ms | 57 | 55 | 307 |
| admit | 6664 | 15.9 ms | 57 | 59 | 400 |
| admit | 8330 | 38.6 ms | 57 | 63 | 340 |
| admit | 9996 | 23.4 ms | 57 | 67 | 348 |
| admit | 11662 | 17.8 ms | 57 | 71 | 362 |
| admit | 13328 | 14.8 ms | 57 | 75 | 446 |
| checkpoint | 14994 | 14.2 ms | 49 | 83 | 413 |
| checkpoint | 16660 | 13.9 ms | 49 | 77 | 410 |
| checkpoint | 18326 | 13.7 ms | 49 | 71 | 396 |
| checkpoint | 19992 | 18.8 ms | 49 | 65 | 362 |

