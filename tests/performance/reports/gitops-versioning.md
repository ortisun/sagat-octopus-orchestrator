# Benchmark: GitOps and versioning (feature 011)

Measured by `tests/performance/gitops-versioning/main.rs`. Hypotheses, not promises: figures are valid only for this environment.

## Environment

- CPU: Apple M3 Max (16 logical cores)
- OS: Darwin 25.3.0 arm64
- Load average when the run began: 22.35 11.39 6.53 (other work on the machine adds noise)
- Build profile of the test process and services: release
- Commit: acfcdfa plus uncommitted changes to tracked files
- PostgreSQL 18.6 (Homebrew) in a server named by SAGAT_TEST_DATABASE_URL on the same machine; `fsync = on`, `synchronous_commit = on`, `wal_sync_method = open_datasync` (with either of the first two off a commit does not wait for the disk; on macOS the default method does not ask the drive to flush its own cache, so commits here are faster than on the storage of a server: read every write figure below as optimistic)
- Services and the command line run in-process (no network between them and the client; the command line speaks HTTP to the API on the loopback)

## Latency

| Operation | Samples | p50 | p95 | p99 | max |
|---|---:|---:|---:|---:|---:|
| publish a version, 5 nodes | 100 | 0.68 ms | 1.5 ms | 1.9 ms | 2.1 ms |
| publish a version, 100 nodes | 50 | 1.0 ms | 1.4 ms | 1.4 ms | 1.4 ms |
| publish a version, 1000 nodes | 20 | 5.7 ms | 6.2 ms | 6.4 ms | 6.4 ms |
| move the pointer, no triggers | 200 | 0.44 ms | 1.0 ms | 1.3 ms | 2.3 ms |
| move the pointer, 16 event triggers registered again | 200 | 1.1 ms | 1.4 ms | 2.2 ms | 6.0 ms |
| start a run, the pointer still | 300 | 1.1 ms | 1.6 ms | 2.4 ms | 5.0 ms |
| start a run, an operator moving the pointer all the while | 300 | 1.5 ms | 2.2 ms | 3.2 ms | 4.1 ms |
| list the history, newest 50 of 10002 | 200 | 0.26 ms | 0.37 ms | 0.62 ms | 0.80 ms |
| open, approve and publish a request (three calls) | 50 | 1.8 ms | 2.9 ms | 3.8 ms | 3.8 ms |

## Contention and the command line

| Measurement | Result |
|---|---|
| eight operators moving the pointer of one workflow, each expecting the version the others will have changed (5 s) | 6189 moved and 43315 refused as stale in 5.0 s: **1238 moves/s** |
| eight publishers, each on a workflow of its own (5 s) | 12912 versions: **2582 versions/s** |
| start a run while the pointer moves: median slowdown | 1.13 ms to 1.45 ms (**1.29x**), the pointer moved 592 times meanwhile |
| `sagat diff`, 50 workflows none of which exist yet | 5.1 ms |
| `sagat publish`, 50 new workflows (create, then publish) | 106.6 ms (**2 ms** a workflow) |
| `sagat diff --check`, 50 workflows, nothing changed | 3.8 ms |
| `sagat publish`, 50 workflows of which 5 changed | 20.8 ms |

## Hypotheses

| Hypothesis | Result |
|---|---|
| publishing a 5-node version, p95 < 50 ms | 1.5 ms: **met** |
| publishing a 1000-node version, p95 < 500 ms | 6.2 ms: **met** |
| moving the pointer (with or without triggers), p95 < 25 ms | 1.4 ms: **met** |
| moving the pointer slows the median start of a run by less than 1.5x | 1.29x: **met** |
| the history, 50 of 10002 rows, p95 < 20 ms | 0.37 ms: **met** |
| the approval path (open, approve, publish), p95 < 100 ms | 2.9 ms: **met** |
| `sagat publish` of 50 new workflows < 15 s | 106.6 ms: **met** |
| `sagat diff` of 50 unchanged workflows < 3 s | 3.8 ms: **met** |

## Reading the numbers

- Publishing parses and validates the document (schema, graph rules, expressions), canonicalises it, hashes it and stores the version, the compensation plans, the change of the pointer and the dependencies in one transaction: the cost grows with the document, as the 5, 100 and 1000 node rows show. The edit of the draft before it is not part of the figure.
- Moving the pointer takes the workflow row lock, so changes of one workflow are serialised and changes of different workflows are not: that is what the two rates under contention compare. The stale refusals in the first are the compare-and-swap doing its job (`expectedVersion`); they are cheap and are not retried by the benchmark.
- A run is started on the version the pointer holds when it is read; starting does not take the lock the switch takes, so the slowdown is the cost of the database being busy, not of waiting for the switch.
- The history is read through an index on `(tenant_id, workflow_id, created_at DESC, id DESC)`; the rows were inserted by a statement that copies real ones, which is not how they grow. The history has no retention (G194).
- The command line figures include everything a pipeline waits for: reading the manifest, validating every document with the platform's codec, listing the namespace, reading the active version of each changed workflow and, for `publish`, a create and a publish per workflow, each an HTTP call. There is no network between the command line and the API here: add the round trips of a real one (about 4 calls a changed workflow).
- The machine is shared with unrelated workloads (see the load average), so absolute figures vary between runs; the structural claims (one transaction, one lock per workflow, a compare-and-swap) are what the tests prove.
