# Tasks: Native sagas and compensations

**Input:** `specs/003-sagas-compensations/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T003-001 Check FR-003-001..FR-003-007, SCs and ADRs; record gaps in `specs/003-sagas-compensations/research.md`. — gaps G51–G65 and decisions D59–D71 in `research.md`; the review of the implementation is in `quickstart.md` and D71.
- [x] T003-002 [P] Define fixtures and contract validation in `tests/contract/sagas-compensations/fixtures/`. — there is no `fixtures/` directory: the bodies of the operations are built inline from the shipped example and validated against the OpenAPI schemas in `rest/conformance.rs`; the worker protocol is checked in `grpc/protocol.rs`.
- [x] T003-003 [P] Document types, transitions and invariants in `specs/003-sagas-compensations/data-model.md`. — `data-model.md` (IR fields, tables, state machines, invariants, lock order, migrations).

## Phase 2: Foundation
- [x] T003-004 Create the necessary domain contracts/interfaces in `crates/domain/src/saga.rs` and invariant tests. — `crates/domain/src/saga.rs` (policies, outcomes, the plan of a definition), `workflow.rs` (the saga node and the derived nodes) and `execution/sagas.rs` (the coordinator), with `crates/domain/tests/sagas.rs` (60 scenarios) and the extended property tests.
- [x] T003-005 Implement ports/use cases in the appropriate module based on `specs/003-sagas-compensations/plan.md`. — `crates/application/src/{execution,worker,runs,definitions}.rs` (keys, task kinds and targets, `RunService::review`, plans, the breaker's policy), plus the routes of `crates/api-http` and `crates/api-grpc`.
- [x] T003-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — `20261011000000_sagas.up.sql` / `.down.sql`: additive, reversible, checked by the rollback test of feature 001.
- [x] T003-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/sagas-compensations/`. — `tests/integration/sagas-compensations/` (74 tests: plans, compensations, lookups, decisions, stopping, children, the breaker).

## Phase 3: US01 — Define compensation on publish (P1)
**Independent test:** Given a reserve action with compensate release; When the author publishes a Saga; Then the plan records dependencies and durable compensation IDs.
- [x] T003-008 [P] [US01] Write Given/When/Then test for defining compensation on publish in `tests/e2e/sagas-compensations/us01.rs`. — `us01.rs`.
- [x] T003-009 [US01] Implement domain path and error handling for defining compensation on publish in `crates/domain/src/saga.rs`. — in `crates/domain/src/saga.rs` and `workflow.rs`: the plan is a function of the definition, validated when it is published.
- [x] T003-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/definitions.rs` (hands the plans over at publish, serves them) and `crates/infra-postgres/src/definitions.rs` (`saga_plans`, `saga_plan_entries`).
- [x] T003-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — the counters are taken after the commit in `crates/infra-postgres/src/signals.rs`, through the `metrics` facade that `crates/telemetry` installs (D69); `e2e/.../telemetry.rs` checks them and that no label names a tenant, run, node or payment.
- [x] T003-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/sagas-compensations/`. — `plan::*` (immutability, tenant boundary), `fault_sagas_compensations`.

## Phase 4: US02 — Compensate after permanent failure (P1)
**Independent test:** Given reserve completed and submit failed permanently; When the Saga Coordinator triggers recovery; Then release executes idempotently and the run indicates COMPENSATED.
- [x] T003-013 [P] [US02] Write Given/When/Then test for compensating after permanent failure in `tests/e2e/sagas-compensations/us02.rs`. — `us02.rs`.
- [x] T003-014 [US02] Implement domain path and error handling for compensating after permanent failure in `crates/saga/src/coordinator.rs`. — there is no `crates/saga`: the coordinator is `crates/domain/src/execution/sagas.rs` (G51, ADR-0010).
- [x] T003-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/worker.rs` (task kind, target, keys) and `crates/infra-postgres/src/execution.rs` (`upsert_saga`, projections).
- [x] T003-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — `signals.rs`; the lease, the fence and the bindings of 009 apply to a compensation because it is a step.
- [x] T003-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/sagas-compensations/`. — `crashes.rs`, `races.rs`, `provider.rs` and `chaos.rs` in `tests/fault/sagas-compensations/`.

