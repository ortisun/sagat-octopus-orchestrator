# Tasks: Durable execution engine and scheduler

**Input:** `specs/002-durable-execution/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T002-001 Check FR-002-001..FR-002-009, SCs and ADRs; record gaps in `specs/002-durable-execution/research.md`.
  - Done: Gaps G19-G30 and decisions D25-D36 in `research.md`.
- [x] T002-002 [P] Define fixtures and contract validation in `tests/contract/durable-execution/fixtures/`.
  - Done: No static fixtures: the live conformance suites generate their own data: `tests/contract/durable-execution/{rest,grpc}/` (REST against the OpenAPI, the worker protocol over a real socket).
- [x] T002-003 [P] Document types, transitions and invariants in `specs/002-durable-execution/data-model.md`.
  - Done: `data-model.md`.

## Phase 2: Foundation
- [x] T002-004 Create the necessary domain contracts/interfaces in `crates/domain/src/workflow.rs` and invariant tests.
  - Done: `crates/domain/src/workflow.rs` (definition, retry policy, child node) and `crates/domain/src/execution/` (events, reducer, planner, commands); tests in `crates/domain/tests/{workflow,execution,execution_properties}.rs`.
- [x] T002-005 Implement ports/use cases in the appropriate module based on `specs/002-durable-execution/plan.md`.
  - Done: Ports and use cases in `crates/application/src/{definitions,runs,worker,execution,maintenance}.rs`; scheduler and dispatcher in `crates/scheduler`; expression subset in `crates/infra-expr`; IR codec in `crates/infra-ir`.
- [x] T002-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
  - Done: `crates/infra-postgres/migrations/2026100900{00,01}_*` with `.down.sql`; forward/rollback: `tests/integration/platform-foundation/migrations.rs` (covers every table).
- [x] T002-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/durable-execution/`.
  - Done: `tests/integration/durable-execution/` (117 tests against real PostgreSQL, tenant isolation in `isolation.rs`).

## Phase 3: US01 — Execute a DAG and persist checkpoints (P1)
**Independent test:** Given a published definition with A→B; When the client starts the run; Then A and B are executed, and checkpoints and journal record progress.
- [x] T002-008 [P] [US01] Write a Given/When/Then test to execute a DAG and persist checkpoints in `tests/e2e/durable-execution/us01.rs`.
  - Done: `tests/e2e/durable-execution/stories.rs::us01_*` (REST + gRPC).
- [x] T002-009 [US01] Implement the domain path and error handling to execute a DAG and persist checkpoints in `crates/domain/src/workflow.rs`.
  - Done: `crates/domain/src/workflow.rs` and `crates/domain/src/execution/`.
- [x] T002-010 [US01] Integrate adapters and persistence for the US01 scenario in `crates/application/` and `crates/infra-postgres/`.
  - Done: `crates/application/src/` and `crates/infra-postgres/src/{execution,definitions,workers,maintenance}.rs`.
- [x] T002-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to the US01 scenario in `crates/telemetry/`.
  - Done: Metrics and spans fields in `crates/api-grpc/src/service.rs` and `crates/scheduler/src/{passes,outbox}.rs`; tenant and secret protection tested in `tests/contract/durable-execution/grpc/protocol.rs`.
- [x] T002-012 [US01] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US01 in `tests/fault/durable-execution/`.
  - Done: Duplicates, timeouts, worker failure and tenant boundary: `tests/integration/durable-execution/{faults,concurrency,isolation,scheduler}.rs` (not in a separate `tests/fault/` directory).

## Phase 4: US02 — Recover after a worker crash (P1)
**Independent test:** Given a running step and a registered lease; When the worker dies and the lease expires; Then another worker resumes without reusing the stale generation.
- [x] T002-013 [P] [US02] Write a Given/When/Then test to recover after a worker crash in `tests/e2e/durable-execution/us02.rs`.
  - Done: `tests/e2e/durable-execution/stories.rs::us02_*`.
- [x] T002-014 [US02] Implement the domain path and error handling to recover after a worker crash in `crates/domain/src/execution.rs`.
  - Done: `crates/domain/src/execution/{state,command}.rs` (lease loss, fencing).
- [x] T002-015 [US02] Integrate adapters and persistence for the US02 scenario in `crates/application/` and `crates/infra-postgres/`.
  - Done: `crates/application/src/worker.rs`, `crates/scheduler/src/passes.rs` (`leases`), `crates/infra-postgres/src/{execution,maintenance}.rs`.
- [x] T002-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to the US02 scenario in `crates/telemetry/`.
  - Done: `scheduler_actions_total`, `grpc_requests_total`; see the runbook `docs/runbooks/execution-engine.md`.
- [x] T002-017 [US02] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US02 in `tests/fault/durable-execution/`.
  - Done: `tests/integration/durable-execution/{faults,scheduler,concurrency}.rs`, `tests/e2e/durable-execution/engine.rs::the_engine_takes_a_dead_workers_step_back_by_itself`.

