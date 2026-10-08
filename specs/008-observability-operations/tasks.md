# Tasks: Distributed observability and operations

**Input:** `specs/008-observability-operations/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T008-001 Review FR-008-001..FR-008-006, SCs and ADRs; record gaps in `specs/008-observability-operations/research.md`.
- [ ] T008-002 [P] Define fixtures and contract validation in `tests/contract/008-observability-operations/fixtures/`.
- [ ] T008-003 [P] Document types, transitions and invariants in `specs/008-observability-operations/data-model.md`.

## Phase 2: Foundation
- [ ] T008-004 Create the necessary domain contracts/interfaces in `crates/telemetry/src/lib.rs` and invariant tests.
- [ ] T008-005 Implement ports/use cases in the appropriate module based on `specs/008-observability-operations/plan.md`.
- [ ] T008-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T008-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/008-observability-operations/`.

## Phase 3: US01 — Inspect timeline and attempt (P1)
**Independent test:** Given a Run with 3 attempts; When the operator opens the Timeline; Then each transition has a timestamp, cause and contextualized trace.
- [ ] T008-008 [P] [US01] Write a Given/When/Then test for inspecting timeline and attempt in `tests/e2e/008-observability-operations/us01.rs`.
- [ ] T008-009 [US01] Implement the domain path and error handling for inspecting timeline and attempt in `crates/telemetry/src/lib.rs`.
- [ ] T008-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T008-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T008-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/008-observability-operations/`.

## Phase 4: US02 — Correlate external calls (P1)
**Independent test:** Given an HTTP activity with trace propagation; When the operator opens the step's span; Then the trace crosses ingress, scheduler, worker and instrumented provider.
- [ ] T008-013 [P] [US02] Write a Given/When/Then test for correlating external calls in `tests/e2e/008-observability-operations/us02.rs`.
- [ ] T008-014 [US02] Implement the domain path and error handling for correlating external calls in `crates/application/src/operations.rs`.
- [ ] T008-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T008-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T008-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/008-observability-operations/`.

## Phase 5: US03 — Receive alerts for stuck runs (P1)
**Independent test:** Given outbox age/expired leases above the threshold; When the monitor detects the condition; Then an alert with runbook and correct namespace is emitted.
- [ ] T008-018 [P] [US03] Write a Given/When/Then test for receiving alerts for stuck runs in `tests/e2e/008-observability-operations/us03.rs`.
- [ ] T008-019 [US03] Implement the domain path and error handling for receiving alerts for stuck runs in `crates/api-http/src/routes/operations.rs`.
- [ ] T008-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T008-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T008-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/008-observability-operations/`.

## Phase 6: US04 — Execute an audited operational action (P1)
**Independent test:** Given an operator with retry permission; When they execute a retry with justification; Then new attempts appear without erasing the previous attempt.
- [ ] T008-023 [P] [US04] Write a Given/When/Then test for executing an audited operational action in `tests/e2e/008-observability-operations/us04.rs`.
- [ ] T008-024 [US04] Implement the domain path and error handling for executing an audited operational action in `frontend/apps/studio/src/features/runs/Timeline.tsx`.
- [ ] T008-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T008-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T008-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/008-observability-operations/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T008-028 [US01] Instrument Admission→Scheduler→Worker→External spans and W3C propagation in `crates/telemetry/src/`.
- [ ] T008-029 [US01] Create runbook for outbox stuck/compensation_failed in `docs/runbooks/`.
- [ ] T008-030 [US01] Test projection/journal inconsistency and display lag in the Studio in `tests/integration/008-observability-operations/`.

## Final Phase: Hardening / rollout
- [ ] T008-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/008-observability-operations/`.
- [ ] T008-032 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T008-033 Run the benchmark defined in `specs/008-observability-operations/plan.md` and publish the report in `tests/performance/reports/008-observability-operations.md`.
- [ ] T008-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/008-observability-operations/quickstart.md`.
- [ ] T008-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004, 007.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
