# crates/infra-postgres/src/execution

## Overview

PostgreSQL run-storage internals for private definitions, managed-database attempt credentials, exact-path snapshots/mutations and scoped-event receipts.

## Interface

- Parent-private ad-hoc helpers `retain`, `private_definition` and `latest_probe` retain immutable definitions and read actor-scoped definition/probe metadata. [ad_hoc.rs:retain] [ad_hoc.rs:private_definition] [ad_hoc.rs:latest_probe]
- Parent-private `PgRunTx` database methods cover role/owner/admission/report checks, connection snapshots, secret pin/readback and output limits. [database.rs:PgRunTx::database_role] [database.rs:PgRunTx::database_admission] [database.rs:PgRunTx::database_snapshot] [database.rs:PgRunTx::database_pin] [database.rs:PgRunTx::database_output_limits]
- `PgPathExecutionTx` implements `PathExecutionTx` snapshots/time, shard and worker authorization, secrets, admission, lease/report/idempotency checks, addressed persistence, scoped-event delivery and commit. [path.rs:impl PathExecutionTx for PgPathExecutionTx]
- Exact-path persistence helpers validate/write events, outputs and blobs; charge quotas; pin script retention; update breaker/projection state; emit ready outbox rows; and match lease tokens. [path.rs:PgPathExecutionTx::persist_path_addressed] [path.rs:PgPathExecutionTx::upsert_path_instance] [path.rs:PgPathExecutionTx::outbox_path_task_ready] [path.rs:PgPathExecutionTx::lease_token_matches]
- Parent-private `PgRunStore::read_path_snapshot` reconstructs an exact path, ancestor fences, local collection/loop/saga state, data and outside-work summary. [path_read.rs:PgRunStore::read_path_snapshot]
- Scoped-event receipt/persistence/admission/clock/row-validation helpers are parent-private; `validate_scoped_event_identity` is re-exported crate-wide. [scoped_events.rs:ScopedEventDelivery] [scoped_events.rs:prepare_delivery] [scoped_events.rs:require_delivery] [scoped_events.rs:require_scoped_event_clock] [scoped_events.rs:EventWaitReadState] [../execution.rs:validate_scoped_event_identity]

## Invariants

- A bounded write cannot cross its exact path, contain root-only effects, use unsupported event kinds or diverge between state version and event count. [path.rs:PgPathExecutionTx::persist_path_addressed]
- Each output needs a unique completion event; blob commit requires one matching forward completion, descriptor, tenant/namespace/run/step identity, attempt and generation. [path.rs:PgPathExecutionTx::persist_path_addressed] [path.rs:blob_output_matches]
- Commit rejects failed scoped-event persistence, an unpersisted receipt or a state version different from the persisted journal version. Run output retention is checked against `MAX_RUN_OUTPUT_BYTES` and reconciled with the usage ledger under the run lock. [path.rs:PathExecutionTx::commit] [path.rs:PgPathExecutionTx::charge_path_output_bytes]
- A scoped wait value needs one exact non-root satisfaction and its current locked receipt; output digest/effects must match. Clock decisions need acknowledged scheduler authority, exact stored pre-transition state and at most one clocked action per instance per write. [scoped_events.rs:require_delivery] [scoped_events.rs:require_scoped_event_clock]
- Path restoration rejects contradictory lifecycle, drain/count/owner/definition, physical event identity, saga dependency and ancestor-scope projections. [path_read.rs:PgRunStore::read_path_snapshot]

## Side Effects

- Ad-hoc retention locks workflow quota and inserts `workflows`, `workflow_versions` and `private_execution_definitions`; lookup helpers query PostgreSQL. [ad_hoc.rs:retain] [ad_hoc.rs:private_definition] [ad_hoc.rs:latest_probe]
- Database attempt methods lock/query memberships, sessions, catalog, policy, secrets and credentials, and insert `step_database_credentials`; customer-database/provider I/O is outside these methods. [database.rs:PgRunTx::database_admission] [database.rs:PgRunTx::database_snapshot] [database.rs:PgRunTx::database_pin]
- Exact-path persistence writes payloads/blobs, journal, telemetry backlog, step/run projections, audit/idempotency rows and `task.ready` outbox rows in its transaction, while changing snapshot data in memory. [path.rs:PgPathExecutionTx::persist_path_addressed] [path.rs:PgPathExecutionTx::outbox_path_task_ready]
- Successful commit commits PostgreSQL before emitting accumulated in-process signals. Scoped-event admission locks inbox/step rows and inserts an idempotent `event_deliveries` receipt. [path.rs:PathExecutionTx::commit] [scoped_events.rs:prepare_delivery]
- Path reads query PostgreSQL and reconstruct in-memory snapshots without stored-state mutation. [path_read.rs:PgRunStore::read_path_snapshot]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-postgres --locked
```

These commands were not run in this documentation-only pass.
