# Validation report of feature 041 (step logs)

Measurements of design 041 ([docs/designs/041-step-logs.md](../../docs/designs/041-step-logs.md)) against its
success criteria. **These are measurements of one machine on one day, not promises**; each section states the machine,
its load and the PostgreSQL configuration.

## 2026-11-19 — T041-31, the ingest spike before the schema

**Result: SC-041-007 fails for workload (a); by the rule of T041-31 the design changes and is reviewed again before
T041-5 starts.** The owner question this raises is O041-036 in [research.md](research.md).

### How it was measured

`SAGAT_PERF_STEP_LOG_SPIKE=1 SAGAT_PERF_SCENARIOS=step_log_ingest_spike tests/performance/scalability-ha/run.sh`
runs the scenario `step_log_ingest_spike` of `tests/performance/scalability-ha/main.rs` (driver in
`tests/performance/scalability-ha/step_log_spike.rs`) against a PostgreSQL the script starts with the configuration below.
`SAGAT_PERF_STEP_LOG_DRIVER=on|off` keeps one side of the comparison; both run by default. Nothing it creates reaches a
migration: the tables are the DDL of data-model.md §2 in a scratch schema `spike_041`, the line table carries the two
ADR-0027 triggers charging its text to the existing `log` class (so every commit takes the log policy fence and the
tenant usage row, as the `step_log` class will), and the SQL is T1 to T10 of data-model.md §8 (14 round trips) with
the lease, session, binding and membership read from the real tables. The step-log resources do not exist yet: their
six policy fences are taken by their keys and their defaults used. Two departures from data-model.md §8, both stated:
the session check accepts any minor (1.7 does not exist yet), and no marker row is written (no workload meets a
ceiling but (b), where the marker is one row of 128 bytes). The tenant's `tenant_log_bytes` stands in for
`tenant_step_log_bytes` and is set to the catalogue's maximum (2^50), so no batch takes the over-quota path.

Workloads, on the benchmark's in-process API replicas (each with a pool of 20 connections, as the engine's default):

- (a) every step sends one batch of ten 200-byte `info` lines and then an empty final batch, inside a final flush
  bounded at 2 s (a refused batch is sent again after 200 ms, as the SDK does), then completes with a 1 KiB output that
  differs per task; the baseline (off) completes with the same output and sends nothing. Rows "1 replica, 16 workers"
  and "8 replicas, 128 workers", 3,000 runs of three chained activities each.
- (a') not a workload of the design, recorded beside (a): the ten lines sent in the final batch itself (one ingest
  per step), which is what the SDK of `sdks/SPEC.md` sends for a step shorter than its one-second flush.
- (a) with chunk rows: one row per batch holding its records (O041-021).
- (b) full batches of 500 × 200-byte records from tickets × R attempts of one tenant at once, until the 1 MiB attempt
  ceiling, 3 rounds.
- (c) a fan-out: 10 runs of a `forEach` over 200 items at concurrency 64 whose body is one logging step, 64 scoped
  workers on one replica, the scheduler's passes ticking every 20 ms.
- (d) workload (a), 1 × 16, while another connection holds `BEGIN; SELECT pg_current_xact_id();` for the whole row.
- (e) workload (a), 1 × 16, with a Logs class cap of 1 and one ticket, so ingest sheds.

Admission: per replica a Logs class cap and per-tenant tickets; no call waits (`RETRY_LATER` at once). The tickets were
set from a calibration row (1 × 16 with cap = tickets = 10, half the pool, the most the design allows): tickets =
ceil(batch rate × p99 of S / 0.7).

### Environment

- CPU: Apple M3 Max (16 logical cores), 48 GiB of memory; macOS (Darwin 25.3.0 arm64); release build; commit 0856adad
  plus the uncommitted spike driver of this change.
- PostgreSQL 18.6 (Homebrew), on the same machine as the services: `shared_buffers=2621448kB`,
  `effective_cache_size=20971528kB`, `work_mem=16384kB`, `max_connections=400`, `max_wal_size=16384MB`,
  `checkpoint_timeout=900s`, `wal_level=replica`, `fsync=on`, `synchronous_commit=on`, `full_page_writes=on`.
- **Load: the machine was shared** with another agent's builds and test suites and a virtual machine; the load average
  was 23.7 / 22.1 / 26.5 when the scenario began and moved between 11 and 60 during the runs of the day. The baseline
  of the same row moved by a third between runs (1 × 16 off: 1,141, 1,026 and 873 transitions/s). The figures carry
  that noise; the structural ones (how many transactions a step adds) do not.

### The run of record (3,000 runs per row)

Complete and final flush are per call; S is the service time of one ingest transaction that ran to its end; commit
waits are 10 ms samples of a non-ingest backend waiting on a lock in `COMMIT` (the tenant usage row), and of those
whose blocker was an ingest backend.

