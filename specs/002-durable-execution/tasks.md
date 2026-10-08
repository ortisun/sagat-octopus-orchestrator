# Tasks: Durable execution engine and scheduler

**Input:** `specs/002-durable-execution/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T002-001 Check FR-002-001..FR-002-009, SCs and ADRs; record gaps in `specs/002-durable-execution/research.md`.
- [ ] T002-002 [P] Define fixtures and contract validation in `tests/contract/002-durable-execution/fixtures/`.
- [ ] T002-003 [P] Document types, transitions and invariants in `specs/002-durable-execution/data-model.md`.

## Phase 2: Foundation
- [ ] T002-004 Create the necessary domain contracts/interfaces in `crates/domain/src/workflow.rs` and invariant tests.
- [ ] T002-005 Implement ports/use cases in the appropriate module based on `specs/002-durable-execution/plan.md`.
- [ ] T002-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T002-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/002-durable-execution/`.

## Phase 3: US01 — Execute a DAG and persist checkpoints (P1)
**Independent test:** Given a published definition with A→B; When the client starts the run; Then A and B are executed, and checkpoints and journal record progress.
- [ ] T002-008 [P] [US01] Write a Given/When/Then test to execute a DAG and persist checkpoints in `tests/e2e/002-durable-execution/us01.rs`.
- [ ] T002-009 [US01] Implement the domain path and error handling to execute a DAG and persist checkpoints in `crates/domain/src/workflow.rs`.
- [ ] T002-010 [US01] Integrate adapters and persistence for the US01 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T002-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to the US01 scenario in `crates/telemetry/`.
- [ ] T002-012 [US01] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US01 in `tests/fault/002-durable-execution/`.

## Phase 4: US02 — Recover after a worker crash (P1)
**Independent test:** Given a running step and a registered lease; When the worker dies and the lease expires; Then another worker resumes without reusing the stale generation.
- [ ] T002-013 [P] [US02] Write a Given/When/Then test to recover after a worker crash in `tests/e2e/002-durable-execution/us02.rs`.
- [ ] T002-014 [US02] Implement the domain path and error handling to recover after a worker crash in `crates/domain/src/execution.rs`.
- [ ] T002-015 [US02] Integrate adapters and persistence for the US02 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T002-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to the US02 scenario in `crates/telemetry/`.
- [ ] T002-017 [US02] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US02 in `tests/fault/002-durable-execution/`.

## Phase 5: US03 — Wait without consuming compute (P1)
**Independent test:** Given a Run enters a one-hour timer; When no worker remains allocated; Then the scheduler creates a new task when the timer expires.
- [ ] T002-018 [P] [US03] Write a Given/When/Then test to wait without consuming compute in `tests/e2e/002-durable-execution/us03.rs`.
- [ ] T002-019 [US03] Implement the domain path and error handling to wait without consuming compute in `crates/application/src/commands.rs`.
- [ ] T002-020 [US03] Integrate adapters and persistence for the US03 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T002-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to the US03 scenario in `crates/telemetry/`.
- [ ] T002-022 [US03] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US03 in `tests/fault/002-durable-execution/`.

## Phase 6: US04 — View a redelivered attempt (P1)
**Independent test:** Given NATS delivers a duplicate task; When two workers contend for the claim; Then only one generation and commit are accepted.
- [ ] T002-023 [P] [US04] Write a Given/When/Then test to view a redelivered attempt in `tests/e2e/002-durable-execution/us04.rs`.
- [ ] T002-024 [US04] Implement the domain path and error handling to view a redelivered attempt in `crates/scheduler/src/claim.rs`.
- [ ] T002-025 [US04] Integrate adapters and persistence for the US04 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T002-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to the US04 scenario in `crates/telemetry/`.
- [ ] T002-027 [US04] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US04 in `tests/fault/002-durable-execution/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T002-028 [US01] Implement pure reducer `apply(event,state) -> state` and golden tests in `crates/domain/src/execution.rs`.
- [ ] T002-029 [US01] Implement `READY→LEASED` with `FOR UPDATE SKIP LOCKED`, generation+1, lease_token in `crates/scheduler/src/claim.rs`.
- [ ] T002-030 [US01] Implement CommitStep TX atomic journal/steps/outbox/timers in `crates/application/src/commands.rs`.
- [ ] T002-031 [US01] Reconciler requeue in `crates/scheduler/src/reconcile.rs` and chaos test for connection drop after COMMIT.

## Final Phase: Hardening / rollout
- [ ] T002-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/002-durable-execution/`.
- [ ] T002-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T002-034 Run the benchmark defined in `specs/002-durable-execution/plan.md` and publish the report in `tests/performance/reports/002-durable-execution.md`.
- [ ] T002-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/002-durable-execution/quickstart.md`.
- [ ] T002-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
