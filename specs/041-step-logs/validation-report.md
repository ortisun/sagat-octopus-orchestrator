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
