# src

## Overview

TypeScript executable serving common order activities and starting a selected workflow.

## Interface

- The local `run` function is launched by top-level executable code. [examples/order-processing/typescript/src/main.ts:run]

## Invariants

- All four connection settings are mandatory; a missing setting exits 2. [examples/order-processing/typescript/src/main.ts:setting]
- `SAGAT_SCENARIO` defaults to `routing` and accepts only routing, fulfillment, or reminder. [examples/order-processing/typescript/src/main.ts:scenarioSetting]
- Registers the four `order.*` activities; notification rejection is permanent `NOTIFICATION_REJECTED`. Success requires `SUCCEEDED`. [examples/order-processing/typescript/src/main.ts:run]

## Side Effects

- Starts and stops a worker, starts a workflow, waits at most 60 seconds, handles SIGINT through cancellation, and prints run/error diagnostics. [examples/order-processing/typescript/src/main.ts:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored order_processing::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
