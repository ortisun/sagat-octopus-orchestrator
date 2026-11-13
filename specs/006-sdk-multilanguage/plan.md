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
tests/e2e/sdk-multilanguage/
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

## Completion integration

The owner selected MIT on 2026-10-11. Package builds must produce local artifacts and exercise
clean consumer installs before any registry release. Credential-free CI stores the same artifacts.
OpenTelemetry bridges remain opt-in and use the application's provider/exporter; instrumentation
failures must not change workflow results or emit payloads, credentials or arbitrary exception text.
Worker protocol 1.3 blob support depends on the transactional catalog and bounded transport of
feature 002; all five languages share its contract and common conformance scenarios.

## Task streaming completion

Implement [`stream-completion.md`](./stream-completion.md) in this order: canonical additive
schema and shared durable admission; bounded authenticated server stream; five independent
SDK adapters/host lifecycles; shared conformance, failure matrix and comparative measurement.
Features 001 and 002 are approved dependencies. Protocol minor 1.4 is independent of package
SemVer. Root integration owns shared fixtures and reviews; SDK writers own only their subtree.


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. Worker minor 1.5 adds scoped task identity while every accepted 1.0–1.4 form remains served. Unary/stream claims share eligibility and fencing. All five SDKs add additive v1 declarative builders and explicit operation/chain clients only against reviewed shared conformance; package versions and worker minor remain independent.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.


## Approved dependent delivery: Rust SDK (2026-10-27)

The owner requested an official Rust SDK during the active 0.3 delivery. Feature
[034](../034-rust-sdk/spec.md) and [ADR-0070](../../docs/adr/ADR-0070-rust-sdk-shares-the-canonical-sdk-contract.md)
extend the same SDK semantic contract, canonical protocol, errors, conformance and package gates to
Rust. Existing five-language evidence stays intact. This authorizes the dependency; the new SDK's
actual acceptance is tracked by T034 tasks and is not supplied by this note.
