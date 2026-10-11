# Baseline benchmark: platform foundation (feature 001)

Measured by `tests/performance/platform-foundation/main.rs`. Hypotheses, not promises: figures are valid only for this environment.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- Memory: 48 GiB
- OS: Darwin 25.3.0 arm64
- Build profile of the test process and API: release
- Commit: a20c570
- PostgreSQL 16 in a Docker container on the same machine (Docker Desktop VM on macOS)
- API, client and database share this machine, so the numbers exclude any real network

## Workload

- 1000 namespaces in one tenant; authenticated requests (JWT verified on every request) with 32 concurrent clients
- 2 s warm-up, then 10 s measured per scenario
- Rate limit and in-flight cap raised so they do not mask the system; database pool of 8 connections

## Results

| Scenario | Requests | Errors | Throughput | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|---:|---:|
| namespace read (GET /namespaces/{id}) | 23945 | 0 | 2392 req/s | 13.1 ms | 15.0 ms | 21.5 ms | 36.0 ms |
| namespace list (GET /namespaces?limit=50) | 21193 | 0 | 2118 req/s | 14.8 ms | 17.3 ms | 22.7 ms | 32.5 ms |

Target (plan): namespace read p95 < 250 ms. Result: **met**.
