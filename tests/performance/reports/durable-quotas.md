# Durable quota lock measurement

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average when the run began: 5.60 5.89 6.46 (other work on the machine adds noise)
- Build profile of the test process and services: debug (unoptimised)
- Commit: 3831da0 plus uncommitted changes to tracked files
- Services run in-process (no network between them and the client)
- Database: PostgreSQL 17.11 on aarch64-unknown-linux-musl, compiled by gcc (Alpine 15.2.0) 15.2.0, 64-bit
- Container: Colima sagat-remaining, 4 vCPU / 6 GiB; shared loaded host
- 250 operations per scenario, eight concurrent callers, two rounds; 512-byte unique payloads
- API rate configured to 1,000,000/s for measurement; content/log enforcement enabled

| Scenario | Samples | p50 | p95 | p99 | Max |
|---|---:|---:|---:|---:|---:|
| round 1 / API admission / 1 tenant(s) | 250 | 9.2 ms | 14.3 ms | 152.1 ms | 157.5 ms |
| round 1 / API admission / 2 tenant(s) | 250 | 5.8 ms | 7.4 ms | 8.4 ms | 8.7 ms |
| round 1 / content commit / 1 tenant(s) | 250 | 2.3 ms | 4.1 ms | 9.9 ms | 12.3 ms |
| round 1 / content commit / 2 tenant(s) | 250 | 2.2 ms | 3.1 ms | 3.3 ms | 5.0 ms |
| round 2 / API admission / 1 tenant(s) | 250 | 9.4 ms | 11.6 ms | 12.5 ms | 13.6 ms |
| round 2 / API admission / 2 tenant(s) | 250 | 6.2 ms | 9.4 ms | 10.9 ms | 11.3 ms |
| round 2 / content commit / 1 tenant(s) | 250 | 2.4 ms | 4.0 ms | 5.0 ms | 5.3 ms |
| round 2 / content commit / 2 tenant(s) | 250 | 2.2 ms | 3.1 ms | 3.4 ms | 4.5 ms |

All 2,000 operations committed; each content batch increased exact retained usage by 128,000 bytes, and API-only batches changed no content bytes. These measurements include database round trips, deferred trigger accounting and pool contention, using direct adapter calls. They do not measure full HTTP latency, cloud networking, multi-node HA or a commercial tier. The shared host was running other validation work; compare repeated controlled runs before sizing a deployment. No unmeasured throughput floor is claimed.

Reproduce: `SAGAT_QUOTA_PERF_REPORT=tests/performance/reports/durable-quotas.md cargo test -p sagat-tests --test perf_security_multitenancy durable_quota_hot -- --ignored --nocapture`. Use the documented Docker socket when running in this environment.
