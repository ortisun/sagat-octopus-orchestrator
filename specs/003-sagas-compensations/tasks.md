# Tasks: Native sagas and compensations

**Input:** `specs/003-sagas-compensations/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T003-001 Check FR-003-001..FR-003-007, SCs and ADRs; record gaps in `specs/003-sagas-compensations/research.md`.
- [ ] T003-002 [P] Define fixtures and contract validation in `tests/contract/003-sagas-compensations/fixtures/`.
- [ ] T003-003 [P] Document types, transitions and invariants in `specs/003-sagas-compensations/data-model.md`.

## Phase 2: Foundation
- [ ] T003-004 Create the necessary domain contracts/interfaces in `crates/domain/src/saga.rs` and invariant tests.
- [ ] T003-005 Implement ports/use cases in the appropriate module based on `specs/003-sagas-compensations/plan.md`.
- [ ] T003-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T003-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/003-sagas-compensations/`.

## Phase 3: US01 — Define compensation on publish (P1)
**Independent test:** Given a reserve action with compensate release; When the author publishes a Saga; Then the plan records dependencies and durable compensation IDs.
- [ ] T003-008 [P] [US01] Write Given/When/Then test for defining compensation on publish in `tests/e2e/003-sagas-compensations/us01.rs`.
- [ ] T003-009 [US01] Implement domain path and error handling for defining compensation on publish in `crates/domain/src/saga.rs`.
- [ ] T003-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T003-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T003-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/003-sagas-compensations/`.

## Phase 4: US02 — Compensate after permanent failure (P1)
**Independent test:** Given reserve completed and submit failed permanently; When the Saga Coordinator triggers recovery; Then release executes idempotently and the run indicates COMPENSATED.
- [ ] T003-013 [P] [US02] Write Given/When/Then test for compensating after permanent failure in `tests/e2e/003-sagas-compensations/us02.rs`.
- [ ] T003-014 [US02] Implement domain path and error handling for compensating after permanent failure in `crates/saga/src/coordinator.rs`.
- [ ] T003-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T003-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T003-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/003-sagas-compensations/`.

## Phase 5: US03 — Reconcile an uncertain effect (P1)
**Independent test:** Given the provider accepted the payment but the response was lost; When the engine restarts; Then a reconciliation step queries the outcome before retrying or compensating.
- [ ] T003-018 [P] [US03] Write Given/When/Then test for reconciling an uncertain effect in `tests/e2e/003-sagas-compensations/us03.rs`.
- [ ] T003-019 [US03] Implement domain path and error handling for reconciling an uncertain effect in `crates/saga/src/reconcile.rs`.
- [ ] T003-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T003-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T003-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/003-sagas-compensations/`.

## Phase 6: US04 — Resolve a compensation failure (P1)
**Independent test:** Given compensation failed after retries; When the operator reviews and intervenes; Then the state and the reason are audited without erasing previous events.
- [ ] T003-023 [P] [US04] Write Given/When/Then test for resolving a compensation failure in `tests/e2e/003-sagas-compensations/us04.rs`.
- [ ] T003-024 [US04] Implement domain path and error handling for resolving a compensation failure in `crates/infra-postgres/src/sagas.rs`.
- [ ] T003-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T003-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T003-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/003-sagas-compensations/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T003-028 [US01] Define compensation graph with reverse topological dependencies in `crates/domain/src/saga.rs`.
- [ ] T003-029 [US01] Persist compensation intents and reconciliation handles in `crates/infra-postgres/src/sagas.rs`.
- [ ] T003-030 [US01] Test provider accepted-before-timeout with fake HTTP + duplicate return in `tests/fault/003-sagas-compensations/`.

## Final Phase: Hardening / rollout
- [ ] T003-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/003-sagas-compensations/`.
- [ ] T003-032 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T003-033 Run the benchmark defined in `specs/003-sagas-compensations/plan.md` and publish the report in `tests/performance/reports/003-sagas-compensations.md`.
- [ ] T003-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/003-sagas-compensations/quickstart.md`.
- [ ] T003-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
