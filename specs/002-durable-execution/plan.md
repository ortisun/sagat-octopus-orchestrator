# Implementation Plan: Durable execution engine and scheduler

**Branch:** `002-durable-execution` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Ensure DAG progress, checkpoints, suspension, retries and failure recovery without re-executing commits.

**Technical approach:** Planner reduces events into a pure state machine; scheduler uses `FOR UPDATE SKIP LOCKED` and fencing in transactions; outbox dispatcher sends hints to JetStream; worker protocol confirms via Command API.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Benchmark of at least 10k suspended runs and scheduling latency; 100k/1M as later experiments.
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
crates/domain/src/workflow.rs
crates/domain/src/execution.rs
crates/application/src/commands.rs
crates/scheduler/src/claim.rs
crates/scheduler/src/reconcile.rs
crates/infra-postgres/src/journal.rs
crates/infra-postgres/src/outbox.rs
apps/execution-engine/src/main.rs
tests/contract/
tests/integration/
tests/fault/
tests/e2e/durable-execution/
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

- **Functional risk:** crash after external effect and before the checkpoint. Mitigate via adversarial scenario with fixture.
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

Deterministic crash test at each point of claim→effect→commit→publish, with an intact journal and no double transition.

## Integrated blob completion

The claim-check extension (ADR-0023) adds a provider-neutral application port and
PostgreSQL catalog, then bounded mediated worker transfers and run-input admission.
Descriptor visibility commits with the durable transition; external bytes are staged
first and garbage collection retains retry evidence until external deletion succeeds.
Current worker bindings, tenant/namespace ownership and lease fencing remain authoritative.
Feature 006 supplies common SDK conformance, 009 authorization/quotas, and 010 recovery;
these integrated implementations are approved dependencies for T002-037..039.

## Serialized poll budget hardening

Render and measure a task context before leasing it. Admit at most 8 MiB of serialized inputs per poll; update the response count only after each run transaction commits. A context larger than a fresh response records its preflight attempt and non-retryable `RUN_DATA_LIMIT` together through the existing saga-aware failure transition, with no published token. Prove numeric normalization expansion, sibling tasks split across polls, and cross-run batching against PostgreSQL (`T002-039`).

## Run-input blob slice

Extend the existing catalog with tagged ownership and nullable pre-start run identity; reuse streaming storage, limits, audit and tombstone cleanup. Add authorized raw REST upload, stable pre-catalog start replay, transactional run-input binding, bounded descriptor schema validation and durable lease input reference. Configure the same storage provider in control plane and engine. Verify the REST-to-worker path with PostgreSQL and the in-process storage test backend, then exercise production configuration and compatibility. ADR-0023 specifies the input semantics; The server path is covered by real REST/PostgreSQL/gRPC tests in `grpc/input_blobs.rs`; T002-040 is verified by all-five SDK conformance over shared filesystem storage, with terminal-state, descriptor/digest and early-response integrity assertions.
