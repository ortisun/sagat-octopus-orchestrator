# Implementation Plan: Visual Studio, Monaco and runtime graph

**Branch:** `007-workflow-studio` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Offer intuitive authoring and operation of complex processes with a faithful depiction of executions.

**Technical approach:** React Flow for the DAG; Monaco in the panel; versioned server-side state (optimistic concurrency) and runtime graph projected by event cursor; virtualization of large DAGs.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** 1k-node DAG usable in the reference browser; virtualized lists for 10k+; FPS targets per UX benchmark.
- **Constraints:** single-region HA; state loss after ACK forbidden; side effects at-least-once; security-by-default.
- **Visual direction:** [`design/README.md`](./design/README.md) (FR-007-009, D154–D159): tokens in `frontend/apps/studio/src/styles/base.css`, English labels, the accessibility floor above the prototype, no element without a data source.
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
frontend/apps/studio/src/features/workflows/WorkflowCanvas.tsx
frontend/apps/studio/src/features/workflows/NodeInspector.tsx
frontend/apps/studio/src/features/runs/ExecutionGraph.tsx
frontend/apps/studio/src/features/runs/Timeline.tsx
crates/api-http/src/routes/runs.rs
tests/contract/
tests/integration/
tests/fault/
tests/e2e/workflow-studio/
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

- **Functional risk:** graph with 50k nodes. Mitigate via adversarial scenario with fixture.
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

E2E create→validate→publish→trigger→fail→compensate→inspect journal/traces.

## Workflow schema editor completion (2026-10-12)

Reuse the existing script editor's lazy-loaded Monaco JSON service and bundled meta-schema for
workflow input/output schemas, with separate model addresses. The document contract remains an
object or absent; incomplete JSON remains local and the last valid schema stays in the draft.
Plain-text mode preserves that local text and exposes parse errors through accessible descriptions.
No remote schema requests or changes to workflow validation semantics are introduced.
