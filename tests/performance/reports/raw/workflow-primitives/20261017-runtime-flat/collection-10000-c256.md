- CPU: Apple M3 Max (16 logical cores), 48 GiB RAM
- OS: Darwin 25.3.0 arm64
- Load average at start: 6.90 13.10 17.33
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

### 10000 items, maxConcurrency 256

- Outcome: run status Succeeded, completed; wall 1387.1 s, 20002 scheduler transitions, 7.2 items/s (items checkpointed per second), 14.4 transitions/s.
- Peak active iterations observed (after every admission and every 8 transitions): 256; configured maxConcurrency 256; tenant policy slots 256. Both bounds were asserted at every sample.
- Scheduler errors returned by a transition: 0. No client retries.
- Resources before: 6 threads, 38 MiB RSS, 6 PostgreSQL connections (1 active, 0 idle in transaction).
- Resources after: 5 threads, 120 MiB RSS, 6 PostgreSQL connections (1 active, 0 idle in transaction). Peak threads 6, peak PostgreSQL connections 8 (sampled every 256 transitions).
- Sampled peak RSS: 120.4 MiB (before/after and every 256 transitions; not the OS process peak).
- Durable size: 2334 journal bytes per item (`pg_column_size`, no index/toast), 20002 step rows, 10000 checkpoints.
- p50 (ms) of admit/checkpoint transitions by tenth of the run, first to last: 13.8, 20.8, 19.1, 20.5, 19.4, 15.6, 16.9, 16.7, 28.7, 24.4

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler: admit one iteration | 10000 | 17.7 ms | 62.5 ms | 208.8 ms | 7612.9 ms |
| scheduler: checkpoint one iteration | 10000 | 19.3 ms | 46.3 ms | 91.2 ms | 1761.3 ms |
| scheduler: enter (snapshot the items) | 1 | 3427.1 ms | 3427.1 ms | 3427.1 ms | 3427.1 ms |
| scheduler: finish the collection | 1 | 1047.6 ms | 1047.6 ms | 1047.6 ms | 1047.6 ms |
| scheduler: nothing to advance (waiting for a worker) | 40 | 9.3 ms | 19.8 ms | 83.0 ms | 83.0 ms |
| worker: complete one scoped task | 10000 | 14.8 ms | 51.5 ms | 221.8 ms | 7611.1 ms |
| worker: poll that leased tasks | 157 | 1135.9 ms | 3384.5 ms | 15087.8 ms | 92784.4 ms |
| worker: poll with nothing ready | 40 | 5.4 ms | 12.3 ms | 16.0 ms | 16.0 ms |
| worker: ready-to-lease queue delay | 10000 | 7966.6 ms | 43926.5 ms | 190292.8 ms | 191036.9 ms |

Work by transition family (families are never mixed in these medians):

- admit: first/last two-probe median rows 305/337.
- checkpoint: first/last two-probe median rows 797/65.

What the database did for single sampled transitions:

| Transition | Sequence | Latency | SQL statements | Rows returned/affected | Buffer accesses |
|---|---:|---:|---:|---:|---:|
| admit | 1666 | 14.5 ms | 57 | 303 | 920 |
| admit | 1667 | 14.6 ms | 57 | 305 | 921 |
| checkpoint | 3332 | 23.2 ms | 49 | 797 | 2700 |
| checkpoint | 3333 | 23.1 ms | 49 | 794 | 2692 |
| checkpoint | 4998 | 23.6 ms | 49 | 407 | 1643 |
| checkpoint | 4999 | 18.6 ms | 49 | 404 | 1635 |
| admit | 6664 | 12.5 ms | 57 | 59 | 419 |
| admit | 6665 | 12.8 ms | 57 | 61 | 427 |
| admit | 8330 | 222.7 ms | 57 | 319 | 1164 |
| admit | 8331 | 58.3 ms | 57 | 321 | 1169 |
| checkpoint | 9996 | 19.8 ms | 49 | 773 | 2882 |
| checkpoint | 9997 | 20.5 ms | 49 | 770 | 2870 |
| checkpoint | 11662 | 15.0 ms | 49 | 383 | 1474 |
| checkpoint | 11663 | 15.1 ms | 49 | 380 | 1470 |
| admit | 13328 | 11.0 ms | 57 | 75 | 385 |
| admit | 13329 | 11.1 ms | 57 | 77 | 392 |
| admit | 14994 | 14.2 ms | 57 | 335 | 1067 |
| admit | 14995 | 15.0 ms | 57 | 337 | 1074 |
| checkpoint | 16660 | 31.8 ms | 49 | 749 | 2608 |
| checkpoint | 16661 | 33.8 ms | 49 | 746 | 2595 |
| checkpoint | 18326 | 21.6 ms | 49 | 359 | 1464 |
| checkpoint | 18327 | 20.9 ms | 49 | 356 | 1453 |
| checkpoint | 19992 | 22.7 ms | 49 | 65 | 371 |
| checkpoint | 19993 | 27.6 ms | 49 | 62 | 359 |

