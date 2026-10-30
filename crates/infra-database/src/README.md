# crates/infra-database/src

## Overview

Isolated PostgreSQL target execution with a restricted SQL subset, pinned destination/TLS, bounded wire/results and typed failures.

## Interface

- `ExecutionError::failure` maps to domain failure codes; redacted `DatabaseResult` exposes retained bytes. `PostgresExecutor` configures capacity and executes one deadline/cancellation-bounded attempt. [error.rs:ExecutionError] [error.rs:ExecutionError::failure] [executor.rs:DatabaseResult] [executor.rs:PostgresExecutor]
- `DestinationScope` and `NetworkPolicy` configure/check operator/namespace networks and ports; `validate_statement` enforces SQL AST restrictions. [network.rs:DestinationScope] [network.rs:NetworkPolicy] [statement.rs:validate_statement]

## Invariants

- Capacity is 1–256 and released only after the socket pump stops. Limits intersect connection policy; cancellation/deadline precede admission. Destination sets are bounded, at most 16 DNS answers are accepted, every answer must satisfy policy, and one address is pinned. [executor.rs:PostgresExecutor::new] [executor.rs:PostgresExecutor::execute] [network.rs:DestinationScope::new] [network.rs:NetworkPolicy::resolve]
- SQL is at most 64 KiB and one statement: Query for reads or Insert/Update/Delete for execute, excluding SELECT INTO/locks/settings/other refused clauses. Parameters are typed binds. Production requires verified TLS on the pinned socket without downgrade or another DNS lookup. [statement.rs:validate_statement] [statement.rs:Guard] [values.rs:bindings] [transport.rs:socket]
- Frames/wire bytes/rows/results are bounded and privileged roles/server-file/program memberships are rejected. Errors after Execute dispatch become OutcomeUnknown rather than asserting rollback. [transport.rs:bounded] [frame.rs:BoundedStream::poll_read] [executor.rs:execute_transaction] [executor.rs:PostgresExecutor::execute]

## Side Effects

- Execution resolves DNS, opens fresh TCP/TLS connections, authenticates, checks roles and starts/commits transactions; cancellation can open a separate request to the same pinned address. Query is read-only; Execute may mutate the target and uncertain results require reconciliation. [network.rs:NetworkPolicy::resolve] [transport.rs:socket] [executor.rs:PostgresExecutor::execute] [executor.rs:execute_transaction]
- Driver notices are discarded without formatting, sensitive buffers zeroize, and metrics/spans report execution/connection outcomes. [executor.rs:PostgresExecutor::execute] [executor.rs:DatabaseResult]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-database --locked
```

These commands were not run in this documentation-only pass.
