# src

## Overview

Java greeting command serving and starting the two-activity hello-world workflow.

## Interface

- [main](main/README.md) — Java greeting command serving and starting the two-activity hello-world workflow.

## Invariants

- `SAGAT_SERVER`, `SAGAT_GRPC`, `SAGAT_NAMESPACE`, and `SAGAT_TOKEN` are mandatory; a missing setting exits 2. [examples/hello-world/java/src/main/java/HelloWorld.java:setting]
- The name defaults to `Ada`; delivery reads the prior `compose` output. [examples/hello-world/java/src/main/java/HelloWorld.java:run]
- Registers `greeting.compose` and `greeting.deliver`; command success requires terminal status `SUCCEEDED`. [examples/hello-world/java/src/main/java/HelloWorld.java:run]

## Side Effects

- Starts and stops the worker, starts a workflow through the HTTP client, waits at most 60 seconds, and prints diagnostics. A shutdown hook gives the worker ten seconds to stop. [examples/hello-world/java/src/main/java/HelloWorld.java:run]

## Verification

From the repository root, with Docker, the selected example toolchains, and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test -p sagat-tests --test e2e_developer_experience --locked -- --ignored
```

The shared real-platform suite is declared in `tests/Cargo.toml`; language preparation and run commands remain in the example READMEs. [tests/Cargo.toml:e2e_developer_experience]
