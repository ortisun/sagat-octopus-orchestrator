# Benchmark: observability and operations (feature 008)

Measured by `tests/performance/observability-operations/main.rs`. Hypotheses, not promises: figures are valid only for this environment.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average when the run began: 8.91 7.98 6.34 (other work on the machine adds noise)
- Build profile of the test process and services: release
- Commit: 8086b99
- PostgreSQL 18.6 (Homebrew) in a server named by SAGAT_TEST_DATABASE_URL on the same machine; `fsync = on`, `synchronous_commit = on`, `wal_sync_method = open_datasync` (with either of the first two off a commit does not wait for the disk; on macOS the default method does not ask the drive to flush its own cache, so commits here are faster than on the storage of a server: read every write figure below as optimistic)
- Services, projector and a fake collector run in-process (no network between them and the client)

## Latency

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| complete a step, projector idle | 300 | 0.83 ms | 1.4 ms | 1.6 ms | 3.1 ms |
| complete a step, projector working | 300 | 0.86 ms | 1.2 ms | 1.8 ms | 4.5 ms |
| the note's upsert alone (rolled back) | 300 | 0.03 ms | 0.05 ms | 0.06 ms | 0.09 ms |
| GET /runs/{id}/telemetry (service call) | 200 | 0.07 ms | 0.21 ms | 0.37 ms | 0.39 ms |
| GET /runs/{id}/attempts (service call) | 200 | 0.48 ms | 0.94 ms | 1.4 ms | 1.6 ms |
| GET /audit, newest 50 of 10669 records | 200 | 0.26 ms | 0.41 ms | 0.71 ms | 1.2 ms |
| GET /audit?action=run.started, 50 of 10669 records | 200 | 0.27 ms | 0.39 ms | 0.67 ms | 0.94 ms |
| GET /alerts, newest 50 | 200 | 0.07 ms | 0.10 ms | 0.18 ms | 0.24 ms |
| monitor round, 0 live runs, default thresholds (nothing fires) | 20 | 6.5 ms | 12.2 ms | 23.2 ms | 23.2 ms |
| monitor round, 0 live runs, every threshold zero (everything fires) | 20 | 125.4 ms | 129.2 ms | 129.3 ms | 129.3 ms |
| monitor round, 2500 live runs, default thresholds (nothing fires) | 20 | 8.3 ms | 12.7 ms | 18.8 ms | 18.8 ms |
| monitor round, 2500 live runs, every threshold zero (everything fires) | 20 | 147.8 ms | 152.2 ms | 152.5 ms | 152.5 ms |
| monitor round, 5000 live runs, default thresholds (nothing fires) | 20 | 9.6 ms | 12.6 ms | 19.0 ms | 19.0 ms |
| monitor round, 5000 live runs, every threshold zero (everything fires) | 20 | 169.8 ms | 172.5 ms | 172.5 ms | 172.5 ms |
| detect OUTBOX_STUCK, every threshold zero | 20 | 153.9 ms | 158.0 ms | 158.5 ms | 158.5 ms |
| detect LEASE_NOT_REAPED, every threshold zero | 20 | 0.20 ms | 0.51 ms | 0.53 ms | 0.53 ms |
| detect STEP_UNCLAIMED, every threshold zero | 20 | 4.0 ms | 4.4 ms | 4.5 ms | 4.5 ms |
| detect REVIEW_PENDING, every threshold zero | 20 | 0.19 ms | 0.43 ms | 0.55 ms | 0.55 ms |
| detect DEAD_ITEMS, every threshold zero | 20 | 0.20 ms | 0.96 ms | 0.97 ms | 0.97 ms |

## Throughput and lag

| Measurement | Result |
|---|---|
| backlog taken (600 runs of two steps, none projected before) | 600 runs and 1800 spans in 0.5 s over 13 rounds: **1280 runs/s** (backlog before: 600 runs, oldest 3.9 s) |
| sustained writing, 8 callers for 10 s with the projector working | 8896 steps completed in 10.1 s: **880 steps/s** |
| lag during that, sampled every 100 ms | oldest unprojected change: p95 **0.0 s**, max **0.0 s**; most runs waiting at once: 24 |

## Hypotheses

| Hypothesis | Result |
|---|---|
| a busy projector slows the median completion by less than 1.5x | 0.8 ms to 0.9 ms (1.04x): **met** |
| the note's upsert is less than 20% of the median completion | 0.03 ms of 0.8 ms (4%): **met** |
| a backlog is projected at no less than 50 runs/s | 1280 runs/s: **met** |
| under sustained writing the oldest unprojected change stays under the 30 s lag budget | max 0.0 s: **met** |
| a monitor round at 5000 live runs, every condition firing, p95 < 500 ms | 172.5 ms: **met** |
| each new read, p95 < 100 ms | 0.9 ms (the slowest): **met** |

## Reading the numbers

- The write path pays one upsert of a row of its own per transaction that moves the journal. The row is per run, so writers of different runs do not meet on it; writers of the same run already meet on the run's row. The projector reads the notes and the journal in transactions of its own and takes no lock a writer waits for: that is what the second row of the table compares with the first.
- The statement alone was timed in a transaction that was rolled back, so it is a bound on what the note adds and not an A/B of the engine with and without it: the engine has no switch for that and one would not be honest to add only for this.
- The projector's rate is set by the database round trips of a batch of runs (50) and by the collector; here the collector is in the same process and parses the spans, which a real one does elsewhere. A backlog it cannot take in the lag budget is what `TELEMETRY_BEHIND` and the run's `BEHIND` state report; one projector at a time is a design limit (G177, feature 010).
- Lag is measured as the database sees it (`oldest_seconds` of the backlog), at 100 ms, so a figure below that is below the resolution. It includes the time a change waits for the projector's next round.
- A monitor round is a handful of aggregate queries and one reconciliation per kind. With every threshold at zero each ready step, each undelivered outbox row and each lease counts as a condition: the worst case for the detection queries, plus the writes for the few alerts that result (at most 25 per kind per namespace). **This benchmark runs no outbox dispatcher, so every message it ever created is still undelivered** (21344 rows when the detection rows were measured, with 5000 ready steps and 10648 runs in all): the outbox query reads those rows, and it is the figure to look at in the detection rows. A healthy deployment has a few seconds of outbox and not this, but this is the one query whose cost grows with the very failure it detects (a dispatcher that is down): about 9 µs per undelivered message here, so some 10 s of a round at a million. Bounding the scan is the remedy and is not done (G186). The round with default thresholds (nothing old enough to fire) is the cost of looking when all is well.
- The audit reads are measured at 10669 records, with and without a filter, and the reading itself is written in the same transaction (included). The plan was not examined and the trail has no retention (G182): measure again when it holds millions.
- The machine is shared with unrelated workloads (see the load average), so absolute figures vary between runs; the structural claims (a note per run, no lock shared with the projector, aggregate detection) are what the tests prove.
