# crates/infra-postgres/src/scripts

## Overview

Parent-private PostgreSQL helpers for category-only script-violation audit evidence on fenced production and test attempts.

## Interface

- Parent-private `production_was_recorded` and `test_was_recorded` check prior incident evidence. [incident.rs:production_was_recorded] [incident.rs:test_was_recorded]
- Parent-private `record` accepts the store, principal, stored incident target/category, trace ID and timestamp. The `incident` module is private. [incident.rs:record] [../scripts.rs:mod incident]

## Invariants

- Prior evidence is restricted to seven incident categories and `script.violation`. [incident.rs:CATEGORIES] [incident.rs:production_was_recorded]
- Production recording checks tenant/namespace/run/step identity, attempt/generation/token, active unexpired database-clock lease, READY artifact, current session/subject and matching activity binding before insertion. [incident.rs:record]
- Production lock order is member chain, run row, binding advisory lock, then step. Test recording requires a matching RUNNING row/token, attempt at least one and an unexpired database-clock lease. [incident.rs:record]

## Side Effects

- The evidence helpers query PostgreSQL. [incident.rs:production_was_recorded] [incident.rs:test_was_recorded]
- `record` opens a safe transaction, acquires row/advisory locks, reads authority rows, appends an audit record and commits on success; false/error paths do not commit an audit. [incident.rs:record]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-infra-postgres --locked
```

These commands were not run in this documentation-only pass.
