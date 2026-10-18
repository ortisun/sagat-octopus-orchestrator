- CPU: Apple M3 Max (16 logical cores), 48 GiB RAM
- OS: Darwin 25.3.0 arm64
- Load average at start: 16.83 20.91 21.06
- Docker VM: 16 CPUs, 8318709760 bytes
- Build profile: debug
- Commit: b1855cbe
- PostgreSQL: PostgreSQL 16.15 on aarch64-unknown-linux-musl, compiled by gcc (Alpine 15.2.0) 15.2.0, 64-bit
- Setup/observer role: postgres; worker/scheduler role: sagat_app (verified by fixture)
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

- Outcome: run status Succeeded, completed; wall 860.1 s, 20002 scheduler transitions, 11.6 items/s (items checkpointed per second), 23.3 transitions/s.
- Peak active iterations observed (after every admission and every 8 transitions): 16; configured maxConcurrency 16; tenant policy slots 16. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 6 threads, 38 MiB RSS, 6 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 5 threads, 121 MiB RSS, 7 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 6, peak PostgreSQL connections 8 (sampled every 256 transitions).
- Sampled peak RSS: 120.6 MiB (before/after and every 256 transitions; not the OS process peak).
- Durable size: 2334 journal bytes per item (`pg_column_size`, no index/toast), 20002 step rows, 10000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 11.8, 15.2, 16.7, 18.1, 13.4, 13.2, 12.1, 11.6, 9.9, 8.8

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 10000 | 13.1 ms | 31.4 ms | 54.0 ms | 562.9 ms |
| scheduler: checkpoint one iteration | 10000 | 12.6 ms | 28.8 ms | 44.5 ms | 738.0 ms |
| scheduler: enter (snapshot the items) | 1 | 4116.7 ms | 4116.7 ms | 4116.7 ms | 4116.7 ms |
| scheduler: finish the collection | 1 | 841.9 ms | 841.9 ms | 841.9 ms | 841.9 ms |
| scheduler: nothing to advance (waiting for a worker) | 625 | 3.5 ms | 7.7 ms | 10.6 ms | 39.0 ms |
| worker: complete one scoped task | 10000 | 14.6 ms | 30.9 ms | 49.1 ms | 1080.0 ms |
| worker: poll that leased tasks | 625 | 278.8 ms | 493.8 ms | 763.0 ms | 1294.8 ms |
| worker: poll with nothing ready | 625 | 5.5 ms | 9.0 ms | 16.5 ms | 70.7 ms |
| worker: ready-to-lease queue delay | 10000 | 293.5 ms | 729.0 ms | 989.2 ms | 6110.4 ms |

Work by transition family (families are never mixed in these medians):

- admit: first/last two-probe median rows 49/75.
- checkpoint: first/last two-probe median rows 86/65.

What the database did for single sampled transitions:

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 1666 | 13.5 ms | 57 | 47 | 330 |
| admit | 1667 | 11.9 ms | 57 | 49 | 328 |
| admit | 3332 | 22.3 ms | 57 | 51 | 294 |
| admit | 3333 | 29.0 ms | 57 | 53 | 311 |
| admit | 4998 | 15.9 ms | 57 | 55 | 392 |
| admit | 4999 | 21.8 ms | 57 | 57 | 399 |
| admit | 6664 | 25.5 ms | 57 | 59 | 309 |
| admit | 6665 | 28.0 ms | 57 | 61 | 320 |
| admit | 8330 | 12.3 ms | 57 | 63 | 400 |
| admit | 8331 | 13.6 ms | 57 | 65 | 420 |
| admit | 9996 | 12.2 ms | 57 | 67 | 365 |
| admit | 9997 | 11.5 ms | 57 | 69 | 363 |
| admit | 11662 | 24.0 ms | 57 | 71 | 403 |
| admit | 11663 | 29.5 ms | 57 | 73 | 372 |
| admit | 13328 | 11.0 ms | 57 | 75 | 400 |
| checkpoint | 13329 | 9.9 ms | 49 | 86 | 471 |
| checkpoint | 14994 | 15.3 ms | 49 | 83 | 440 |
| checkpoint | 14995 | 11.6 ms | 49 | 80 | 424 |
| checkpoint | 16660 | 10.9 ms | 49 | 77 | 393 |
| checkpoint | 16661 | 11.9 ms | 49 | 74 | 378 |
| checkpoint | 18326 | 9.5 ms | 49 | 71 | 366 |
| checkpoint | 18327 | 11.2 ms | 49 | 68 | 355 |
| checkpoint | 19992 | 9.6 ms | 49 | 65 | 343 |
| checkpoint | 19993 | 10.6 ms | 49 | 62 | 326 |

