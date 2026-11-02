# payment-saga

## Overview

Payment compensation and reconciliation example using the TypeScript SDK.

## Interface

- [typescript](typescript/README.md) — Preparation and launch commands for the TypeScript payment-saga example.

## Invariants

- Commands require the four platform connection settings and use bounded client waits. [examples/payment-saga/typescript/src/main.ts:setting] [examples/payment-saga/typescript/src/main.ts:run]
- The simulated provider deduplicates reservation/charge keys; compensation and reconciliation operate on the target effect key. [examples/payment-saga/typescript/src/main.ts:Provider] [examples/payment-saga/typescript/src/main.ts:run]

## Side Effects

- Preparation installs/builds the local SDK and example; execution starts a worker and a durable workflow run, then prints the outcome. [examples/payment-saga/typescript/prepare.sh:npm_install] [examples/payment-saga/typescript/src/main.ts:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored saga::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
