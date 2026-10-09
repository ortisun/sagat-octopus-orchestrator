# Benchmark: security and multi-tenancy controls (feature 009)

Measured by `tests/performance/009-security-multitenancy/main.rs`. Hypotheses, not promises: figures are valid only for this environment.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average when the run began: 12.79 15.31 17.47 (other work on the machine adds noise)
- Build profile of the test process and services: release
- Commit: 1cb0c78
- PostgreSQL 16 in a Docker container on the same machine
- Services run in-process (no network between them and the client)

## Results

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| start a run, 0 active runs in the namespace | 200 | 5.1 ms | 6.5 ms | 7.2 ms | 11.4 ms |
| start a run, 2500 active runs in the namespace | 200 | 5.6 ms | 6.5 ms | 6.8 ms | 7.4 ms |
| start a run, 5000 active runs in the namespace | 200 | 6.2 ms | 7.5 ms | 8.8 ms | 9.2 ms |
| the exact count alone, 5200 active runs in the namespace | 50 | 0.9 ms | 1.0 ms | 1.1 ms | 1.1 ms |
| start a run, 16 callers into ONE namespace | 1600 | 61.2 ms | 66.7 ms | 68.7 ms | 82.6 ms |
| start a run, 16 callers over 16 namespaces of ONE tenant | 1600 | 49.0 ms | 52.9 ms | 55.7 ms | 65.7 ms |
| start a run, 16 callers over 2 tenants | 1600 | 33.7 ms | 49.1 ms | 51.8 ms | 55.5 ms |
| resolve 1 secret (audited, in the lease's transaction) | 60 | 2.5 ms | 4.3 ms | 5.1 ms | 5.1 ms |
| resolve 10 secrets (audited, in the lease's transaction) | 60 | 2.1 ms | 2.4 ms | 2.7 ms | 2.7 ms |
| heartbeat, step declaring none (binding re-checked) | 60 | 2.2 ms | 4.1 ms | 4.8 ms | 4.8 ms |
| complete with a 200 KB output, step declaring none | 60 | 4.4 ms | 5.3 ms | 7.5 ms | 7.5 ms |
| heartbeat, step declaring 1 secret (binding re-checked) | 60 | 2.0 ms | 2.3 ms | 2.5 ms | 2.5 ms |
| complete with a 200 KB output, step declaring 1 secret | 60 | 4.4 ms | 5.4 ms | 6.8 ms | 6.8 ms |
| heartbeat, step declaring 10 secrets (binding re-checked) | 60 | 1.8 ms | 2.1 ms | 2.1 ms | 2.1 ms |
| complete with a 200 KB output, step declaring 10 secrets | 60 | 4.2 ms | 4.9 ms | 7.0 ms | 7.0 ms |

- Starting with 16 callers: into one namespace **260 runs/s**; over 16 namespaces of one tenant **324 runs/s** (1.2x); over two tenants **446 runs/s** (1.7x). Read the first figure with care: that namespace holds most of the table, so each of its starts also pays for counting thousands of rows, which the other two do not. The comparison that isolates the locks is the last two: in both the counts are small; creators in different namespaces of one tenant share the tenant-wide count (a start locks its namespace and then its tenant, and holds both until it commits), while tenants share no lock. Starting over namespaces of one tenant reached 0.73 of the rate over two tenants, which is what the tenant-wide lock costs here at this concurrency; the rest is set by the insert path and the pool of eight connections.
- Plan of the count for a namespace that is a small share of the table (it must use the `runs_active_idx` partial index): `Aggregate  (cost=166.31..166.32 rows=1 width=8) /   ->  Index Only Scan using runs_active_idx on runs  (cost=0.29..166.08 rows=92 width=0) /         Index Cond: ((tenant_id = '01a11f5a-6c3d-7211-bb5b-bdafb7d02d03'::uuid) AND (namespace_id = '01a11f5a-ca1d-7209-aa84-bc1992fd33e7'::uuid))`.
- Plan of the count for the namespace of this benchmark, which holds most of the table (a sequential scan is the right choice there): `Aggregate  (cost=275.47..275.48 rows=1 width=8) /   ->  Seq Scan on runs  (cost=0.00..262.40 rows=5227 width=0) /         Filter: ((finished_at IS NULL) AND (tenant_id = '01a11f5a-6c3d-7211-bb5b-bdafb7d02d03'::uuid) AND (namespace_id = '01a11f5a-6c53-769a-a793-db706fde828b'::uuid))`.
- Exact usage after 5200 starts into the namespace: 5200, equal to what was started.

## Hypotheses

| Hypothesis | Result |
|---|---|
| starting a run with 5 000 active runs in the namespace, p95 < 60 ms | 7.5 ms: **met** |
| the exact count makes a start (p50) less than 3x slower from 0 to 5 000 active runs | 5.1 ms to 6.2 ms (1.2x): **met** |
| resolving 10 secrets, p95 < 60 ms | 2.4 ms: **met** |

## Reading the numbers

- A quota is decided by counting, under an advisory lock on the scope, in the transaction that creates the run. It is exact whatever the number of replicas, and its cost is a count of the namespace's active runs (an index-only scan when the namespace is a small share of the table, which is the normal case; a sequential scan when it is nearly all of it) plus the time the lock is held (the length of the insert). **It grows linearly with the namespace's active runs**: negligible up to the default limit, and the reason a limit far above 100 000 should come with a counter row instead. Creators in different **tenants** do not wait for each other; creators in different namespaces of **one** tenant do, for the length of an insert, because the tenant-wide count is a second scope they share.
- The ceiling for *one* namespace is, in principle, the rate at which one transaction can count and insert; on this machine the measurement does not reach it (see the ratios above). A namespace that needs more should be split, or the count replaced by a transactional counter row (which serialises the same way and adds a decrement on every path a run can end by): not done before it is measured to matter.
- The binding check and the output scan only run for steps that need them; the heartbeat rows show the check costs one indexed read on the transaction's own connection.
- Resolving secrets reads the version pinned by the run, the publisher's role and the access list, decrypts, and writes the audit record, in one transaction under the run's lock.
- The machine is shared with unrelated workloads (see the load average), so absolute figures vary between runs; the structural claims (index-only counting, one lock per scope, no second connection while a transaction is open) are what the tests prove.
