# Tasks: CLI, quickstarts, local testing and documentation

**Input:** `specs/012-developer-experience/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T012-001 Check FR-012-001..FR-012-006, SCs and ADRs; record gaps in `specs/012-developer-experience/research.md`.
- [ ] T012-002 [P] Define fixtures and contract validation in `tests/contract/012-developer-experience/fixtures/`.
- [ ] T012-003 [P] Document types, transitions and invariants in `specs/012-developer-experience/data-model.md`.

## Phase 2: Foundation
- [ ] T012-004 Create the necessary domain contracts/interfaces in `tools/cli/src/main.rs` and invariant tests.
- [ ] T012-005 Implement ports/use cases in the appropriate module based on `specs/012-developer-experience/plan.md`.
- [ ] T012-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T012-007 Create PostgreSQL integration tests and tenant isolation tests in `tests/integration/012-developer-experience/`.

## Phase 3: US01 — Start first workflow quickly (P1)
**Independent test:** Given a developer follows the quickstart; When they run compose + CLI + SDK; Then a two-step workflow executes and is visible in Studio.
- [ ] T012-008 [P] [US01] Write Given/When/Then test for starting the first workflow quickly in `tests/e2e/012-developer-experience/us01.rs`.
- [ ] T012-009 [US01] Implement domain path and error handling for starting the first workflow quickly in `tools/cli/src/main.rs`.
- [ ] T012-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T012-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T012-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/012-developer-experience/`.

## Phase 4: US02 — Test Saga locally (P1)
**Independent test:** Given a developer has a payment saga; When they inject a failure into the mock provider; Then they see reconciliation and compensation in the development emulator.
- [ ] T012-013 [P] [US02] Write Given/When/Then test for testing a saga locally in `tests/e2e/012-developer-experience/us02.rs`.
- [ ] T012-014 [US02] Implement domain path and error handling for testing a saga locally in `examples/payment-saga/`.
- [ ] T012-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T012-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T012-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/012-developer-experience/`.

## Phase 5: US03 — Generate examples per SDK (P1)
**Independent test:** Given a new developer uses Go or C#; When they copy the example and run it; Then the example produces a contract/trace consistent with the other SDKs.
- [ ] T012-018 [P] [US03] Write Given/When/Then test for generating examples per SDK in `tests/e2e/012-developer-experience/us03.rs`.
- [ ] T012-019 [US03] Implement domain path and error handling for generating examples per SDK in `examples/hello-world/`.
- [ ] T012-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T012-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T012-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/012-developer-experience/`.

## Phase 6: US04 — Diagnose publishing errors (P2)
**Independent test:** Given a manifest has an invalid node; When the CLI validate runs; Then they receive errors with path, remediation and docs links.
- [ ] T012-023 [P] [US04] Write Given/When/Then test for diagnosing publishing errors in `tests/e2e/012-developer-experience/us04.rs`.
- [ ] T012-024 [US04] Implement domain path and error handling for diagnosing publishing errors in `docs/quickstarts/`.
- [ ] T012-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T012-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T012-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/012-developer-experience/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T012-028 [US01] Create Rust CLI `df workflow validate|publish|run|inspect` in `tools/cli/src/`.
- [ ] T012-029 [US01] Write TS/Go/Python/C#/Java quickstarts with mocks and test harness in `docs/quickstarts/`.
- [ ] T012-030 [US01] Implement self-check of environment, broker and protocol version in `tools/cli/src/commands/doctor.rs`.

## Final Phase: Hardening / rollout
- [ ] T012-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/012-developer-experience/`.
- [ ] T012-032 Apply lint and acyclic-dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T012-033 Run the benchmark defined in `specs/012-developer-experience/plan.md` and publish the report in `tests/performance/reports/012-developer-experience.md`.
- [ ] T012-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/012-developer-experience/quickstart.md`.
- [ ] T012-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004, 005, 006, 007.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
