# Validation report of feature 041 (step logs)

Measurements of design 041 ([docs/designs/041-step-logs.md](../../docs/designs/041-step-logs.md)) against its
success criteria. **These are measurements of one machine on one day, not promises**; each section states the machine,
its load and the PostgreSQL configuration.

## 2026-11-19 — T041-31, the ingest spike before the schema

**Result: SC-041-007 fails for workload (a), and one calibration needed more tickets than half the pool. By T041-31
and the design's Open Questions, the design changes and is reviewed again, and nothing after T041-31 starts until
then.** The owner question this raises is O041-036 in [research.md](research.md); T041-3 and T041-4, built on the same
branch nonetheless, are held unticked (O041-037).

### How it was measured

`SAGAT_PERF_STEP_LOG_SPIKE=1 SAGAT_PERF_SCENARIOS=step_log_ingest_spike tests/performance/scalability-ha/run.sh`
runs the scenario `step_log_ingest_spike` of `tests/performance/scalability-ha/main.rs` (driver in
`tests/performance/scalability-ha/step_log_spike.rs`) against a PostgreSQL the script starts with the configuration below.
`SAGAT_PERF_STEP_LOG_DRIVER=on|off` keeps one side of the comparison; both run by default. Nothing it creates reaches a
migration: the tables are the DDL of data-model.md §2 in a scratch schema `spike_041`, the line table carries the two
ADR-0027 triggers charging its text to the existing `log` class (so every commit takes the log policy fence and the
tenant usage row, as the `step_log` class will), and the SQL is T1 to T10 of data-model.md §8 (14 round trips) with
the lease, session, binding and membership read from the real tables. The step-log resources do not exist yet: their
six policy fences are taken by their keys and their defaults used. Departures from data-model.md §8 and T041-31, all
recorded with no threshold of their own: the session check accepts any minor (1.7 does not exist yet); no marker row is
written (no workload meets a ceiling but (b), where the marker is one row of 128 bytes); T9 writes
`ceiling_reason = 'attempt_bytes'` whatever was met and does not split gaps around `ceiling_sequence`; (d) holds its
transaction for the length of the row (14.1 s), not a minute, and reports the completion p99 of that row as "the commit
time of other writers". The tenant's `tenant_log_bytes` stands in for
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

- CPU: Apple M3 Max (16 logical cores), 48 GiB of memory; macOS (Darwin 25.3.0 arm64); release build. The run of
  record ran the driver as committed in 370997d1 (on top of 0856adad); `SAGAT_PERF_STEP_LOG_ONLY=fanout` runs only
  the calibration and workloads (c) to (e), as used while the fan-out was made to work.
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

- The completion's wait on the tenant usage row is about 0.25 ms per completion on average at both rows (samples
  times 10 ms, a lower bound), against a p99 excess of 17 ms (1 × 16) and 48 ms (8 × 128). That compares a mean with a
  tail: the waits sampled at 1 × 16 (at least 2.25 s in all) could explain the whole excess of the slowest completions
  if they sit in them. So the run does not settle T041-31's rule for O041-026 (more than half of the p99 excess waiting
  on the tenant row); it is measured again, per completion, with O041-036.
- Chunk rows do not pass at 1 × 16, so by the rule of T041-31 the failure is not put to the owner as O041-021: one row
  per batch saves little when a batch holds ten lines; the cost is the transaction, not the rows.
