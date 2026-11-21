# execution-engine

## Overview

Worker gRPC serving and durable scheduling/event/delivery/projection loops assembled over PostgreSQL with optional hint transports and connectors.

## Interface

- [Source contract](./src/README.md) — public library API, command surface and process composition.
- Package `sagat-execution-engine` is declared by the local manifest. [Cargo.toml:name]

## Invariants

- Source-specific validation and lifecycle checks are defined in the [source contract](./src/README.md). This package boundary adds no separate runtime enforcement.

## Side Effects

- Process effects are described in the [source contract](./src/README.md); loading this manifest does not start the process.

## Verification

From the repository root, using the resource profile in [AGENTS.md](../../AGENTS.md):

```sh
cargo test -p sagat-execution-engine --locked
```

This verification command was not executed in the documentation pass.
