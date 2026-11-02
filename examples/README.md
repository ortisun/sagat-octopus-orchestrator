# examples

## Overview

Runnable SDK examples for greeting workflows, order routing and fulfillment, and a payment saga.

## Interface

- [hello-world](hello-world/README.md) — The same two-activity greeting workflow served and started by five SDK language examples.
- [order-processing](order-processing/README.md) — Five SDK implementations of the shared order routing, fulfillment, and reminder examples.
- [payment-saga](payment-saga/README.md) — Payment compensation and reconciliation example using the TypeScript SDK.

## Invariants

- Commands receive platform addresses, namespace, and credentials through required environment settings. [examples/hello-world/typescript/src/main.ts:setting]
- Order scenarios simulate provider effects; the payment saga uses an in-memory provider with explicit keyed reservation/charge behavior. [examples/order-processing/typescript/src/main.ts:run] [examples/payment-saga/typescript/src/main.ts:Provider]

## Side Effects

- Preparation creates dependency/build output in the language projects. [examples/hello-world/typescript/prepare.sh:npm_install] [examples/hello-world/python/prepare.sh:pip install]
- Example execution registers workers, starts durable workflow runs against the configured platform, waits for their outcomes, and prints diagnostics. [examples/hello-world/typescript/src/main.ts:run] [examples/order-processing/typescript/src/main.ts:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