| Row | Transitions | Transitions/s | Complete p50 / p95 / p99 | Final flush p50 / p95 / p99 | S p50 / p99 | Batches | RETRY_LATER for a ticket | RETRY_LATER for a lock timeout | Quota retries | Flushes given up | Commit waits / by ingest |
|---|---:|---:|---|---|---|---:|---:|---:|---:|---:|---|
| calibration 1×16 | 9000 | 682 | 6.8 / 8.9 / 18.0 ms | 5.4 / 7.1 / 49.3 ms | 2.69 / 5.13 ms | 18056 | 0.22% (0.00% of finals) | 0.09% (0.00% of finals) | 0 | 0 | 423 / 300 |
| (a) 1×16 off | 9000 | 873 | 6.8 / 8.5 / 22.4 ms | 0.0 / 0.0 / 0.0 ms | 0.00 / 0.00 ms | 0 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 346 / 0 |
| (a) 1×16 on | 9000 | 492 | 7.0 / 10.9 / 39.5 ms | 5.0 / 7.8 / 52.1 ms | 2.48 / 9.57 ms | 18022 | 0.10% (0.00% of finals) | 0.03% (0.00% of finals) | 0 | 1 | 387 / 225 |
| (a') 1×16 on, lines in the final batch | 9000 | 790 | 7.4 / 10.6 / 19.7 ms | 2.9 / 4.3 / 12.2 ms | 2.88 / 10.46 ms | 9007 | 0.08% (0.08% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 234 / 106 |
| (a) 1×16 on, chunk rows | 9000 | 585 | 6.9 / 9.9 / 51.1 ms | 4.6 / 6.6 / 22.1 ms | 2.27 / 6.01 ms | 18025 | 0.12% (0.00% of finals) | 0.02% (0.00% of finals) | 0 | 0 | 636 / 443 |
| (a) 8×128 off | 9000 | 425 | 58.4 / 92.0 / 125.4 ms | 0.0 / 0.0 / 0.0 ms | 0.00 / 0.00 ms | 0 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 285 / 0 |
| (a) 8×128 on | 9000 | 369 | 56.4 / 103.5 / 173.5 ms | 39.5 / 82.1 / 147.3 ms | 19.46 / 88.92 ms | 18002 | 0.01% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 855 / 231 |
| (a') 8×128 on, lines in the final batch | 9000 | 419 | 54.4 / 77.0 / 137.5 ms | 19.4 / 34.5 / 87.2 ms | 19.35 / 83.81 ms | 9006 | 0.07% (0.07% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 523 / 165 |
| (a) 8×128 on, chunk rows | 9000 | 415 | 54.3 / 80.5 / 140.0 ms | 37.6 / 60.1 / 114.5 ms | 18.61 / 65.12 ms | 18001 | 0.01% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 839 / 337 |
| (c) fan-out 10×200, 1×64 | 2000 | 69 | 6.0 / 11.5 / 19.0 ms | 4.1 / 9.2 / 16.5 ms | 2.02 / 8.80 ms | 4000 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 0 / 0 |
| (d) 1×16 on, a transaction held | 9000 | 639 | 8.8 / 12.4 / 17.2 ms | 6.1 / 8.9 / 12.9 ms | 2.97 / 7.09 ms | 18014 | 0.08% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 251 / 115 |
| (e) 1×16 on, cap 1, 1 ticket | 9000 | 170 | 3.6 / 4.7 / 7.1 ms | 2.3 / 405.0 / 1208.0 ms | 1.13 / 2.25 ms | 21491 | 16.50% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 28 | 1 / 1 |

### Against the criteria

| Criterion | 1 × 16 | 8 × 128 | Verdict |
|---|---|---|---|
| SC-041-007 rate, (a) on / off ≥ 90% | 492 / 873 = 56.4% | 369 / 425 = 86.8% | **fail** |
| SC-041-007 completion p99, (a) on ≤ off + 10% | 39.5 ms / 22.4 ms = +76.1% | 173.5 ms / 125.4 ms = +38.4% | **fail** |
| (a) with chunk rows, rate / p99 | 67.0% / +128.1% | 97.7% / +11.7% | **fail** (1 × 16) |
| (a') lines in the final batch, rate / p99 (no criterion of its own) | 90.5% / −12.3% | 98.4% / +9.6% | within, once |
| Fan-out: RETRY_LATER for a lock timeout < 1% | 0.00% of 4,000 batches | | **pass** |
| Tickets ≤ half the pool (10) | calibration: 1,364 batches/s, S p99 5.13 ms → 10 | | within, at the edge |

Earlier runs of the same day, (a) only, same rows: rate 75% and 75% at 1 × 16, 86% and 82% at 8 × 128; completion p99
+23% and +34% at 1 × 16, −8% and +24% at 8 × 128; the calibration of one of them needed 17 tickets (S p99 8.3 ms at
1,401 batches/s), above half the pool.

Recorded with no threshold: the final flush of (a) adds p50 5.0 ms / p99 52.1 ms at 1 × 16 and 39.5 / 147.3 ms at
8 × 128 (two transactions); one batch (a') adds 2.9 / 12.2 ms and 19.4 / 87.2 ms. (b): 330 transactions of 500 records,
S p50 62.1 ms, p95 161.0 ms, p99 231.7 ms, no ticket refusal and no lock timeout; the 500 per-row quota triggers and the
deferred flush calls make a full batch about 25 times a 10-record one. (d): `quota_transaction_deltas` grew from
4,194,304 to 12,279,808 bytes while a transaction was held for 14.1 s; the completion p99 was 17.2 ms. (e): 170
transitions/s, completion p99 7.1 ms, 16.5% of batches refused for a ticket and 28 final flushes given up at 2 s.

### What the numbers say

- The completion's wait on the tenant usage row is small: the samples put it at about 0.25 ms per completion on
  average at both rows, against a p99 excess of 17 ms (1 × 16) and 48 ms (8 × 128). By the rule of T041-31 the failing
  p99 is not put to the owner as O041-026 (the excess is not more than half wait on the tenant row).
- Chunk rows do not pass at 1 × 16, so by the rule of T041-31 the failure is not put to the owner as O041-021: one row
  per batch saves little when a batch holds ten lines; the cost is the transaction, not the rows.
- What fails is the shape of workload (a): two more transactions per step, sent before the completion, in a closed loop.
  With one transaction per step (a') both rows were within the criterion in the one run made, at the edge.
- By T041-31, "If a figure fails, stop: the design changes and is reviewed again before T041-5 starts." T041-5, T041-6
  and the tasks after them wait; the change is the owner's question O041-036.
