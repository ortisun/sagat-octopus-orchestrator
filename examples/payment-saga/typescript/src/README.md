# src

## Overview

TypeScript payment saga demonstrating keyed effects, compensation, and unknown-outcome reconciliation.

## Interface

- The local `run` function is launched by top-level executable code. [examples/payment-saga/typescript/src/main.ts:run]
- The command-local `Provider` simulates reservation and payment effects in memory. [examples/payment-saga/typescript/src/main.ts:Provider]

## Invariants

- All four connection settings are mandatory; a missing setting exits 2. [examples/payment-saga/typescript/src/main.ts:setting]
- Reservation and charge keys reuse existing effects. `decline` raises permanent `CARD_DECLINED`; `lost-answer` records the charge before raising outcome-unknown. [examples/payment-saga/typescript/src/main.ts:Provider]
- Expected terminal states are `SUCCEEDED` for `ok` and `lost-answer`, and `COMPENSATED` for `decline`. [examples/payment-saga/typescript/src/main.ts:EXPECTED]
- Registers three activities, two compensations using `targetIdempotencyKey`, and a reconciler returning applied/not-applied verdicts. [examples/payment-saga/typescript/src/main.ts:run]

## Side Effects

- Starts and stops a worker, starts a workflow, waits at most 60 seconds, handles SIGINT through cancellation, and prints run/error diagnostics. [examples/payment-saga/typescript/src/main.ts:run]
- Mutates in-memory reservation/charge ledgers and prints effect and reconciliation diagnostics. [examples/payment-saga/typescript/src/main.ts:Provider]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored saga::
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
