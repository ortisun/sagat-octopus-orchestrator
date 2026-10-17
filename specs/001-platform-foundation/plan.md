# Implementation Plan: Platform foundation and contracts

**Branch:** `001-platform-foundation` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Provide a foundation of tenants, namespaces, HTTP/gRPC contracts and a reproducible environment without coupling the domain to infrastructure.

**Technical approach:** Rust workspace with domain/application/infra-postgres/api-http/api-grpc, Axum+tonic+SQLx. Protobuf and JSON schema are canonical contracts. RLS as an optional additional defense, never replacing the tenancy filter in the application layer.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** p95 <250ms on namespace read in the baseline benchmark; authentication fails closed.
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
crates/domain/src/identity.rs
crates/application/src/identity.rs
crates/infra-postgres/src/identity.rs
apps/control-plane/src/main.rs
crates/api-http/src/routes/identity.rs
crates/api-grpc/src/server.rs
infra/docker/compose.yaml
proto/sagat_octopus_orchestrator/v1/worker.proto
tests/contract/
tests/integration/
tests/fault/
tests/e2e/platform-foundation/
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

- **Functional risk:** JWT expired/revoked or JWKS temporarily unavailable. Mitigate via adversarial scenario with fixture.
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

Contract/API + tenant isolation suite and reproducible local environment with migrations executed twice.

## Integrated completion

T001-036 adds server-generated per-request correlation in the outer HTTP middleware,
OpenAPI and all live contract checks. The additive optional schema field allows older
servers to remain representable; the current server always emits it on errors.
T001-037 reuses tenant-scoped command deduplication for optional identity creation keys,
committing the resource, audit and replay record atomically.

## HTTP listener shutdown completion

The listener owns connection tasks, including native TLS handshakes, until they finish.
After shutdown grace expires it cancels and drains remaining tasks before returning;
a detached handler must not retain a socket, body or admission permit. The fault suite
uses a handler that never completes and verifies the client socket actually closes.

## Product version 0.1.0

T001-041 aligns release metadata and generated lockfiles, records the owner-requested numbering
reset in release notes, preserves wire protocol 1.4, replaces the old v0.4.0 tag only after validation, and validates package
metadata, generated API types and Helm rendering before main/develop integration.


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. The canonical workflow schema gains additive bounded declarations and node kinds; worker 1.5 scoped
identity, primitive HTTP operations and new error reasons preserve accepted root forms/runs,
existing worker minors, Error.code and run-status enums. Publication checks enabled capabilities,
not merely schema acceptance.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.
