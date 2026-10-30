# crates

## Overview

Rust workspace packages for domain/application logic, API and infrastructure adapters, scheduling and telemetry.

## Interface

- Core packages are sagat-domain and sagat-application; transports are sagat-api-http and sagat-api-grpc. [domain/Cargo.toml:package.name] [application/Cargo.toml:package.name] [api-http/Cargo.toml:package.name] [api-grpc/Cargo.toml:package.name]
- Infrastructure packages cover PostgreSQL authority, isolated database execution, blobs, messaging, guarded egress, expressions, IR, sandboxing, schemas and secrets. Scheduler and telemetry provide background orchestration and observability. [infra-postgres/Cargo.toml:package.name] [infra-database/Cargo.toml:package.name] [infra-blobstore/Cargo.toml:package.name] [infra-connectors/Cargo.toml:package.name] [infra-nats/Cargo.toml:package.name] [scheduler/Cargo.toml:package.name] [telemetry/Cargo.toml:package.name]
- [api-grpc](./api-grpc/README.md) — sagat-api-grpc adapts the shared worker protocol through Tonic and compiles its Protobuf contract at build time. [api-grpc/src/lib.rs:pb]
- [api-http](./api-http/README.md) — sagat-api-http provides Axum HTTP, authentication, error envelopes, native serving and optional Studio hosting. [api-http/src/auth/config.rs:AuthConfig]
- [application](./application/README.md) — sagat-application defines use cases and storage/provider ports over domain types for adapters to implement. [application/src/runs.rs:RunService]
- [domain](./domain/README.md) — sagat-domain contains framework-free business types, validation and reducers without web/database/broker adapters. [domain/src/lib.rs:pub mod]
- [infra-blobstore](./infra-blobstore/README.md) — sagat-infra-blobstore implements memory, filesystem, S3-compatible and R2 blob storage. [infra-blobstore/src/backend.rs:ObjectStoreBackend]
- [infra-connectors](./infra-connectors/README.md) — sagat-infra-connectors adapts inbound/outbound brokers, Redis and guarded webhooks. [infra-connectors/src/supervisor.rs:Supervisor]
- [infra-database](./infra-database/README.md) — sagat-infra-database executes bounded PostgreSQL operations for the isolated database runner. [infra-database/src/executor.rs:PostgresExecutor]
- [infra-egress](./infra-egress/README.md) — sagat-infra-egress provides policy-enforced outbound HTTP with guarded DNS and redirects. [infra-egress/src/lib.rs:Guard]
- [infra-expr](./infra-expr/README.md) — sagat-infra-expr is a pure bounded evaluator for a CEL-like subset and scalar templates. [infra-expr/src/eval.rs:Env]
- [infra-ir](./infra-ir/README.md) — sagat-infra-ir converts JSON workflow IR into validated domain models, canonical forms and digests. [infra-ir/src/lib.rs:JsonIrCodec]
- [infra-nats](./infra-nats/README.md) — sagat-infra-nats publishes outbox task hints and run events through JetStream. [infra-nats/src/lib.rs:NatsConfig]
- [infra-postgres](./infra-postgres/README.md) — sagat-infra-postgres implements PostgreSQL authority storage ports and embeds schema migrations. [infra-postgres/src/execution.rs:PgRunStore]
- [infra-sandbox](./infra-sandbox/README.md) — sagat-infra-sandbox builds and runs untrusted scripts in isolated Docker containers and embeds language shims. [infra-sandbox/src/docker.rs:DockerSandbox]
- [infra-schema](./infra-schema/README.md) — sagat-infra-schema provides bounded JSON Schema validation/inference, bundle checks and classification projection. [infra-schema/src/lib.rs:Schemas]
- [infra-secrets](./infra-secrets/README.md) — sagat-infra-secrets implements AES-256-GCM sealing, database-credential scanning and an OpenBao KV-v2 adapter. [infra-secrets/src/lib.rs:KeyRing]
- [scheduler](./scheduler/README.md) — sagat-scheduler runs background passes over authoritative application storage ports. [scheduler/src/passes.rs:Scheduler::tick]
- [telemetry](./telemetry/README.md) — sagat-telemetry provides structured logging, Prometheus metrics, trace identity and OTLP export. [telemetry/src/logging.rs:Telemetry]

## Invariants

- The domain has no internal Sagat dependency; application depends inward on domain, while adapters declare application/domain dependencies as needed. Package manifests inherit workspace lints. [domain/Cargo.toml:dependencies] [application/Cargo.toml:sagat-domain] [infra-postgres/Cargo.toml:sagat-application] [domain/Cargo.toml:lints]
- Capacity-transfer, leaf-certification and scoped-leaf-finish contracts describe narrow caller-dormant machinery. Active collection/loop scheduling and source-level bounds do not establish acceptance of general nesting or large workloads. [domain/src/execution/capacity_transfer.rs:acquire_gate] [application/src/leaf_certification.rs:LeafCollectionSuccessStore] [application/src/scoped_leaf_finish.rs:ScopedLeafFinishStore] [scheduler/src/passes.rs:Scheduler::tick]

## Side Effects

- This directory groups workspace members; runtime effects belong to child packages. PostgreSQL adapters persist authority, transport/provider adapters perform external I/O, and scheduler passes coordinate injected ports. [../Cargo.toml:members] [infra-postgres/src/execution.rs:PgRunTx::commit] [scheduler/src/passes.rs:Scheduler::tick]

## Verification

From the repository root, check available disk with `df -h` and use a private
`CARGO_TARGET_DIR` outside the checkout. Run:

```sh
RUST_TEST_THREADS=2 cargo test --workspace --locked --no-fail-fast
```

These commands were not run in this documentation-only pass.
