# Implementation Plan: Distributed observability and operations

**Branch:** `008-observability-operations` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Diagnose and operate runs, steps, external integration, resources and Sagas from reliable data.

**Technical approach:** Standard OTel Collector, journal projection workers, paginated search APIs and actions that generate auditable commands. Delivered as ADR-0017 says: the write path leaves one note per run, a projector derives spans and measurements from the journal and exports them over OTLP/HTTP, a monitor finds the conditions that need a person and keeps alerts in step, and the operator actions are commands the domain checks and the audit trail records.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Projection must not block the write-path; documented and flagged lag budget.
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
crates/telemetry/                      OTLP exporter, span ids and sampling, redaction, JSON logs, metric buckets
crates/domain/src/operations.rs        alert kinds, states, severities; run cause; operator commands in execution/
crates/application/src/telemetry.rs    derive (journal -> spans, measurements), TelemetryStore, per-run state
crates/application/src/operations.rs   monitor/alert/worker/audit ports and OperationsService
crates/scheduler/src/telemetry.rs      the projector;  monitor.rs the monitor
crates/infra-postgres/                 migrations 20261015*, telemetry.rs, operations.rs, workers.rs (drain)
crates/api-http/src/routes/            operations.rs (alerts, workers, drain, audit), execution.rs (telemetry, attempts, actions)
apps/execution-engine/                 the loops and settings of the projector and the monitor
frontend/apps/studio/src/features/runs the timeline with cause, span and telemetry state; the new actions
infra/observability/                   Collector, Prometheus rules (17), Grafana dashboard (34 panels), provisioning
infra/docker/compose.observability.yaml  Collector + Jaeger + Prometheus + Grafana, images pinned
docs/runbooks/{observability,alerts}.md  how to operate it; docs/adr/ADR-0017
tests/{e2e,fault,integration,contract,performance}/observability-operations/
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

- **Functional risk:** trace missing but journal present. Mitigate via adversarial scenario with fixture.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |

## Delivered, and what is not

Phases 1 to 6 were run for US01 to US04; the evidence per story, the review of the implementation and the open points are in [`quickstart.md`](./quickstart.md); decisions D160-D178 and gaps G169-G186 are in [`research.md`](./research.md). The performance goal ("projection must not block the write path; documented and flagged lag budget") is answered by the benchmark in `tests/performance/reports/observability-operations.md`: the note costs a writer about 4% of a completion, a busy projector slows the median completion by 1.04x, and the lag under sustained writing stayed at the resolution of the sample (100 ms) against a 30 s budget. The figures are for one machine and a PostgreSQL 18 with commits that do not ask the drive to flush; they are hypotheses, not promises.

## Delivery Gate

Operator diagnoses a failure in fewer than N interactions during a UX test, with run journal and trace correlated.

## Studio operations completion — T008-036

Reuse feature 007 shell/UI and existing feature 008 namespace alerts/workers/drain/undrain
contracts; no new backend permissions or execution semantics. Generated wire types and the
same-origin API client preserve tenant admission, canonical errors, HTTP traces and server-side
transactional audit. Load alerts by cursor; show both current worker sessions and stored drains
so an identity remains resumable after its session expires. Unmount namespace-scoped content
when context changes, preserve request scope, suppress late mutation callbacks. Do not infer
liveness from registration alone. Verify permission denial, paging, failures, drain/resume
confirmation and cross-context responses with component tests and a real browser/API journey.


## Approved workflow-primitives dependency (2026-10-14)

[ADR-0030](../../docs/adr/ADR-0030-structured-instances-and-continuation-chains.md) and
[ADR-0031](../../docs/adr/ADR-0031-declared-operation-result-disclosure.md) accept the feature-014
architecture and separate declared-result disclosure policy.
[ADR-0032](../../docs/adr/ADR-0032-additive-workflow-ir.md) keeps the workflow IR at v1 and
supersedes the earlier separate-revision allocation. Journal-derived projections may include scoped instance metadata, routes and chain links, with lag and bounded primitive metrics. Payload/state/argument/result values remain absent from logs, audit, spans, graph and SSE; runtime truth never comes from ephemeral telemetry.

This approves the described dependency for implementation, not a runtime guarantee. Canonical
contracts, migration/replay, tenant/fault tests and applicable five-SDK evidence remain the
feature-014 release gates; existing feature behavior stays supported.