- What fails is the shape of workload (a): two more transactions per step, sent before the completion, in a closed loop.
  With one transaction per step (a') both rows were within the criterion in the one run made, at the edge.
- By T041-31 ("If a figure fails, stop: the design changes and is reviewed again before T041-5 starts") and the
  design's Open Questions ("nothing after T041-31 starts until then"), every task after T041-31 waits; the change is
  the owner's question O041-036. T041-3 and T041-4 were built on the branch before this was read as it is written; they
  are held (O041-037).
- The driver of this run let a worker re-take the ticket its previous batch had just released, so the refusals of
  final batches ("0.00% of finals") are understated; and S left out the transactions a lock timeout rolled back. Both
  are fixed in the driver and measured again below.

## 2026-11-19 — T041-31 measured again after the build reviews

The driver as fixed after the reviews of `docs/designs/041-step-logs.reviews/2026-11-18-build/` (a yield before
admission, so a worker no longer re-takes the slot its previous batch released; S counts every transaction that ran,
lock timeouts included; the cap at most half the pool; 30 fan-out runs with the scheduler ticking again at once while
it advances collections; each row runs the baseline again at its end). Same machine and PostgreSQL configuration; the
load average was 23.2 / 12.7 / 10.5 when the scenario began and about 7 to 9 before it. Commit 1bb0b2a6 plus the
driver fixes of the commit that adds this section.

| Row | Transitions | Transitions/s | Complete p50 / p95 / p99 | Final flush p50 / p95 / p99 | S p50 / p99 | Batches | RETRY_LATER for a ticket | RETRY_LATER for a lock timeout | Quota retries | Flushes given up | Commit waits / by ingest |
|---|---:|---:|---|---|---|---:|---:|---:|---:|---:|---|
| calibration 1×16 | 9000 | 676 | 7.4 / 12.0 / 17.4 ms | 6.0 / 10.3 / 16.6 ms | 2.97 / 7.98 ms | 18027 | 0.15% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 71 / 48 |
| (a) 1×16 off | 9000 | 1013 | 7.3 / 8.6 / 10.6 ms | 0.0 / 0.0 / 0.0 ms | 0.00 / 0.00 ms | 0 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 23 / 0 |
| (a) 1×16 on | 9000 | 813 | 6.7 / 9.0 / 13.5 ms | 5.0 / 6.7 / 10.4 ms | 2.37 / 5.03 ms | 18017 | 0.09% (0.01% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 50 / 28 |
| (a') 1×16 on, lines in the final batch | 9000 | 882 | 6.9 / 9.6 / 13.3 ms | 2.8 / 3.9 / 6.5 ms | 2.69 / 6.36 ms | 9002 | 0.02% (0.02% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 31 / 8 |
| (a) 1×16 on, chunk rows | 9000 | 759 | 7.3 / 9.9 / 12.9 ms | 5.0 / 7.0 / 9.8 ms | 2.40 / 4.81 ms | 18013 | 0.07% (0.02% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 27 / 11 |
| (a) 1×16 off, again | 9000 | 901 | 7.4 / 9.3 / 13.2 ms | 0.0 / 0.0 / 0.0 ms | 0.00 / 0.00 ms | 0 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 183 / 0 |
| (a) 8×128 off | 9000 | 495 | 50.9 / 70.2 / 100.0 ms | 0.0 / 0.0 / 0.0 ms | 0.00 / 0.00 ms | 0 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 38 / 0 |
| (a) 8×128 on | 9000 | 433 | 52.0 / 63.1 / 90.6 ms | 38.2 / 45.7 / 68.0 ms | 18.15 / 35.38 ms | 18001 | 0.01% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 95 / 43 |
| (a') 8×128 on, lines in the final batch | 9000 | 408 | 55.2 / 92.4 / 187.0 ms | 20.5 / 39.3 / 97.2 ms | 19.51 / 88.54 ms | 9013 | 0.14% (0.14% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 175 / 52 |
| (a) 8×128 on, chunk rows | 9000 | 453 | 50.6 / 62.0 / 89.8 ms | 36.6 / 44.3 / 72.3 ms | 17.31 / 36.80 ms | 18001 | 0.01% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 37 / 3 |
| (a) 8×128 off, again | 9000 | 503 | 49.5 / 69.6 / 96.0 ms | 0.0 / 0.0 / 0.0 ms | 0.00 / 0.00 ms | 0 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 19 / 0 |
| (c) fan-out 30×200, 1×64 | 6000 | 127 | 7.8 / 12.8 / 18.0 ms | 5.5 / 10.8 / 16.0 ms | 2.63 / 8.34 ms | 12000 | 0.00% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 0 / 0 |
| (d) 1×16 on, a transaction held | 9000 | 704 | 8.0 / 10.2 / 12.7 ms | 5.7 / 7.7 / 10.0 ms | 2.69 / 5.42 ms | 18013 | 0.07% (0.00% of finals) | 0.00% (0.00% of finals) | 0 | 0 | 144 / 54 |
| (e) 1×16 on, cap 1, 1 ticket | 9000 | 219 | 3.1 / 3.9 / 5.3 ms | 1.9 / 403.7 / 1006.3 ms | 1.02 / 1.92 ms | 20864 | 13.77% (1.00% of finals) | 0.00% (0.00% of finals) | 0 | 4 | 1 / 0 |

| Criterion | 1 × 16 | 8 × 128 | Verdict |
|---|---|---|---|
| SC-041-007 rate, (a) on / off ≥ 90% | 813 / 1,013 = 80.3% (against the second baseline, 901: 90.3%) | 433 / 495 = 87.4% (against 503: 86.1%) | **fail** |
| SC-041-007 completion p99 ≤ off + 10% | 13.5 / 10.6 ms = +27.5% (against the second: +2.7%) | 90.6 / 100.0 ms = −9.4% | **fail** at 1 × 16 |
| (a) with chunk rows, rate / p99 | 74.9% / +21.2% | 91.6% / −10.2% | **fail** at 1 × 16 |
| (a') lines in the final batch, rate / p99 | 87.1% / +25.7% | 82.5% / +87.1% | outside, this run |
| Fan-out: RETRY_LATER for a lock timeout < 1% | 0.00% of 12,000 batches | | pass, at 127 transitions/s and with no commit wait seen |
| Tickets ≤ half the pool (10) | calibration: 1,351 batches/s, S p99 7.98 ms → 16 | | **fail** |

Recorded with no threshold: final batches refused for a ticket are now seen (1.00% of finals with one ticket and a cap
of 1, 0.01% to 0.14% otherwise); (b) S p50 61.1 ms, p95 147.8 ms, p99 205.0 ms for 500 records; (d) a transaction held
12.8 s, `quota_transaction_deltas` unchanged at 17,104,896 bytes (its pages were already allocated by the earlier
rows), completion p99 12.7 ms; (e) 219 transitions/s, completion p99 5.3 ms, 4 final flushes given up at 2 s.

What this run adds: the failure of workload (a) holds at both rows, though at 1 × 16 the baseline itself moved by 11%
within the row, so that row's verdict sits inside the noise of this machine; the one-batch shape (a') did not do better
in this run, so the earlier single run of it is not evidence for O041-036 (b) on its own; the ticket gate failed again
(16 against 10). The fan-out still did not load the ingest enough to show a lock wait: the scheduler's passes, not the
ingest, bound it. O041-036 is restated with these figures.

## Quiet rerun (2026-11-19)

The coordinator stopped the other builds on the machine so the spike could be measured quietly. This run uses the same
method as the two runs above, unchanged: `SAGAT_PERF_STEP_LOG_SPIKE=1 SAGAT_PERF_SCENARIOS=step_log_ingest_spike
tests/performance/scalability-ha/run.sh`, release build, the driver at commit a8512cf2 (the fixed driver of the
second run), the same PostgreSQL 18.6 configuration and the same rows. It ran three times in a row. Before each round
the runner waited, for at most 10 minutes, until the 1-minute load average was under 3. Other work still on the
machine (a virtual machine and another project's test suites) was not touched. `LC_ALL=en_US.UTF-8` was set because
PostgreSQL refused to start without a locale in the runner's shell. That setting is environment only, not method.

| Round | Waited for load < 3 | Load at start (1 / 5 / 15 min) | Load at end |
|---|---:|---|---|
| 1 | 45 s | 2.74 / 5.78 / 7.75 | 6.44 / 8.18 / 8.39 |
| 2 | 256 s | 2.95 / 5.15 / 6.95 | 7.53 / 8.49 / 8.10 |
| 3 | 556 s | 2.71 / 4.58 / 6.10 | 6.83 / 8.62 / 7.71 |

Each round started under 3, and the load rose to about 7 during the round. Most of that rise is the benchmark itself
(PostgreSQL, up to 8 replicas and 128 workers on 16 cores).

### Transitions per second and completion p99, per round (3,000 runs of three activities per row)

| Row | Round 1 | Round 2 | Round 3 |
|---|---|---|---|
| 1×16 off | 1,093/s, 12.2 ms | 1,031/s, 14.8 ms | 1,089/s, 10.2 ms |
| 1×16 per-line rows (a) | 854/s, 10.6 ms | 865/s, 10.5 ms | 862/s, 12.4 ms |
| 1×16 lines in the final batch (a') | 977/s, 9.3 ms | 977/s, 9.1 ms | 934/s, 14.5 ms |
| 1×16 chunk rows | 853/s, 10.2 ms | 887/s, 8.4 ms | 816/s, 13.0 ms |
| 1×16 off, again | 1,107/s, 11.5 ms | 1,133/s, 9.0 ms | 1,064/s, 10.1 ms |
| 8×128 off | 540/s, 89.0 ms | 531/s, 85.6 ms | 561/s, 85.3 ms |
| 8×128 per-line rows (a) | 476/s, 78.1 ms | 462/s, 93.7 ms | 476/s, 108.2 ms |
| 8×128 lines in the final batch (a') | 489/s, 84.3 ms | 477/s, 88.5 ms | 480/s, 83.6 ms |
| 8×128 chunk rows | 476/s, 75.8 ms | 476/s, 84.7 ms | 456/s, 91.2 ms |
| 8×128 off, again | 542/s, 78.3 ms | 550/s, 81.9 ms | 545/s, 81.7 ms |

The baseline moved within a row by +1.3%, +9.9% and −2.3% at 1×16, and by +0.4%, +3.6% and −2.9% at 8×128. On the
shared machine it moved by 11% to 33%.

### Against SC-041-007 (on / off, against the first baseline of the row, as the earlier runs)

| Variant | Size | Rate per round | Rate median (spread) | p99 excess per round | p99 median (spread) |
|---|---|---|---|---|---|
| per-line rows (a), the design's | 1×16 | 78.2%, 83.9%, 79.2% | **79.2%** (78.2–83.9) | −12.8%, −29.1%, +22.2% | −12.8% (−29.1 to +22.2) |
| per-line rows (a), the design's | 8×128 | 88.2%, 87.0%, 84.7% | **87.0%** (84.7–88.2) | −12.2%, +9.5%, +26.8% | +9.5% (−12.2 to +26.8) |
| chunk rows | 1×16 | 78.0%, 86.0%, 74.9% | 78.0% (74.9–86.0) | −15.8%, −43.2%, +28.1% | −15.8% (−43.2 to +28.1) |
| chunk rows | 8×128 | 88.1%, 89.7%, 81.2% | 88.1% (81.2–89.7) | −14.8%, −1.0%, +6.9% | −1.0% (−14.8 to +6.9) |
| lines in the final batch (a') | 1×16 | 89.4%, 94.7%, 85.8% | 89.4% (85.8–94.7) | −23.7%, −38.7%, +42.4% | −23.7% (−38.7 to +42.4) |
| lines in the final batch (a') | 8×128 | 90.6%, 89.9%, 85.6% | 89.9% (85.6–90.6) | −5.3%, +3.4%, −2.0% | −2.0% (−5.3 to +3.4) |

Against the second baseline (taken at the end of each row), the per-line rows reach 77.2%, 76.4% and 81.0% at 1×16 and
87.9%, 84.0% and 87.3% at 8×128.

| Criterion, per-line rows | Round 1 | Round 2 | Round 3 |
|---|---|---|---|
| Rate ≥ 90%, 1×16 / 8×128 | fail / fail | fail / fail | fail / fail |
| Completion p99 ≤ +10%, 1×16 / 8×128 | pass / pass | pass / pass | **fail** / **fail** |
| Tickets ≤ half the pool (10) | 13 (1,697 batches/s, S p99 5.34 ms): fail | 15 (1,684/s, 6.05 ms): fail | 11 (1,729/s, 4.19 ms): fail |
| Fan-out: RETRY_LATER for a lock timeout < 1% | 0.00% of 12,000: pass | 0.00% of 12,001: pass | 0.00% of 12,000: pass |

The other workloads, recorded with no threshold:

- (b): S p99 of 240.7, 199.2 and 206.4 ms for 500 records, with no refusal and no lock timeout.
- (c): 135, 136 and 135 transitions/s.
- (d): a transaction held for 11.1 to 12.2 s, `quota_transaction_deltas` unchanged, completion p99 of 12.2 to 14.7 ms.
- (e): 230 to 235 transitions/s, 12 to 14 final flushes given up at 2 s, and 1.02% to 1.33% of final batches refused
  for a ticket.
- Commit waits caused by an ingest, during (a) per-line: 8, 8 and 37 samples at 1×16, and 65, 54 and 177 at 8×128.
  That is at most 0.20 ms per completion on average.

### Reading

On a quiet machine, the design's variant, one row per line, fails SC-041-007 in every round. Its rate with capture on
is 78% to 84% of the rate with capture off at 1×16 (median 79.2%) and 85% to 88% at 8×128 (median 87.0%). The gap is
larger than the baseline's own drift within a row (at most 10% at 1×16, 4% at 8×128). The completion p99 passes in
rounds 1 and 2 and fails in round 3 at both sizes (+22.2%, +26.8%). The ticket rule fails in all three rounds (11 to 15
tickets needed against 10).

T041-31 stays unticked and the design is unchanged. Chunk rows fail the rate too (median 78.0% / 88.1%), so by
T041-31's rule O041-021 is not put to the owner.

O041-026 is still not settled. The p99 fails in only one round of three, and the waits on the tenant row are sampled
as a mean (at most 0.20 ms per completion). That mean cannot show whether more than half of the round-3 excess was
spent there. The failure that holds in every round is the rate, and the rate is not O041-026's trigger.

The one-ingest shape (a') sits at the edge: median 89.4% / 89.9%. It meets the rate in two of six round-rows
(1×16 round 2, 8×128 round 1), and its p99 fails at 1×16 in round 3 (+42.4%). So it does not meet SC-041-007 in all
three rounds either.

For O041-036, which is the owner's to answer: option (a), measuring on a quiet machine, is now done. It shows the
failure is not noise. By the path O041-036 recommends ("(b) if (a') meets the criterion there and (c) or (e) if it
does not"), (a') did not meet the criterion consistently. It fell short by less than one point at the medians.

## 2026-11-20 — T041-35, the spike of the carry (OD-2026-11-19-1)

**Result: T041-35 fails gate (2), the completion p99, at both sizes; gates (1), (3) and (4) pass. By its rule the
design changes and is reviewed again before T041-5 starts.** The owner question is O041-039 in
[research.md](research.md). T041-35 stays unticked.

### How it was measured

`SAGAT_PERF_STEP_LOG_SPIKE=1 SAGAT_PERF_STEP_LOG_CARRY=1 SAGAT_PERF_SCENARIOS=step_log_carry_spike
SAGAT_PERF_CARGO_FEATURES=perf-spike tests/performance/scalability-ha/run.sh`, one invocation per round on a fresh
database, the driver at commit 2c6ad584. The report hook (`sagat-infra-postgres`'s `perf-spike` feature, enabled only by
`sagat-tests --features perf-spike`) runs the carry's statements CA, CB, CC, CC′, CD and CE of data-model.md §8a inside
the real report transaction of each root completion, before its `COMMIT`, against the scratch tables `spike_041`
(their text charged to the `log` class through the ADR-0027 triggers). The plan is inline (a new stream, or a stream
holding one batch, every record kept). Departures, recorded with no threshold of their own: the hook runs right before
the commit, after the journal writes, as section 2a says, but in `PgRunTx::commit` rather than in `finish`; the tenant's
`tenant_log_bytes` stands in for `tenant_step_log_bytes`; the resources' defaults are used. Workloads, 3,000 runs of
three chained activities per measurement, 9,000 transitions each: (a) ten 200-byte lines carried on every completion,
1 KiB output differing per task; (f) the same with output `{}`; (g) one `AppendStepLogs` batch of ten lines (stream lock
in T3), its answer awaited, then a carry of ten more, cap and tickets 10; (h) 64-record carries at 1 × 16; (d) (a) at
1 × 16 with a transaction held for the measurement. Off-1K and off-{} are capture off with the 1 KiB and `{}` outputs,
each taken first and again at the end of its size.

Rule (fixed in T041-35 before the first round, with the waiting rule the coordinator set on 2026-11-19: each round waits
at most 45 minutes for a 1-minute load under 3, else runs marked "not quiet"; at most six attempts): a round is void
when a size's two off figures differ by more than 10% or more than 0.1% of a workload's carries were not written; gates
(1) to (3) pass on the median of three valid rounds (two quiet rounds meeting it), gate (4) in every valid round.

### Environment

Apple M3 Max (16 logical cores), 48 GiB; Darwin 25.3.0 arm64; release build; PostgreSQL 18.6 (Homebrew) on the same
machine, `shared_buffers=2621448kB`, `effective_cache_size=20971528kB`, `work_mem=16384kB`, `max_connections=400`,
`max_wal_size=16384MB`, `checkpoint_timeout=900s`, `wal_level=replica`, `fsync=on`, `synchronous_commit=on`,
`full_page_writes=on`. Other work on the machine (virtual machines, desktop applications, another project's tests) was
not touched. The machine rebooted between attempts 1 and 2.

| Attempt | Waited | Load at start (1 / 5 / 15) | Load at end | Quiet | Valid |
|---|---:|---|---|---|---|
| 1 | 315 s | 2.87 / 4.12 / 4.44 | 17.32 / 14.70 / 9.44 | yes | **void**: off-1K at 8 × 128 drifted 22.2% |
| 2 | 2,220 s | 2.98 / 11.04 / 14.14 | 8.05 / 9.64 / 12.62 | yes | yes (drifts ≤ 0.8%) |
| 3 | 120 s | 2.93 / 7.24 / 11.29 | 9.70 / 8.59 / 10.74 | yes | yes (drifts ≤ 0.9%) |
| 4 | 120 s | 2.94 / 6.34 / 9.56 | 10.57 / 8.58 / 9.63 | yes | yes (drifts ≤ 2.5%) |

Every carry of every valid round was written (9,000 of 9,000 in each workload; none `contended`, none skipped), and the
hook fired once per completion.

### Transitions per second and completion p50 / p99 (ms), valid rounds

| Measurement | Round 2 | Round 3 | Round 4 |
|---|---|---|---|
| 1×16 off-1K | 1,156/s, 6.5 / 7.5 | 1,160/s, 6.5 / 7.4 | 1,166/s, 6.5 / 7.5 |
| 1×16 (a) | 1,080/s, 7.7 / 12.5 | 1,088/s, 7.7 / 10.3 | 1,098/s, 7.7 / 10.5 |
| 1×16 off-{} | 1,166/s, 6.4 / 7.5 | 1,170/s, 6.4 / 7.3 | 1,153/s, 6.4 / 7.5 |
| 1×16 (f) | 1,094/s, 7.4 / 10.6 | 1,101/s, 7.4 / 9.1 | 1,100/s, 7.4 / 9.1 |
| 1×16 (g) | 949/s, 7.5 / 11.5 | 957/s, 7.5 / 9.4 | 953/s, 7.6 / 10.1 |
| 8×128 off-1K | 580/s, 44.0 / 75.1 | 580/s, 43.5 / 84.3 | 596/s, 41.5 / 73.9 |
| 8×128 (a) | 560/s, 51.2 / 92.9 | 572/s, 47.7 / 90.2 | 560/s, 50.4 / 91.4 |
| 8×128 off-{} | 581/s, 42.9 / 71.7 | 588/s, 43.6 / 73.6 | 577/s, 43.1 / 74.3 |
| 8×128 (f) | 580/s, 47.9 / 86.4 | 559/s, 51.0 / 88.3 | 561/s, 50.4 / 88.8 |
| 8×128 (g) | 515/s, 55.0 / 89.3 | 515/s, 54.9 / 92.7 | 512/s, 54.1 / 89.4 |
| 1×16 (h), 64 records | 1,040/s, 8.6 / 19.7 | 1,009/s, 9.0 / 18.0 | 1,051/s, 8.6 / 14.2 |
| 1×16 (d), a transaction held | 1,032/s, 7.6 / 14.8 | 1,027/s, 7.9 / 11.3 | 1,071/s, 7.5 / 10.1 |

### Against the criteria

| Gate | 1 × 16, per round (median) | 8 × 128, per round (median) | Verdict |
|---|---|---|---|
| (1) rate of (a) ≥ 90% of off-1K | 93.5%, 93.8%, 94.2% (93.8%) | 96.4%, 98.8%, 94.0% (96.4%) | **pass** |
| (2) completion p99 of (a) ≤ off-1K + 10% | +66.6%, +38.6%, +40.6% (+40.6%) | +23.8%, +6.9%, +23.7% (+23.7%) | **fail** at both sizes |
| (3) rate of (f) ≥ 90% of off-{} | 93.8%, 94.1%, 95.4% (94.1%) | 99.8%, 95.2%, 97.2% (97.2%) | **pass** |
| (4) tickets for (g) ≤ 10, refusals < 1% | 5, 5, 5 (refusals ≤ 0.07%) | 4, 4, 4 (no refusal) | **pass** |

Recorded with no threshold: the carry's statements CA to CE took p50 1.35 to 1.40 ms and p99 2.29 to 2.31 ms per
completion of (a) at 1 × 16, and p50 5.2 to 5.6 ms and p99 15.1 to 16.2 ms at 8 × 128; (h)'s 64-record carry p50 2.2 to
2.3 ms and p99 2.8 to 3.9 ms, its rate 87% to 90% of off-1K and its completion p99 14.2 to 19.7 ms; (g)'s rate 82% of
off-1K at 1 × 16 and 86% to 89% at 8 × 128 (every step a long step: a batch transaction and a carry), its S p99 3.2 to
3.3 ms and 33.4 to 33.8 ms; (d) held a transaction for 8.4 to 8.8 s, `quota_transaction_deltas` grew from 10.3–10.8 MB to
13.0 MB, its completion p99 10.1 to 14.8 ms. Commit waits sampled every 10 ms during (a) at 1 × 16: 82, 38 and 45, of
which 55, 29 and 28 behind a carrying report (none in off-{}, where no completion takes the tenant usage row); during
(f) at 1 × 16, 46, 28 and 32, of which 37, 15 and 17 behind a carrying report; at 8 × 128 they were 13 to 23 per row.

### Reading

The carry removes what failed T041-31: the rate with capture on is now 93.5% to 99.8% of the rate with capture off in
every valid round, the `{}` output included, and the ticket gate passes with half its allowance. What fails is the tail
of a completion that carries records. At 1 × 16 the capture-off p99 is 7.4 to 7.5 ms, so +10% is 0.75 ms, while the
carry's own five statements take 1.4 ms at the median and 2.3 ms at the 99th percentile, and the completion p50 rises
by about 1.2 ms (6.5 to 7.7 ms): on this machine a five-statement carry cannot meet +10% of a 7.5 ms p99 whatever the
rest does. At 8 × 128 the excess is smaller relative to the p99 (+7% to +24%).

O041-026's condition (the sampled commit waits on the tenant usage row more than half of the p99 excess) is not shown
either way: the samples are a lower bound on the time spent waiting and are not tied to the slowest completions; in
the three rounds at 1 × 16 they total 0.28 to 0.55 s behind carrying reports, of the order of the slowest 1% of
completions' excess (about 90 completions × 3 to 5 ms), and the carry's own statement time is a direct share of the
same size. By T041-35's rule O041-026 is therefore not put, and the change is asked as O041-039.

Not measured, as T041-35 states: the scoped report path (G041-019), a database on another host (G041-022), the CPU of C2
and C3 and the larger gRPC message, a purge beside the workloads.

## 2026-11-20 — T041-35 second run, first job on a hosted runner: not decided

Run 44706547359 of `.github/workflows/perf-spike.yml` at `a34e18df` (D041-102): one GitHub-hosted `ubuntu-latest`
runner, AMD EPYC 9V74 (4 logical cores), 16 GiB, PostgreSQL 18.7, release build, three rounds in a row, every round
quiet by the declared environment. Each round was judged by `carry_spike_judge.py` with the pre-fixed rule (a capture-off
baseline whose rate moves more than 10% between its first and its second measurement voids the round).

| Round | off-1K drift 1 × 16 / 8 × 128 | off-{} drift 1 × 16 / 8 × 128 | Valid | Why |
|---|---|---|---|---|
| 1 | 16.0% / 1.4% | 5.6% / 1.4% | no | off-1K at 1 × 16 drifted 16.0% |
| 2 | 1.5% / 3.1% | 19.2% / 0.0% | no | off-{} at 1 × 16 drifted 19.2% |
| 3 | 2.3% / 0.8% | 11.1% / 1.1% | no | off-{} at 1 × 16 drifted 11.1% |

Verdict: **not decided** (no valid round). The gates of void rounds are not read: a void round decides nothing,
neither a pass nor a fail. The load average went from 5 to 14–16 during each round on 4 cores (the benchmark itself:
PostgreSQL, up to 8 replicas and 128 workers), and every void round failed at 1 × 16, where a row lasts about 11 s and
a short stall of a shared cloud host moves its rate most. The second job follows D041-104. The rounds, their reports
and the verdict are kept as the run's artifact `carry-spike-a34e18dfa7179b2266060628c52cad2089d4f82c`.

## 2026-11-20 — T041-35 second run, second job on four hosted runners: FAIL

**Result: T041-35's second run fails at 1 × 16 on gates (1), (2b), (3) and (4); every gate passes at 8 × 128. By its
rule the design changes and is reviewed again before T041-5 starts.** T041-35 stays unticked; O041-038 and O041-040 are
not put.

Run 44717007853 of `.github/workflows/perf-spike.yml` at `69dfa81b` (D041-104): four GitHub-hosted `ubuntu-latest`
runners in parallel, three rounds each, then the `judge` job over all twelve in the fixed order (shard *s*, round *r* is
round 3(*s* − 1) + *r*). Each runner: 4 logical cores, 16 GiB, release build, the driver of `69dfa81b` (one call of
`spike_041.step_log_carry`, the step-log rows charged once per insert statement, the flush queued once per transaction,
data-model.md §8a); shards 1 and 2 on AMD EPYC 9V74, shards 3 and 4 on AMD EPYC 9V45; PostgreSQL 18.7 in rounds 1 to 3
and 7 to 9, 18.6 in rounds 6 and 10 to 12 (the image's package moved between runners); the configuration of the first
run's table. Every round is quiet by the declared environment (D041-102). The rule, the gates and their thresholds are
those fixed before the first job.

| Round | Shard / round | CPU | off-1K drift 1 × 16 / 8 × 128 | off-{} drift 1 × 16 / 8 × 128 | Valid | Why |
|---|---|---|---|---|---|---|
| 1 | 1 / 1 | 9V74 | 14.5% / 0.4% | 7.3% / 6.8% | no | off-1K at 1 × 16 drifted 14.5% |
| 2 | 1 / 2 | 9V74 | 7.4% / 2.5% | 6.0% / 1.9% | **yes** | |
| 3 | 1 / 3 | 9V74 | 11.1% / 3.0% | 2.6% / 2.8% | no | off-1K at 1 × 16 drifted 11.1% |
| 4 | 2 / 1 | 9V74 | 30.2% / — | 4.5% / — | no | off-1K at 1 × 16 drifted 30.2%; the round stopped in (a) at 8 × 128 (`complete: Unavailable`) |
| 5 | 2 / 2 | 9V74 | 13.8% / — | 5.0% / — | no | off-1K at 1 × 16 drifted 13.8%; the round stopped in (g) at 8 × 128 (`complete: Unavailable`) |
| 6 | 2 / 3 | 9V74 | 23.6% / 5.7% | 1.1% / 5.0% | no | off-1K at 1 × 16 drifted 23.6% |
| 7 | 3 / 1 | 9V45 | 13.6% / 1.8% | 0.6% / 3.5% | no | off-1K at 1 × 16 drifted 13.6% |
| 8 | 3 / 2 | 9V45 | 12.2% / 6.8% | 2.6% / 14.4% | no | off-1K at 1 × 16 drifted 12.2%, off-{} at 8 × 128 14.4% |
| 9 | 3 / 3 | 9V45 | 0.2% / 5.5% | 7.2% / 4.3% | **yes** | |
| 10 | 4 / 1 | 9V45 | 0.6% / 11.8% | 1.7% / 3.4% | no | off-1K at 8 × 128 drifted 11.8% |
| 11 | 4 / 2 | 9V45 | 2.0% / 3.1% | 1.0% / 7.6% | **yes** | |
| 12 | 4 / 3 | 9V45 | 0.7% / 3.8% | 1.3% / 7.9% | yes (not used) | |

Valid rounds: 2, 9, 11 and 12; the judge used the first three, 2, 9 and 11. Every carry of every valid round was
written (9,000 of 9,000 in each workload; none `contended`, none `neither`). Rounds 4 and 5 stopped with a completion
answered `Unavailable` at 8 × 128; they are void by the drift at 1 × 16 whatever the cause, which is not diagnosed
further here.

### Transitions per second and completion p50 / p99 (ms), the three rounds used

| Measurement | Round 2 | Round 9 | Round 11 |
|---|---|---|---|
| 1×16 off-1K | 277/s, 26.2 / 90.3 | 326/s, 23.3 / 54.8 | 353/s, 21.0 / 85.7 |
| 1×16 (a) | 201/s, 30.6 / 294.0 | 281/s, 28.4 / 79.5 | 296/s, 26.7 / 103.8 |
| 1×16 off-{} | 246/s, 25.1 / 205.9 | 360/s, 20.6 / 48.2 | 377/s, 19.5 / 44.5 |
| 1×16 (f) | 231/s, 30.4 / 125.9 | 313/s, 24.6 / 83.7 | 327/s, 23.5 / 84.0 |
| 1×16 (g) | 199/s, 29.9 / 135.5 | 262/s, 24.9 / 99.0 | 241/s, 24.0 / 118.5 |
| 1×16 off-1K again | 256/s, 26.0 / 191.5 | 325/s, 23.0 / 86.3 | 360/s, 20.8 / 71.1 |
| 1×16 off-{} again | 261/s, 24.7 / 160.7 | 334/s, 21.7 / 52.6 | 373/s, 19.6 / 46.9 |
| 8×128 off-1K | 114/s, 248.2 / 945.4 | 128/s, 222.0 / 655.4 | 147/s, 183.1 / 496.6 |
| 8×128 (a) | 111/s, 266.7 / 723.9 | 119/s, 237.7 / 711.7 | 130/s, 216.0 / 535.1 |
| 8×128 off-{} | 118/s, 233.8 / 631.7 | 132/s, 198.1 / 518.9 | 154/s, 173.9 / 438.4 |
| 8×128 (f) | 107/s, 271.2 / 802.3 | 120/s, 251.9 / 2,132.3 | 141/s, 188.5 / 577.3 |
| 8×128 (g) | 99/s, 266.5 / 860.8 | 105/s, 253.1 / 795.6 | 119/s, 215.8 / 517.5 |
| 8×128 off-1K again | 117/s, 235.8 / 713.5 | 121/s, 221.9 / 544.2 | 142/s, 190.6 / 493.1 |
| 8×128 off-{} again | 116/s, 232.5 / 808.2 | 137/s, 190.9 / 467.4 | 142/s, 186.5 / 750.1 |
| 1×16 (h), 64 records | 167/s, 34.1 / 377.1 | 265/s, 28.5 / 113.4 | 280/s, 27.5 / 107.4 |
| 1×16 (d), a transaction held | 199/s, 33.6 / 235.8 | 252/s, 29.4 / 165.6 | 280/s, 27.5 / 95.5 |

On these runners a row runs at a quarter to a fifth of the laptop's rate (277 to 377/s against 1,153 to 1,170/s at
1 × 16) with a completion p50 three to four times higher: the benchmark (PostgreSQL, the replicas and the workers in one
process) loads 4 cores to a load average of 14 to 17.

### Against the criteria

| Gate | 1 × 16, rounds 2 / 9 / 11 (median) | 8 × 128, rounds 2 / 9 / 11 (median) | Verdict |
|---|---|---|---|
| (1) rate of (a) ≥ 90% of off-1K | 72.6%, 86.4%, 83.8% (83.8%) | 97.3%, 93.2%, 88.8% (93.2%) | **fail** at 1 × 16 |
| (2b) completion p99 of (a) ≤ off-1K p99 + max(10%, 2 ms) | +194.7, +19.3, +9.5 ms over the allowance of 9.0, 5.5, 8.6 ms (+19.3 ms) | −316.0, −9.3, −11.2 ms against 94.5, 65.5, 49.7 ms (−11.2 ms) | **fail** at 1 × 16 |
| (3) rate of (f) ≥ 90% of off-{} | 94.1%, 86.9%, 86.8% (86.9%) | 90.9%, 90.9%, 91.8% (90.9%) | **fail** at 1 × 16 |
| (4) tickets for (g) ≤ 10, refusals < 1% | 29, 16, 44 (refusals 0.13%, 0.04%, 0.52%) | 7, 10, 5 (0.02%, 0.13%, 0%) | **fail** at 1 × 16 |

Recorded with no threshold: the +10% reading of gate (2) is +225.5%, +45.2%, +21.1% at 1 × 16 and −23.4%, +8.6%, +7.7%
at 8 × 128; the carry function took p50 1.78, 2.33, 2.18 ms and p99 5.07, 7.51, 6.47 ms per completion of (a) at 1 × 16
(p50 7.2 to 8.4 ms, p99 30.3 to 32.4 ms at 8 × 128); the batch service time S of (g) had p99 100.2, 42.5 and 125.7 ms at
1 × 16 (3.2 to 3.3 ms on the laptop), which is what gate (4)'s ticket count multiplies.

Commit waits (10 ms samples of a backend whose statement is `COMMIT` and which waits on a heavyweight lock; a sample
counts the waiting backends), rounds 2 / 9 / 11 at 1 × 16: off-1K 1,399 / 525 / 1,999 and again 3,484 / 1,091 / 844;
off-{} 0 / 0 / 0 both times; (a) 8,221 / 762 / 2,661; (f) 4,269 / 676 / 692; (g) 3,626 / 1,305 / 4,588; (h) 13,178 /
2,584 / 2,000; (d) 6,851 / 5,609 / 1,227. At 8 × 128: off-1K 15,115 / 9,751 / 893; off-{} 0; (a) 9,753 / 3,632 / 1,345;
(f) 12,445 / 31,951 / 1,044. In the carry rows nearly every blocker is labelled "a carrying report" because every report
of those rows carries; the label does not say that the carry's own statements hold the lock. Off-1K, which carries
nothing, waits in `COMMIT` too, and off-{}, which charges no quota at commit, never does. What the waits are on is the
subject of the diagnosis that follows this run (the cause is proved there, not here).

### Reading

The verdict is FAIL by the rule fixed before the first round. On a 4-core runner the 1 × 16 rows fail every gate and the
8 × 128 rows pass every gate. On the laptop (first run) the 1 × 16 rates passed and the p99 failed at both sizes. (f)
waits in `COMMIT` in every round and size and off-{} never does; (a) against off-1K is not consistent (more waits in
eight of its twelve comparisons with the two off-1K measurements of its round and size, fewer in four). The design changes and is reviewed again (D041-105).

## 2026-11-20 — The commit waits of T041-35's second run, diagnosed: the usage row, but not the cause of the fail

**Result: the commits of T041-35's carrying rows wait on the tenant's `tenant_retained_usage` row, held from ADR-0027's
deferred finalizer to the end of the commit's WAL flush; but moving the step-log charge off that row, or removing it,
changes the 1 × 16 rate by at most 2% and the completion p99 by about 1 ms at the median. The 1 × 16 fail is the
carry's own cost per completion on a CPU-saturated 4-core runner, not the serialization on the row.** Not a gate; the
figures support D041-106.

Run 44730444900 of `.github/workflows/perf-spike.yml` at `5986fca9` on `perf/041-diag-hot-row` (the branch prefix runs
`carry_diag_rounds.sh` instead of the gated rounds): four `ubuntu-latest` runners (4 logical cores, 16 GiB, PostgreSQL
18 of the image, `track_wal_io_timing=on`), each one invocation of `step_log_carry_diag`: three repetitions of nine rows
at 1 replica × 16 workers, 3,000 runs of three chained activities per row, so twelve paired blocks per row. Rows:
off-{} and off-1K (capture off); (f) and (a) as in T041-35 (the step-log charge on the transaction's delta row and the
finalizer at commit, the second run's shape); `f-none` (the carry with no quota charge, an ablation); `f-slot` and
`a-slot` (the charge added, in the statement trigger, to one of 64 per-tenant rows taken `FOR UPDATE SKIP LOCKED`, never
the usage row); (g) and `g-slot`. Every report's `COMMIT` was timed by the report hook (capture-off rows get a hook that
carries nothing), every lock of the usage row was timed by a scratch `AFTER UPDATE` trigger on it, and every 10 ms the
sampler recorded what each backend waiting in `COMMIT` waited for and what each other backend in `COMMIT` was doing.

| Row (1 × 16, medians of 12) | Rate / its baseline [min, max] | p99 − baseline | Waiting in `COMMIT` on the usage row, per sample | Row busy | Lock → `COMMIT` returned p50 / p99 | Back-to-back gap p50 / p99 |
|---|---|---|---|---|---|---|
| (f) / off-{} | 0.893 [0.877, 0.986] | +9.3 ms | 0.111 | 23.1% | 1.10 / 4.04 ms | 0.76 / 4.08 ms |
| `f-none` / off-{} | 0.911 [0.885, 1.063] | +8.0 ms | 0 | — | — | — |
| `f-slot` / off-{} | 0.901 [0.872, 0.920] | +13.3 ms | 0 | — | — | — |
| (a) / off-1K | 0.906 [0.822, 0.917] | +11.0 ms | 0.217 | 23.9% | 1.15 / 4.45 ms | 0.80 / 3.88 ms |
| `a-slot` / off-1K | 0.903 [0.821, 0.918] | +10.6 ms | 0.179 | 23.8% | 1.16 / 4.30 ms | 0.80 / 3.81 ms |
| (g) / off-1K | 0.786 [0.743, 0.805] | +10.7 ms | 0.181 | 68.1% | 1.15 / 3.91 ms | 0.69 / 3.13 ms |
| `g-slot` / off-1K | 0.794 [0.727, 0.816] | +9.7 ms | 0.111 | 20.9% | 1.15 / 4.07 ms | 0.80 / 3.80 ms |
| off-1K / off-{} | 0.966 [0.926, 1.131] | +2.5 ms | 0.095 (off-1K) | 24.0% | 1.09 / 3.73 ms | 0.78 / 3.32 ms |

Paired within a block: (f) / `f-none` 0.982 [0.928, 1.009], p99 +1.1 ms; (f) / `f-slot` 0.993 [0.979, 1.102], p99
−1.3 ms; `f-slot` / `f-none` 0.988 [0.842, 1.006], p99
+3.4 ms; (a) / `a-slot` 1.008 [0.936, 1.020], p99 +0.1 ms. Capture itself, without any quota charge (`f-none` / off-{}):
rate 0.911, completion p50 +5.1 ms, p99 +8.0 ms.

What the waits are on. Every backend waiting in `COMMIT` in the rows that charge the usage row at commit waited on a
row lock of `tenant_retained_usage` (the relation of its tuple lock) or on the transaction holding it
(`transactionid`): in (f) 772 samples named the relation, 4,336 the transaction id, 12 a tuple lock, 863 had already
been granted when their locks were read; no other relation and no advisory lock appeared. In `f-none` and `f-slot`, 4
and 1 samples in the whole run named any lock at all. The committing backends that held no lock spent their samples in
`LWLock:WALWrite` and `IO:WalSync` (0.12 to 0.43 and 0.12 to 0.16 backends per sample), so the hold of the row includes
the holder's WAL flush; a WAL `fsync` took 0.31 to 0.36 ms on average, with 1.25 to 1.31 commits per `fsync`. A carry adds
about 3.1 to 3.5 KiB of WAL per commit (off-{} 4,097 B, (f) 7,226 B, `f-none` 7,633 B; off-1K 4,900 B, (a) 8,356 B).

How long it is held. From the finalizer's lock to the `COMMIT`'s return (an upper bound: the lock is released before the
client sees the answer) p50 1.1 ms and p99 4.0 to 4.5 ms; the gap between two locks when the second report was already
committing (its hold plus the hand-over) p50 0.69 to 0.80 ms and p99 3.1 to 4.1 ms; the row is held 23% to 24% of the time
in (f), (a) and off-1K (about 1.2 ms per lock at 195 locks a second), and 68% in (g), where each step locks it twice (its
batch and its report). The row therefore caps one tenant at roughly 800 charging commits a second on this runner, half
that with a batch per step; at 1 × 16 neither row reaches the cap.

Recorded, no threshold: rounds 4 and 5 of the second job (D041-105) stopped with `complete: Unavailable` at 8 × 128; the
benchmark's replicas acquire a connection with a 2-second timeout (`tests/support/engine.rs:764`), which a 2.1 s
completion p99 (round 9's (f) at 8 × 128) can reach. Not checked further.

## 2026-11-20 — The carry's cost, broken down (D041-107)

**Result: at 1 × 16 the carry's policy fences, stream lock and policy read cost nothing measurable (0.996 of the rate
with capture off); writing the stream row costs 5.0% of the rate (+2.1 ms of completion p50); the ten line rows cost 2.2%
more (+1.3 ms). How the lines are stored (no secondary index, no foreign key, one chunk row, an unlogged table) changes
the rate by less than the noise (0.969 to 0.981 of the stream-only row against 0.978 for the design's rows) but changes
the WAL: 7,874 bytes a commit for the design's rows, 6,980 without the secondary indexes, 5,280 as one chunk row.** Not a
gate; the figures support D041-111.

Run 44741773762 at `5af7c9d2` on `perf/041-diag-cost`: four `ubuntu-latest` runners, each `step_log_carry_diag` with
`SAGAT_PERF_DIAG_SET=cost`: three repetitions of eight rows at 1 × 16 (twelve paired blocks), every row with output `{}`
and the step-log charge `none`, so only the carry's storage differs; then (a) and (g) at 8 × 128 twice each with the
pools not filled first (`-cold`). The carry function reads a mode at its start in every row; the modes were proved to
write only their storage by `step_log_leases_tests.rs::each_carry_mode_of_the_cost_breakdown_writes_only_its_storage`.

| Row (1 × 16, medians of 12) | Rate | p50 / p99 (ms) | WAL a commit | Rate / off-{} [min, max] | Against the row before |
|---|---:|---|---:|---|---|
| off-{} | 289.7/s | 24.4 / 55.8 | 4,172 B | 1 | |
| `c-locks`: fences, stream lock, policy read | 286.8/s | 26.2 / 62.3 | 4,265 B | 0.996 [0.741, 1.078] | |
| `c-stream`: + the stream row (insert, then update) | 264.9/s | 27.8 / 65.9 | 4,895 B | 0.919 [0.876, 1.007] | / `c-locks` 0.950, p50 +2.11 ms |
| `c-full`: + ten line rows (the design) | 254.0/s | 29.6 / 78.1 | 7,874 B | 0.901 [0.813, 0.937] | / `c-stream` 0.978, p50 +1.27 ms |
| `c-noidx`: lines without the two secondary indexes | 260.3/s | 28.2 / 71.3 | 6,980 B | 0.913 [0.871, 0.962] | / `c-stream` 0.981 |
| `c-nofk`: lines without the foreign key | 265.9/s | 28.6 / 78.2 | 7,946 B | 0.914 [0.760, 1.037] | / `c-stream` 0.976 |
| `c-chunk`: one row holding the ten records | 259.4/s | 28.1 / 70.0 | 5,280 B | 0.917 [0.683, 0.957] | / `c-stream` 0.981 |
| `c-unlogged`: the lines unlogged (a measurement point only) | 252.7/s | 28.7 / 66.0 | 5,245 B | 0.895 [0.841, 0.943] | / `c-stream` 0.969 |

The stream row of a short step is written twice in one transaction: inserted, then updated with its counters, its final
mark and its expiry. `expires_at` is a key of `step_log_streams_expiry_idx`, so the update is not HOT and adds a heap
tuple and an entry in each of the table's five indexes that cover it, and it fires the table's guard trigger.

The `-cold` rows: sixteen rows of (a) and (g) at 8 × 128 with empty pools all completed (no `Unavailable`, no
`spike-failure-json`; rates 74 to 145 a second, completion p99 512 to 2,717 ms), so the cause of D041-105's rounds 4 and
5 did not recur and stays unknown (D041-110). The servers' logs hold 1,075 `canceling statement due to lock timeout`
errors (9 in shard 2, 1,066 in shard 3), every one a `COMMIT` of an ingest transaction (its 200 ms lock timeout) waiting
on the tenant's `tenant_retained_usage` row in the finalizer (`INSERT … ON CONFLICT DO NOTHING` or `SELECT … FOR UPDATE`):
at 8 × 128 the usage row turns batches away, which the leases of ADR-0097 remove.

## 2026-11-20 — Hand mutations of the lease prototype (ADR-0097, D041-108, D041-111, D041-112)

The prototype's tests, `tests/performance/scalability-ha/step_log_leases_tests.rs`, run by hand against a disposable
PostgreSQL 18 with `pg_stat_statements` preloaded (`SAGAT_TEST_DATABASE_URL=… cargo test -p sagat-tests --test
perf_scalability_ha --features perf-spike leases_tests`); all 17 pass as committed (D041-113). Each mutation below was applied by
hand to `step_log_spike.rs`, the tests run, the file restored and compared:

| Mutation | Test that failed |
|---|---|
| M1 no `SKIP LOCKED` in the charge | `a_charge_skips_a_lease_another_transaction_holds_and_never_waits_for_it` |
| M2 the charge locks the usage row (a first attempt mutated the wrong statement and survived; redone on the charge) | `a_charge_with_a_free_lease_never_takes_the_usage_row…`, `a_charge_skips…`, `the_last_bytes…` |
| M3 no lowered-limit check | `a_lowered_limit_stops_charges_into_leases_with_room` |
| M4 a grant past the free room | `concurrent_charges_never_exceed_the_limit_and_count_every_byte_once` |
| M5 no reclaim near the limit (a first run survived; the deterministic test `the_last_bytes_of_the_limit_are_admitted_from_the_room_of_idle_leases` was added) | `the_last_bytes_of_the_limit_are_admitted_from_the_room_of_idle_leases` |
| M6 the refill does not fold its lease | `a_charge_with_a_free_lease_never_takes_the_usage_row_and_a_refill_alone_waits_for_it` |
| S1 the new stream is also updated | `a_short_steps_carry_writes_its_new_stream_once` |
| S2 the expiry index keyed on `expires_at` | `a_carry_onto_an_existing_stream_updates_it_hot` |
| C1 the `locks` mode of the cost breakdown writes a stream | `each_carry_mode_of_the_cost_breakdown_writes_only_its_storage` |
| R1 a charge past a lowered limit raises (before the reclaim on a limit change existed) | `a_limit_lowered_above_the_usage_still_admits_what_fits` |
| R2 a release waits for the leases | `a_release_never_waits_for_a_lease_and_never_goes_below_zero` |
| R3 a release with no underflow check | `a_release_never_waits_for_a_lease_and_never_goes_below_zero` |
| R4 the carry's refill waits (no `NOWAIT`) | `a_carry_never_waits_for_the_usage_row` |
| R5 the carry does not catch a refusal | `a_carry_refused_by_the_limit_records_its_mark_in_the_same_call` |
| R6 a change of the limit reclaims nothing | `a_limit_lowered_above_the_usage_still_admits_what_fits` |
| R7 the refill inserts the usage row before its `NOWAIT` lock (D041-113) | `a_carry_never_waits_for_the_usage_row` (its holder updates the row) |
| R8 no fast path (the reservation always fails) | `only_a_carry_that_needs_a_refill_opens_a_subtransaction` |
| R9 a refused long step's mark is not written (a first, weaker version survived: it changed only `last_sequence`; redone so that the stream is never found) | `a_long_steps_carry_refused_by_the_limit_records_its_mark_on_its_stream` |

The budget lines were derived before they were run and seen failing where the derivation missed a statement: a charged
insert derived as 6 nested statements ran 16 (one foreign-key check a line); a short step's carry derived as 22 ran 23
(the stream's path check), then with the no-wait setting derived as 25 ran 24 (the new stream's path returned before a
reset, which was removed), then with the fast path derived as 29 ran 27 (a function called in an `IF` or a `RETURN` is an
expression, not a statement). Not covered by the prototype: the direct charge with all 256 leases held (T041-5, with a
lowered cap), the privileges of ADR-0097 decision 1 (T041-5's runtime-role tests).

## 2026-11-20 — Preview of the third shape at 3f1f0aba (not a gate)

Run 44761982218 on `perf/041-diag-third` (the diagnosis's `third` set: off-1K, (a), off-{}, (f) and (g) at 1 × 16, three
repetitions on each of four runners, twelve paired blocks, the step-log charge on the tenant's leases). The shape is the
one of `3f1f0aba`: every carry ran its writes inside the `BEGIN … EXCEPTION` block (a subtransaction on every carrying
report) and its refill inserted the usage row before its `NOWAIT` lock; D041-113 then gave the carry a fast path with no
subtransaction, which this preview does not measure. Its first attempt (run 44753470534, `0b3f14b3`) stopped on the
prototype's lease pick (`query returned more than one row`).

| Pair (1 × 16, paired medians of 12) | Rate [min, max] | p50 | p99 [min, max] |
|---|---|---|---|
| (a) / off-1K | 0.882 [0.820, 0.924] | +4.73 ms | +17.9 ms [+7.0, +40.8] |
| (f) / off-{} | 0.897 [0.854, 0.921] | +4.21 ms | +21.7 ms [+4.4, +45.3] |
| (g) / off-1K | 0.772 [0.714, 0.802] | +4.82 ms | +19.7 ms [+5.0, +45.4] |

Against the second run's shape in run 44730444900 ((a) / off-1K 0.906, (f) / off-{} 0.893, (g) / off-1K 0.786): no gain at
1 × 16, as T041-35's third-run text expected, with every carry paying a subtransaction here. (g)'s batch service time p99
was 24.2 ms (median of rows) and its commits no longer waited on the usage row for their step-log charge.

## 2026-11-20 — T041-35 third run, four hosted runners: not decided (no valid round)

**Result: not decided. None of the twelve rounds is valid: every round answered more than 0.1% of the carries of at least
one workload with something other than `written`, which voids it by the validity rule fixed before the first job. The
gate values are recorded below as measured; they decide nothing.** T041-35 stays unticked; ADR-0097, ADR-0095 and
O041-038/O041-040 stay as they are. The unwritten carries are not noise: the coordinating session classes them as a
product defect to diagnose (D041-116, G041-025).

Run 44766210424 of `.github/workflows/perf-spike.yml` at `8afb710f` (`SAGAT_PERF_STEP_LOG_CARRY_RUN=3`, the third shape of
T041-35's text): four GitHub-hosted `ubuntu-latest` runners in parallel, three rounds each, then the `judge` over the
twelve in the fixed order (shard *s*, round *r* is round 3(*s* − 1) + *r*). Each runner: 4 logical cores, 16 GiB, release
build; shard 1 on Intel Xeon 6975P-C, shard 2 on AMD EPYC 7763, shard 3 on AMD EPYC 9V45, shard 4 on AMD EPYC 9V74 (four
CPU models, against two in the second run); PostgreSQL 18.7 in rounds 1 to 6 and 18.6 in rounds 7 to 12. Every round is
quiet by the declared environment (D041-102). Every round ran to its end (no `complete: Unavailable`, no
`spike-failure-json` line); the server's log of each round is kept in the shard's artifact.

| Round | Shard / round | CPU | off-1K drift 1 × 16 / 8 × 128 | off-{} drift 1 × 16 / 8 × 128 | Valid | Why |
|---|---|---|---|---|---|---|
| 1 | 1 / 1 | Xeon 6975P-C | 0.0% / 1.9% | 1.9% / 1.3% | no | carries not written (all eight workloads) |
| 2 | 1 / 2 | Xeon 6975P-C | 2.7% / 0.4% | 1.5% / 8.1% | no | carries not written (all eight) |
| 3 | 1 / 3 | Xeon 6975P-C | 3.0% / 0.3% | 3.3% / 5.4% | no | carries not written (all eight) |
| 4 | 2 / 1 | 7763 | 2.0% / 2.7% | 3.2% / 2.5% | no | carries not written (seven; (f) at 8 × 128 0.06%) |
| 5 | 2 / 2 | 7763 | 3.0% / 3.3% | 3.1% / 0.2% | no | carries not written (seven; (f) at 8 × 128 0.09%) |
| 6 | 2 / 3 | 7763 | 1.6% / 1.4% | 1.9% / 4.9% | no | carries not written (six; (f) at both sizes 0.08%) |
| 7 | 3 / 1 | 9V45 | 4.6% / 6.0% | 2.1% / 5.9% | no | carries not written (six; none in (h) and (d)) |
| 8 | 3 / 2 | 9V45 | 0.0% / 11.8% | 6.0% / 16.3% | no | off-1K at 8 × 128 drifted 11.8%, off-{} 16.3%; carries not written |
| 9 | 3 / 3 | 9V45 | 4.2% / 2.0% | 5.2% / 20.6% | no | off-{} at 8 × 128 drifted 20.6%; carries not written |
| 10 | 4 / 1 | 9V74 | 14.2% / 6.8% | 8.3% / 1.6% | no | off-1K at 1 × 16 drifted 14.2%; carries not written |
| 11 | 4 / 2 | 9V74 | 22.2% / 3.8% | 15.5% / 1.2% | no | off-1K at 1 × 16 drifted 22.2%, off-{} 15.5%; carries not written |
| 12 | 4 / 3 | 9V74 | 1.1% / 0.1% | 7.0% / 2.3% | no | carries not written (all eight) |

Ten rounds are within the drift rule; all twelve are void by the carries. Carries answered other than `written`, of 9,000
a workload (the harness of `8afb710f` counts a mark-only answer, `mark_only`, in `neither`; no carry was `contended`):

| Round | (a) 1 × 16 | (a) 8 × 128 | (f) 1 × 16 | (f) 8 × 128 | (g) 1 × 16 | (g) 8 × 128 | (h) | (d) |
|---|---|---|---|---|---|---|---|---|
| 1 | 66 (0.73%) | 75 (0.83%) | 30 (0.33%) | 23 (0.26%) | 62 (0.69%) | 15 (0.17%) | 393 (4.37%) | 99 (1.10%) |
| 2 | 109 (1.21%) | 90 (1.00%) | 24 (0.27%) | 12 (0.13%) | 37 (0.41%) | 27 (0.30%) | 346 (3.84%) | 102 (1.13%) |
| 3 | 57 (0.63%) | 67 (0.74%) | 15 (0.17%) | 12 (0.13%) | 42 (0.47%) | 24 (0.27%) | 374 (4.16%) | 247 (2.74%) |
| 4 | 43 (0.48%) | 51 (0.57%) | 14 (0.16%) | 5 (0.06%) | 20 (0.22%) | 30 (0.33%) | 225 (2.50%) | 95 (1.06%) |
| 5 | 53 (0.59%) | 64 (0.71%) | 13 (0.14%) | 8 (0.09%) | 26 (0.29%) | 15 (0.17%) | 161 (1.79%) | 74 (0.82%) |
| 6 | 55 (0.61%) | 59 (0.66%) | 7 (0.08%) | 7 (0.08%) | 23 (0.26%) | 14 (0.16%) | 198 (2.20%) | 48 (0.53%) |
| 7 | 132 (1.47%) | 180 (2.00%) | 26 (0.29%) | 81 (0.90%) | 74 (0.82%) | 349 (3.88%) | 0 | 0 |
| 8 | 175 (1.94%) | 133 (1.48%) | 42 (0.47%) | 124 (1.38%) | 82 (0.91%) | 236 (2.62%) | 36 (0.40%) | 96 (1.07%) |
| 9 | 133 (1.48%) | 180 (2.00%) | 48 (0.53%) | 93 (1.03%) | 62 (0.69%) | 168 (1.87%) | 187 (2.08%) | 215 (2.39%) |
| 10 | 535 (5.94%) | 864 (9.60%) | 49 (0.54%) | 323 (3.59%) | 113 (1.26%) | 69 (0.77%) | 543 (6.03%) | 534 (5.93%) |
| 11 | 331 (3.68%) | 810 (9.00%) | 48 (0.53%) | 251 (2.79%) | 132 (1.47%) | 165 (1.83%) | 550 (6.11%) | 733 (8.14%) |
| 12 | 778 (8.64%) | 1,905 (21.17%) | 43 (0.48%) | 247 (2.74%) | 153 (1.70%) | 173 (1.92%) | 274 (3.04%) | 204 (2.27%) |

The scenario grants the tenant the maximum `tenant_step_log_bytes`, so no carry can be refused by the limit, and with
`p_records` 10,000 against at most 64 records no carry meets an attempt ceiling: the answers are the slow path's
mark-only (`tenant_step_log_quota_held`, or the refill's `NOWAIT` on a held usage row, `55P03`), and possibly `neither` for a
stream found final, which the harness cannot tell apart. G041-023 expected about 0.08% of `busy` mark-only answers; the
measured rate is 0.06% to 21.17%. The ingests of (g) also met the usage row: the server logs show 30 `lock timeout`
cancellations (the batch's 200 ms `lock_timeout`) in the refill's `FOR UPDATE` of `tenant_retained_usage` (rounds 7, 11
and 12), answered as ticket refusals (0 to 6 a row).

### Gate values as measured (not a verdict)

| Gate | 1 × 16, rounds 1 to 12 | median of 12 | 8 × 128, rounds 1 to 12 | median of 12 |
|---|---|---|---|---|
| (1) rate of (a) / off-1K, ≥ 90% | 82.7, 79.7, 82.9, 90.4, 91.0, 90.7, 84.3, 80.4, 81.8, 75.5, 83.7, 76.7% | 82.8% | 90.5, 91.3, 91.9, 92.1, 92.6, 93.8, 94.8, 92.7, 86.5, 89.0, 96.5, 92.5% | 92.3% |
| (2b) p99 of (a) over off-1K + max(10%, 2 ms), ≤ 0 | +12.8, +16.1, +18.0, +9.9, +7.9, +9.4, +3.1, +15.5, +37.9, +95.3, +90.3, +43.4 ms | +15.8 ms | +61.0, +12.0, +31.4, −3.0, −87.6, +6.1, −7.4, +8.3, +13.3, +234.8, +236.9, +273.7 ms | +12.7 ms |
| (3) rate of (f) / off-{}, ≥ 90% | 86.4, 87.4, 85.7, 91.9, 94.3, 90.4, 86.2, 87.4, 92.0, 92.3, 81.3, 78.5% | 87.4% | 91.2, 98.8, 94.2, 92.2, 97.2, 91.3, 86.3, 78.2, 92.5, 93.8, 95.9, 93.3% | 92.9% |
| (4) tickets for (g) ≤ 10, refusals < 1% | 7, 7, 7, 7, 6, 6, 10, 12, 8, 17, 32, 22 (refusals ≤ 0.07%) | 7.5 | 5, 5, 5, 5, 4, 4, 16, 6, 5, 5, 6, 7 (refusals ≤ 0.06%) | 5 |

The allowance of (2b) was 3.7 to 17.1 ms at 1 × 16 (median 6.0) and 47.1 to 80.7 ms at 8 × 128 (median 61.5). Over the ten
rounds within the drift rule the medians are, at 1 × 16, (1) 82.8%, (2b) +14.2 ms, (3) 87.4%, (4) 7, and at 8 × 128
(1) 92.3%, (2b) +21.7 ms, (3) 93.5%, (4) 5. Read as a gate on the medians of all twelve, 1 × 16 would fail (1),
(2b) and (3) and pass (4) (7.5 tickets, against 29/16/44 in the second run); 8 × 128 would pass (1), (3) and (4) and fail
(2b), which it passed in the second run.

Medians of the twelve rounds (rate; completion p50 / p99; the carry function's p50 / p99; commit-wait samples):

| Row | 1 × 16 | 8 × 128 |
|---|---|---|
| off-1K | 299/s; 25.2 / 59.6 ms; waits 304 | 116/s; 230.4 / 615.3 ms; waits 596 |
| (a) | 236/s; 32.5 / 78.2 ms; carry 2.48 / 6.93 ms; waits 482 | 102/s; 276.4 / 694.0 ms; carry 9.52 / 44.03 ms; waits 669 |
| off-{} | 277/s; 25.9 / 52.4 ms; waits 0 | 113/s; 233.7 / 598.2 ms; waits 0 |
| (f) | 253/s; 30.5 / 73.1 ms; carry 2.02 / 5.73 ms; waits 0 | 101/s; 273.1 / 711.3 ms; carry 9.45 / 44.30 ms; waits 0 |
| (g) | 214/s; 32.6 / 84.3 ms; S p99 25.5 ms; waits 562 | 93/s; 273.4 / 798.1 ms; S p99 284.2 ms; waits 1,718 |
| (h), 64 records | 203/s; 37.7 / 95.3 ms | — |
| (d), a transaction held | 190/s; 42.5 / 92.6 ms | — |

Against the second run (run 44717007853, all its rounds, not only the three it used): at 1 × 16 the commit waits of (a)
fell from a median of 3,208 samples to 482 and (g)'s S p99 from 42.5 to 125.7 ms in its three used rounds to a median of 25.5 ms; the carry
function's p50 / p99 rose from 2.19 / 6.28 to 2.48 / 6.93 ms at 1 × 16 and from 7.86 / 31.71 to 9.52 / 44.03 ms at
8 × 128; (a) − off-1K completion p99 went from a median of +19.7 ms [−82, +204] to +20.6 ms [+11, +105] at 1 × 16 and from
+56.3 ms [−221, +712] (eleven rounds) to +73.4 ms [−7, +354] at 8 × 128. Why 8 × 128 moved is analysed in the following
section, not here.

### Reading

Not decided, by the rule. What the run shows besides: the hot row is gone from the carry's commits (D041-106's cause), and
gate (4) at 1 × 16 is within its threshold in eight of twelve rounds; the leases' slow path answers a share of carries
with a mark-only final mark while the tenant is far from its limit, a share that grows with load and was badly
underestimated by G041-023; 8 × 128's (2b) is positive in nine of twelve rounds. The validity rule and the gates are
unchanged (D041-116).