## Phase 5: US03 — Reconcile an uncertain effect (P1)
**Independent test:** Given the provider accepted the payment but the response was lost; When the engine restarts; Then a reconciliation step queries the outcome before retrying or compensating.
- [x] T003-018 [P] [US03] Write Given/When/Then test for reconciling an uncertain effect in `tests/e2e/sagas-compensations/us03.rs`. — `us03.rs` (including an engine process stopped with the lease held and restarted).
- [x] T003-019 [US03] Implement domain path and error handling for reconciling an uncertain effect in `crates/saga/src/reconcile.rs`. — there is no `crates/saga/src/reconcile.rs`: the verdicts are in `crates/domain/src/execution/command.rs` and the answer's handling in `crates/application/src/worker.rs` (`complete_reconciliation`).
- [x] T003-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — `worker.rs`, `execution.rs`.
- [x] T003-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — `signals.rs` (`reconciliations_total`).
- [x] T003-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/sagas-compensations/`. — `crashes::a_worker_that_dies_during_the_lookup...`, `races::a_lease_that_runs_out_while_the_answer_arrives...`, `provider::*`.

## Phase 6: US04 — Resolve a compensation failure (P1)
**Independent test:** Given compensation failed after retries; When the operator reviews and intervenes; Then the state and the reason are audited without erasing previous events.
- [x] T003-023 [P] [US04] Write Given/When/Then test for resolving a compensation failure in `tests/e2e/sagas-compensations/us04.rs`. — `us04.rs`.
- [x] T003-024 [US04] Implement domain path and error handling for resolving a compensation failure in `crates/infra-postgres/src/sagas.rs`. — there is no `crates/infra-postgres/src/sagas.rs`: the saga projections are written by `execution.rs` (`upsert_saga`) and the breaker is `breaker.rs`.
- [x] T003-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — `RunService::review` and the operations route.
- [x] T003-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — `signals.rs` (`operator_decisions_total`); the audit record is written in the decision's transaction.
- [x] T003-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/sagas-compensations/`. — `review::*` (racing operators, replays, permissions, tenant), `crashes::a_run_waiting_for_a_person_is_still_waiting_after_every_process_restarted`.

## Phase: Engineering design details (story-linked hardening)

- [x] T003-028 [US01] Define compensation graph with reverse topological dependencies in `crates/domain/src/saga.rs`. — `saga.rs::reverse_dependencies` (nearest entries downstream, the join continuation, linear), checked against the long way.
- [x] T003-029 [US01] Persist compensation intents and reconciliation handles in `crates/infra-postgres/src/sagas.rs`. — `run_sagas`, `run_saga_entries` and the plan tables (the compensation intents and their durable ids), in `execution.rs` and `definitions.rs`; the lookup's handle is the target key of the task.
- [x] T003-030 [US01] Test provider accepted-before-timeout with fake HTTP + duplicate return in `tests/fault/sagas-compensations/`. — `provider.rs`: a real HTTP provider that records the payment and answers too late, one that fails before accepting, and one that is asked to release twice.

## Final Phase: Hardening / rollout
- [x] T003-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/sagas-compensations/`. — `tests/contract/sagas-compensations/{rest,grpc}/`; the OpenAPI lint (10 warnings, the baseline) and the AsyncAPI validation pass.
- [x] T003-032 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`. — `cargo clippy --workspace --all-targets -- -D warnings` and `scripts/check-architecture.py` pass; `docs/architecture/sagas.md`, `execution-semantics.md` and `worker-runtime-protocol.md` say what is built.
- [x] T003-033 Run the benchmark defined in `specs/003-sagas-compensations/plan.md` and publish the report in `tests/performance/reports/sagas-compensations.md`. — `tests/performance/reports/sagas-compensations.md`; one hypothesis failed and was restated openly (16 workers over a pool of 8, G57).
- [x] T003-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/003-sagas-compensations/quickstart.md`. — two independent reviews, sixteen findings, all reproduced and fixed (D71); evidence in `quickstart.md`; the runbook is `docs/runbooks/sagas.md`.
- [x] T003-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — `docs/traceability.md` (the 003 evidence table, with SC-003-003 and SC-003-004 partial and why).

## Dependencies & independent delivery

- This feature requires: 001, 002.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
