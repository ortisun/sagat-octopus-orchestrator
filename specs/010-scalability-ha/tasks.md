# Tasks: Performance, scalability and high availability

**Input:** `specs/010-scalability-ha/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T010-001 Check FR-010-001..FR-010-007, SCs and ADRs; record gaps in `specs/010-scalability-ha/research.md`.
- [ ] T010-002 [P] Define fixtures and contract validation in `tests/contract/010-scalability-ha/fixtures/`.
- [ ] T010-003 [P] Document types, transitions and invariants in `specs/010-scalability-ha/data-model.md`.

## Phase 2: Foundation
- [ ] T010-004 Create the necessary domain contracts/interfaces in `crates/scheduler/src/shards.rs` and invariant tests.
- [ ] T010-005 Implement ports/use cases in the appropriate module based on `specs/010-scalability-ha/plan.md`.
- [ ] T010-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T010-007 Create PostgreSQL integration and per-tenant isolation tests in `tests/integration/010-scalability-ha/`.

## Phase 3: US01 — Scale workers horizontally (P1)
**Independent test:** Given the queue grows and quota allows; When worker pools scale; Then throughput increases and no invalid claim exists.
- [ ] T010-008 [P] [US01] Write a Given/When/Then test for scaling workers horizontally in `tests/e2e/010-scalability-ha/us01.rs`.
- [ ] T010-009 [US01] Implement domain path and error handling for scaling workers horizontally in `crates/scheduler/src/shards.rs`.
- [ ] T010-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T010-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T010-012 [US01] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US01 in `tests/fault/010-scalability-ha/`.

## Phase 4: US02 — Sustain wait volume (P1)
**Independent test:** Given 100k runs await timers/events; When idle workers are drained; Then processes are not dedicated per run.
- [ ] T010-013 [P] [US02] Write a Given/When/Then test for sustaining wait volume in `tests/e2e/010-scalability-ha/us02.rs`.
- [ ] T010-014 [US02] Implement domain path and error handling for sustaining wait volume in `crates/scheduler/src/fairness.rs`.
- [ ] T010-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T010-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T010-017 [US02] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US02 in `tests/fault/010-scalability-ha/`.

## Phase 5: US03 — Survive an engine node failure (P1)
**Independent test:** Given a scheduler shard owner goes down; When another takes over the lease; Then timers and work are resumed without inconsistency.
- [ ] T010-018 [P] [US03] Write a Given/When/Then test for surviving an engine node failure in `tests/e2e/010-scalability-ha/us03.rs`.
- [ ] T010-019 [US03] Implement domain path and error handling for surviving an engine node failure in `crates/infra-postgres/src/partitioning.rs`.
- [ ] T010-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T010-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T010-022 [US03] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US03 in `tests/fault/010-scalability-ha/`.

## Phase 6: US04 — Isolate noisy neighbors (P1)
**Independent test:** Given tenant A fires a burst; When tenant B has ready tasks; Then fairness and quotas prevent starvation beyond an explicit budget.
- [ ] T010-023 [P] [US04] Write a Given/When/Then test for isolating noisy neighbors in `tests/e2e/010-scalability-ha/us04.rs`.
- [ ] T010-024 [US04] Implement domain path and error handling for isolating noisy neighbors in `infra/helm/values.yaml`.
- [ ] T010-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T010-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T010-027 [US04] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US04 in `tests/fault/010-scalability-ha/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T010-028 [US01] Implement scheduler owner leasing with generation in `crates/scheduler/src/shards.rs`.
- [ ] T010-029 [US01] Create a 1M WAITING synthetic load and p50/p95/p99 collection in `tests/performance/scenarios/`.
- [ ] T010-030 [US01] Validate DB/broker/shard failover under chaos in `tests/fault/failover/`.
- [ ] T010-031 [US01] Run PITR restore and journal integrity hash verification, documented in `docs/runbooks/`.

## Final Phase: Hardening / rollout
- [ ] T010-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/010-scalability-ha/`.
- [ ] T010-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T010-034 Run the benchmark defined in `specs/010-scalability-ha/plan.md` and publish the report in `tests/performance/reports/010-scalability-ha.md`.
- [ ] T010-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/010-scalability-ha/quickstart.md`.
- [ ] T010-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004, 005, 006, 008, 009.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
