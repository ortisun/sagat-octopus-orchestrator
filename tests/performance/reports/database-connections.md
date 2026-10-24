# Managed database phase latency

- Date: 2026-10-17T09:09:17Z
- Revision: a4ca3b7 plus the measured working tree
- OS: Darwin 25.3.0 arm64
- Host: Apple M3 Max; 51539607552 bytes RAM; 16 logical cores
- Load context: host shared with other local development/test jobs; these are observed shared-host timings, not isolated-host capacity measurements.
- Docker: 16 CPUs; 8318709760 bytes memory; 29.8.2
- Profiles: API/engine test harness and native runner debug; PostgreSQL 16 and OpenBao 2.7.1 containers
- Transport: numeric loopback development plaintext, persistent gRPC; fresh target connection per attempt
- Workload: one tenant/namespace/connection, one concurrent run, 10 warmups then 200 sequential successful reads; 1 text parameter and 1 row of 1,024 bytes
- Bounds: operation/connection defaults; runner maxConcurrent=2, per-connection poolSize=1, result ceiling=1 MiB
- No throughput or production TLS/HA latency claim; polling wait and result disclosure are excluded from phase timings.

| Phase (milliseconds) | Samples | p50 | p95 | p99 |
|---|---:|---:|---:|---:|
| Admission (HTTP request through durable receipt) | 200 | 29.467 | 43.588 | 73.542 |
| Secret capability (gRPC, both authorization transactions + OpenBao) | 200 | 30.192 | 49.636 | 60.150 |
| Connection acquisition (DNS policy + socket/auth; OTLP span) | 200 | 25.980 | 27.342 | 28.827 |
| Result retention (CompleteStep RPC, provider scan + durable completion) | 200 | 24.694 | 43.675 | 48.934 |

Every sample asserted successful completion and the retained typed result. Failures: 0. Nearest-rank percentiles; timings include instrumentation overhead. Connection spans have no SQL, credentials or result fields. Result retention measures inline content only; blob and slow-reader limits have separate integration proofs.

Reproduce after building the runner: `cargo test -p sagat-tests --test integration_database_runner measure_managed_database_phase_latency --locked -- --ignored`. Set `SAGAT_DATABASE_PERF_REPORT` to choose the report path.

Response resource probe: [native slow-reader measurement](database-result-memory.md).
