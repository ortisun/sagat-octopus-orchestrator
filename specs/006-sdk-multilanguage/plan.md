# Implementation Plan: TypeScript, Go, Python, C# and Java SDKs

**Branch:** `006-sdk-multilanguage` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Allow definition, triggers, activities and Sagas in five languages with common execution semantics.

**Technical approach:** Protobuf codegen + idiomatic adapters and Activity worker; do not make the SDK persist its own journal. Distinguish client SDK and worker SDK from the DSL compiler.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Worker protocol must tolerate high concurrency without busy polling; measure streaming and backpressure.
- **Constraints:** single-region HA; state loss after ACK forbidden; side effects at-least-once; security-by-default.
- **Scale/Scope:** scale horizontally without keeping live processes for waits.

## Constitution Check

- [x] State and retry semantics documented (`I`).
- [x] Modularity/ports and adapters planned (`II`).
- [x] Saga and reversibility applied where relevant (`III`).
- [x] Compatibility and version pinning (`IV`).
- [x] Code/tenant boundary and secrets (`V`).
- [x] OTel and auditable journal (`VI`).
- [x] Redeliverable events and inbox/outbox (`VII`).
- [x] Fault injection/contract plan (`VIII`).

These checks indicate conformance **of the design**, not approval of the implementation.

## Project Structure

```text
proto/sagat_octopus_orchestrator/v1/worker.proto
sdks/typescript/src/index.ts
sdks/go/client/client.go
sdks/python/sagat_octopus_orchestrator/client.py
sdks/dotnet/src/SagatOctopusOrchestrator.Client/WorkflowClient.cs
sdks/java/client/src/main/java/dev/sagatoctopusorchestrator/WorkflowClient.java
tests/contract/sdk-conformance/
tests/contract/
tests/integration/
tests/fault/
tests/e2e/006-sdk-multilanguage/
```

## Implementation phases

1. Domain contracts and types, invariant validation + tests.
2. Persistence and port adapters + migration and integration tests.
3. APIs/command processing + versioning + authorization and observability.
4. Happy path flow with e2e tests.
5. Fault injection, redelivery, timeouts, concurrency and regression.
6. Security/performance review, documentation and release gate.

## Data Model / Contracts

- [`data-model.md`](./data-model.md)
- Canonical contracts in `specs/001-platform-foundation/contracts/` with reviewed migrations.
- Applicable ADRs in `docs/adr/`.

## Risks & Mitigations

- **Functional risk:** SDK worker dies without heartbeat. Mitigate via adversarial scenario with fixture.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |

## Delivery Gate

Same golden flows and failure cases executed in TS, Go, Python, C# and Java with normalized results.
