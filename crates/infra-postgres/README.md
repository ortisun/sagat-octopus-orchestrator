# crates/infra-postgres

## Overview

sagat-infra-postgres implements PostgreSQL authority storage ports and embeds schema migrations.

## Interface

- PgRunStore/PgRunTx implement durable run/journal/projection/receipt/audit/outbox boundaries. PgCollectionStore is runtime-wired via its definition-cache constructor; ScopeFrontierStore and SecretCleanupStore are active ports. [src/execution.rs:PgRunStore] [src/execution.rs:PgRunTx] [src/primitives.rs:PgCollectionStore::with_definition_cache] [src/scope_frontier.rs:ScopeFrontierStore] [src/secret_cleanup.rs:SecretCleanupStore]
- Variable-size storage classification records content, log and protected_system classes with tenant/measurement evidence. [storage-classification.json:classes] [storage-classification.json:columns]
- [migrations](./migrations/README.md) — Ordered PostgreSQL up/down migrations for authority tables, projections, accounting, operational metadata, indexes, mutation guards and runtime grants. Historical migration comments describe their original checkpoint, not current endpoint availability. [migrations/20261008000000_identity.up.sql:tenants]
- [src](./src/README.md) — PostgreSQL authority adapter implementing application storage ports, guarded transactions, durable journal/projections, quotas, receipts, audit and outbox. Cache-backed collection ports are active for their supported subset; exact ChildRoot capacity and narrow proof ports remain separate dormant boundaries. [src/lib.rs:MIGRATOR]

## Invariants

- Transactions enter through cancellation-safe BeginSafely; journal/projection/version checks and related state/outbox writes commit together. [src/pool.rs:BeginSafely::begin_safely] [src/execution.rs:PgRunTx::persist] [src/execution.rs:PgRunTx::commit]
- CurrentChildRoot capacity stores, leaf certification and scoped leaf finish remain narrow caller-dormant boundaries. Their tables, indexes, ports, metrics and local shape checks do not activate scheduling or prove general nesting/large-workload acceptance. [src/capacity_acquire.rs:PgCapacityAcquireStore] [src/leaf_certification.rs:PgLeafCollectionSuccessStore] [src/scoped_leaf_finish.rs:PgScopedLeafFinishStore] [migrations/20261205000000_bounded_execution_discovery.up.sql:steps_path_actionable_idx]

## Side Effects

- The build script requests rebuild when migrations change; migrate executes embedded SQL. Storage operations query/mutate authority, journal, projections, receipts, audits and outbox; commit emits process-local signals. [build.rs:main] [src/lib.rs:migrate] [src/execution.rs:PgRunTx::persist] [src/execution.rs:PgRunTx::commit] [src/signals.rs:Signal::emit]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-postgres --locked
```

These commands were not run in this documentation-only pass.