## Phase 5: US03 — Wait without consuming compute (P1)
**Independent test:** Given a Run enters a one-hour timer; When no worker remains allocated; Then the scheduler creates a new task when the timer expires.
- [x] T002-018 [P] [US03] Write a Given/When/Then test to wait without consuming compute in `tests/e2e/durable-execution/us03.rs`.
  - Done: `tests/e2e/durable-execution/stories.rs::us03_*` (timer and signal).
- [x] T002-019 [US03] Implement the domain path and error handling to wait without consuming compute in `crates/application/src/commands.rs`.
  - Done: `crates/domain/src/execution/{plan,command}.rs` (timers, waits) and `crates/scheduler/src/passes.rs`.
- [x] T002-020 [US03] Integrate adapters and persistence for the US03 scenario in `crates/application/` and `crates/infra-postgres/`.
  - Done: `crates/infra-postgres/src/{execution,maintenance}.rs` (partial indexes make a suspended run a row).
- [x] T002-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to the US03 scenario in `crates/telemetry/`.
  - Done: As T002-016.
- [x] T002-022 [US03] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US03 in `tests/fault/durable-execution/`.
  - Done: `tests/integration/durable-execution/{scheduler,faults,flows}.rs`; the benchmark `tests/performance/durable-execution/main.rs`.

## Phase 6: US04 — View a redelivered attempt (P1)
**Independent test:** Given NATS delivers a duplicate task; When two workers contend for the claim; Then only one generation and commit are accepted.
- [x] T002-023 [P] [US04] Write a Given/When/Then test to view a redelivered attempt in `tests/e2e/durable-execution/us04.rs`.
  - Done: `tests/e2e/durable-execution/stories.rs::us04_*` (eight workers contend over gRPC, a duplicated report commits once).
- [x] T002-024 [US04] Implement the domain path and error handling to view a redelivered attempt in `crates/scheduler/src/claim.rs`.
  - Done: `crates/infra-postgres/src/workers.rs` and `crates/application/src/worker.rs` (claim with `SKIP LOCKED`, fencing); the engine process: `apps/execution-engine`.
- [x] T002-025 [US04] Integrate adapters and persistence for the US04 scenario in `crates/application/` and `crates/infra-postgres/`.
  - Done: As T002-010.
- [x] T002-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to the US04 scenario in `crates/telemetry/`.
  - Done: As T002-016.
- [x] T002-027 [US04] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US04 in `tests/fault/durable-execution/`.
  - Done: `tests/integration/durable-execution/{concurrency,faults}.rs`, `tests/contract/durable-execution/grpc/protocol.rs`.

## Phase: Engineering design details (story-linked hardening)

- [x] T002-028 [US01] Implement pure reducer `apply(event,state) -> state` and golden tests in `crates/domain/src/execution.rs`.
  - Done: `crates/domain/src/execution/state.rs::RunState::apply`; golden scenarios in `crates/domain/tests/execution.rs`, random driver in `execution_properties.rs`.
- [x] T002-029 [US01] Implement `READY→LEASED` with `FOR UPDATE SKIP LOCKED`, generation+1, lease_token in `crates/scheduler/src/claim.rs`.
  - Done: `crates/infra-postgres/src/workers.rs` + `crates/application/src/worker.rs` (the claim lives with the worker service, not in `scheduler/claim.rs`).
- [x] T002-030 [US01] Implement CommitStep TX atomic journal/steps/outbox/timers in `crates/application/src/commands.rs`.
  - Done: `crates/application/src/execution.rs::Engine::run` and `RunTx::persist` in `crates/infra-postgres/src/execution.rs` (not `commands.rs`).
- [x] T002-031 [US01] Reconciler requeue in `crates/scheduler/src/reconcile.rs` and chaos test for connection drop after COMMIT.
  - Done: `crates/scheduler/src/{passes,outbox}.rs`; chaos test (connections killed with `pg_terminate_backend`) in `tests/integration/durable-execution/faults.rs`; broker outage in `nats.rs`.

## Final Phase: Hardening / rollout
- [x] T002-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/durable-execution/`.
  - Done: `tests/contract/durable-execution/{rest,grpc}/` (REST against the OpenAPI, 15 gRPC protocol tests). AsyncAPI: the events are published as hints only (no AsyncAPI operation is implemented yet).
- [x] T002-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
  - Done: `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/check-architecture.py` and its tests; CI jobs `rust`, `architecture`.
- [x] T002-034 Run the benchmark defined in `specs/002-durable-execution/plan.md` and publish the report in `tests/performance/reports/durable-execution.md`.
  - Done: `tests/performance/durable-execution/main.rs`; report `tests/performance/reports/durable-execution.md`.
- [x] T002-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/002-durable-execution/quickstart.md`.
  - Done: Security review: see the evidence table in `quickstart.md`; runbook `docs/runbooks/execution-engine.md`.
- [x] T002-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.
  - Done: `docs/traceability.md` (002 evidence table).

## Dependencies & independent delivery

- This feature requires: 001.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
