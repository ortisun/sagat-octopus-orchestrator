# crates/domain/src/database

## Overview

Closed, payload-free managed-database failure taxonomy exported by the database domain module.

## Interface

- `DatabaseFailure` covers configuration/policy, connection/TLS/authentication, capacity/execution/results, capabilities/credentials, unknown outcomes and worker failures. `code` and `parse` provide stable uppercase wire codes and reject unknown values. [failure.rs:DatabaseFailure] [failure.rs:DatabaseFailure::code] [failure.rs:DatabaseFailure::parse]

## Invariants

- Parsing searches the same closed variant set represented by `code`; the enum carries no SQL, credentials, parameters, payloads or driver diagnostics. [failure.rs:DatabaseFailure::parse] [failure.rs:DatabaseFailure]

## Side Effects

- None.

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
cargo test -p sagat-domain --locked
```

These commands were not run in this documentation-only pass.
