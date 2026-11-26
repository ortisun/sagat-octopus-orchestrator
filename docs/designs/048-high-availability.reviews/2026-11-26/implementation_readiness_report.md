# egm-implementation-readiness on design 048, full round of 2026-11-26

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 1e28351031ae51b7b6625a86f4c4350790ffd2c1
Note: the reviewer's output, condensed by the author; every finding and severity is kept.

## Blocker

1. `commit_safely` cannot be built as described: every site holds a `sqlx::Transaction` (`begin_safely` returns
   `Transaction<'static>`), sqlx 0.9 cannot yield its `PoolConnection` during a pending `COMMIT`, and sqlx returns no
   command tag, which `sqlx-postgres` (not vendored) parses. Fix: give the signature and the type the sites move to, how
   the tag is read, how the detach works.

## Major

1. Task 048-6 cannot merge as one pull request (the proving test, two functions, about 233 commits, about 55 writes, the
   contracts, the checks); the rule would also flag port-level commits in `crates/application` and the customer-database
   commit in `crates/infra-database`. Fix: split it; scope the rule.
2. Class 40 as "not applied" includes 40003; `confirmed_abort` accepts 23, 40001 and 40P01; five module-level
   `CommitOutcomeUnknown` variants exist. Fix: name the SQLSTATEs; say what becomes of those variants.
3. The `25006` rule "by the transaction's mode" has no mechanism: the classifiers take only `sqlx::Error`, and read-only
   transactions are set by `SET TRANSACTION ... READ ONLY`, not by `begin_safely`.
4. Task 048-7 needs the gate reason that Task 048-8 introduces afterwards; say how a reason set fits today's epoch and
   capability gate.
5. `tests/support/replicated_postgres.rs` exists in no task, and no named test has a target or file.
6. Vendoring needs more than one change: `sqlx-core` 0.9.0 has no `socket2` dependency, and `check-vendor.py` is
   hard-coded to rskafka.
7. Tasks 048-2 and 048-3 have no test that can fail; the Patroni and HAProxy configurations, `sagat_app`'s creation, the
   `.env.example` keys and the relation to `compose.yaml` are not listed.
8. Task 048-11's alerts on exporter metrics would fail the contract test that every alerted metric is emitted by the
   platform; the metric names differ between the managers and Compose has no exporter.
9. Task 048-13 does not fit the `scale` job's 60-minute timeout (180 minutes of drill).
10. The conformance scenario cannot be written: faults are count-based, implemented in each SDK's fault engine.
11. The one-run command list is wrong (`cleanup-secrets`) and incomplete, and `migrate`'s confirmation is also claimed by
    design 053's Task 053-4.
12. The API surface is underspecified: no reason or gRPC status for the unconfirmed refusal; `AppError::Unavailable` is a
    unit variant; "retryable only with an idempotency key" needs request context the mapper lacks.

## Nit

1. Deleting `durability_beacons` rows is in no task.
2. Replacing `drill.sh`'s 15 s constant is in no task.
3. Task 048-3 leaves the margin, the check's place and the operator's version open.
4. Task 048-10 names no port, fallback place or test file.
5. Compose cannot tell "the primary's container" from "a node"; the engine's monitor is unnamed; no task records the SDK
   versions.
6. Repeat design 053's and design 051's dependencies on Tasks 048-7 and 048-13.

Counts: Blocker 1, Major 12, Nit 6
