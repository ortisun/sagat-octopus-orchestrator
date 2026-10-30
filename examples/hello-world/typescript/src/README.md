# src

## Overview

TypeScript executable serving and starting the canonical greeting workflow.

## Interface

- The local `run` function is launched by top-level executable code. [examples/hello-world/typescript/src/main.ts:run]

## Invariants

- All four connection settings are mandatory; a missing setting exits 2. [examples/hello-world/typescript/src/main.ts:setting]
- Registers `greeting.compose` and `greeting.deliver`; delivery uses the prior `compose` output. The name defaults to `Ada`, and success requires `SUCCEEDED`. [examples/hello-world/typescript/src/main.ts:run]

## Side Effects

- Starts and stops a worker, starts a workflow, waits at most 60 seconds, handles SIGINT through cancellation, and prints run/error diagnostics. [examples/hello-world/typescript/src/main.ts:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
