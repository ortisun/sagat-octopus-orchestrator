- CPU: Apple M3 Max (16 logical cores), 48 GiB RAM
- OS: Darwin 25.3.0 arm64
- Load average at start: 22.38 21.85 18.52
- Docker VM: 16 CPUs, 8318709760 bytes
- Build profile: debug
- Commit: f23c60af
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
### Two tenants, one bursting

- Configuration: slot limit 8 per tenant; bursting tenant 6 runs x 40 items, each run `maxConcurrency` 8 (demand 48 against 8 slots); quiet tenant 1 run x 40 items; one worker per tenant that reports each leased task 8 rounds after leasing it; scheduler `tick()` rounds (batch 200, fair, concurrency 8).
- Slot bound asserted after every round for both tenants against the usage counter and against the count of iterations that really hold a slot (`slot_held`): peak counter 8 / holding 8 (bursting), 8 / 8 (quiet), limit 8. The two figures agreed in every round.
- Quiet tenant: finished in 87 rounds beside the burst, 87 alone (ratio 1.00); bound asserted: at most 4x.
- Bursting tenant: finished in 279 rounds; total wall 32.9 s. Scheduler actions skipped for a typed reason (tenant at capacity or revision conflict, no client retry involved): 1081.

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| scheduler tick while both tenants are active | 279 | 47.4 ms | 114.5 ms | 626.2 ms | 693.0 ms |
| scheduler tick, quiet tenant alone (control) | 87 | 30.7 ms | 103.2 ms | 163.1 ms | 163.1 ms |

