# crates/infra-database

## Overview

sagat-infra-database executes bounded PostgreSQL operations for the isolated database runner.

## Interface

- PostgresExecutor, NetworkPolicy and validate_statement expose execution, pinned destinations and the accepted SQL subset. [src/executor.rs:PostgresExecutor] [src/network.rs:NetworkPolicy] [src/statement.rs:validate_statement]
- [src](./src/README.md) — Isolated PostgreSQL target execution with a restricted SQL subset, pinned destination/TLS, bounded wire/results and typed failures. [src/error.rs:ExecutionError]

## Invariants

- SQL/DNS/capacity/frame/wire/row/result limits are explicit and production needs verified TLS. Failure after Execute dispatch is outcome unknown rather than proof of rollback. [src/statement.rs:validate_statement] [src/network.rs:NetworkPolicy::resolve] [src/transport.rs:bounded] [src/executor.rs:execute_transaction]

## Side Effects

- Execution resolves DNS, opens TCP/TLS and may commit mutations to the customer database. [src/network.rs:NetworkPolicy::resolve] [src/executor.rs:PostgresExecutor::execute]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-database --locked
```

These commands were not run in this documentation-only pass.
