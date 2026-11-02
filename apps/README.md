# Application processes

## Overview

Rust executables and reusable process-composition APIs for control, scheduling, isolated execution, development identity and CLI operations.

## Interface

- [cli](./cli/README.md) — Embeddable sagat_cli command dispatcher and the sagat executable: local authoring, platform operations, development workers and onboarding diagnostics.
- [control-plane](./control-plane/README.md) — Control-plane composition, environment configuration, operator commands, PostgreSQL migrations and HTTP/Studio serving.
- [database-runner](./database-runner/README.md) — Isolated managed-PostgreSQL executor with a narrow configuration/runner API and crate-private lease, capability, session and reporting machinery.
- [dev-idp](./dev-idp/README.md) — Development-only ES256 identity issuer with discovery/JWKS/token routes and optional persistent signing keys.
- [execution-engine](./execution-engine/README.md) — Worker gRPC serving and durable scheduling/event/delivery/projection loops assembled over PostgreSQL with optional hint transports and connectors.
- [sandbox-runner](./sandbox-runner/README.md) — Remote script build/test/step runner over HTTP and worker gRPC, delegating compilation and execution to a supplied Sandbox.

## Invariants

- The [control-plane](./control-plane/README.md) and [engine](./execution-engine/README.md) contracts describe startup identity, storage and transport checks; the [sandbox](./sandbox-runner/README.md) and [database](./database-runner/README.md) runners describe isolated execution capabilities.
- Development identity and CLI mock behavior remain explicit in their child contracts. This directory adds no executable enforcement.

## Side Effects

- Child executables open listeners, make network/storage calls and emit telemetry as described in their source contracts.

## Verification

Run from the repository root with `RUST_TEST_THREADS=2` and a private `CARGO_TARGET_DIR` outside the checkout:

```sh
cargo test --workspace --locked --no-fail-fast
```

The integration suites require their declared Docker fixtures. No runtime test result is implied by this index.
