# Implementation Plan: Performance, scalability and high availability

**Branch:** `010-scalability-ha` | **Date:** 2026-10-07 | **Spec:** [`spec.md`](./spec.md)

## Summary

Scale scheduler, workers and millions of waits without state loss, and quantify the real limits of the architecture.

**Technical approach:** Partitioned/indexed PostgreSQL; scheduler owner leases; worker pool autoscaling; telemetry-guided tuning. Evaluate another journal backend only via ADR/benchmarks.

## Technical Context

- **Language/Version:** Rust stable pinned via `rust-toolchain.toml`; TypeScript frontend; SDKs per context.
- **Primary Dependencies:** Tokio, Axum, tonic/Protobuf, SQLx, serde/JSON Schema, NATS client, OTel; per feature.
- **Storage:** PostgreSQL with transactions and outbox; NATS JetStream; S3-compatible for large artifacts/payloads.
- **Testing:** cargo test, sqlx migrations, Testcontainers, consumer fixtures, contract/e2e/fault tests.
- **Target Platform:** Linux containers/Kubernetes, dev Docker Compose, HTTP/gRPC protocols.
- **Project Type:** modular Rust monorepo + React frontend + multi-language SDKs.
- **Performance Goals:** Quantifiable and reproducible targets; distinguish wait capacity, active throughput and latency under backlog.
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
crates/domain/src/sharding.rs                 the shard, the set of shards, the share, the plan
crates/application/src/shards.rs              the port of ownership; Window, RunTx::holds_shard
crates/scheduler/src/shards.rs                the keeper, the placement, the snapshot
crates/scheduler/src/fairness.rs              the selection between tenants
crates/scheduler/src/passes.rs, outbox.rs     the passes and the dispatcher over the shards they hold
crates/infra-postgres/src/partitioning.rs     the leases, the members, the weights
crates/infra-postgres/migrations/20261017000000_scheduler_shards, 20261017000100_active_run_counts
infra/helm/                                   the chart (values.yaml, templates, examples/ha.yaml)
infra/dr/                                     the drill and the digest of the journal
tests/performance/scalability-ha/             the load harness and run.sh (T010-029 asks for tests/performance/scenarios/)
tests/fault/failover/, tests/fault/scalability-ha/
tests/contract/scalability-ha/ (with fixtures/), tests/integration/scalability-ha/, tests/e2e/scalability-ha/
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

- **Functional risk:** shard split-brain. Mitigate via adversarial scenario with fixture.
- **Operational risk:** throughput below target; perf profiling and scaling driven by metrics, never a promise without a benchmark.
- **Compatibility risk:** schema/SDK diverges; schema evolution and golden fixtures.
- **Security risk:** tenant/sandbox boundary; adversarial tests and fail closed.

## Complexity Tracking

| Decision | Cost | Justification |
|---|---|---|
| Rust modular monolith | Learning curve | Centralized invariants and gradual evolution |
| DB transactional outbox | Extra reads/writes | Avoid losing a wakeup after commit |
| Multi-SDK via proto | More CI and tests | Real parity and low coupling |

## Delivered (feature 010)

1. Domain contracts and types: `sharding.rs`, `shards.rs` and their tests.
2. Persistence and ports: two additive migrations, the `ShardStore` port, the candidate queries shard first, the counters of active runs.
3. Command processing: the keeper in the engine, the passes and the outbox dispatcher over the shards they hold, the operator's `scheduling-weight`, metrics, alerts, the chart.
4. Happy path: `e2e_scalability_ha`, the four stories against real engine processes.
5. Faults: `fault_failover` (chaos), `fault_scalability_ha`, the drill (`infra/dr/drill.sh`).
6. Reviews, benchmark, documentation: `quickstart.md`, `tests/performance/reports/scalability-ha.md`, ADR-0020, the two runbooks.

## Delivery Gate

Performance reports with hardware/configuration, zero confirmed Run loss in fault injection and validated DR restore.

## Completion hardening

T010-037 groups named engine loops and server futures in a JoinSet, observes their
termination and drains them for at most 30 seconds after shared shutdown. An atomic
process-failure flag is sticky for both probes. No execution state or fencing semantics
change. T010-038 installs the real chart and tests queue-driven scaling in a disposable
cluster; rendering and canary policy tests alone do not close that gate.
