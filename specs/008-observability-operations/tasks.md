# Tasks: Distributed observability and operations

**Input:** `specs/008-observability-operations/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T008-001 Review FR-008-001..FR-008-006, SCs and ADRs; record gaps in `specs/008-observability-operations/research.md`.
- [x] T008-002 [P] Define fixtures and contract validation in `tests/contract/observability-operations/fixtures/`.
- [x] T008-003 [P] Document types, transitions and invariants in `specs/008-observability-operations/data-model.md`.

## Phase 2: Foundation
- [x] T008-004 Create the necessary domain contracts/interfaces in `crates/telemetry/src/lib.rs` and invariant tests.
- [x] T008-005 Implement ports/use cases in the appropriate module based on `specs/008-observability-operations/plan.md`.
- [x] T008-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [x] T008-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/observability-operations/`.

## Phase 3: US01 — Inspect timeline and attempt (P1)
**Independent test:** Given a Run with 3 attempts; When the operator opens the Timeline; Then each transition has a timestamp, cause and contextualized trace.
- [x] T008-008 [P] [US01] Write a Given/When/Then test for inspecting timeline and attempt in `tests/e2e/observability-operations/us01.rs`.
- [x] T008-009 [US01] Implement the domain path and error handling for inspecting timeline and attempt in `crates/telemetry/src/lib.rs`.
- [x] T008-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [x] T008-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [x] T008-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/observability-operations/`.

## Phase 4: US02 — Correlate external calls (P1)
**Independent test:** Given an HTTP activity with trace propagation; When the operator opens the step's span; Then the trace crosses ingress, scheduler, worker and instrumented provider.
- [x] T008-013 [P] [US02] Write a Given/When/Then test for correlating external calls in `tests/e2e/observability-operations/us02.rs`.
- [x] T008-014 [US02] Implement the domain path and error handling for correlating external calls in `crates/application/src/operations.rs`.
- [x] T008-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [x] T008-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [x] T008-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/observability-operations/`.

## Phase 5: US03 — Receive alerts for stuck runs (P1)
**Independent test:** Given outbox age/expired leases above the threshold; When the monitor detects the condition; Then an alert with runbook and correct namespace is emitted.
- [x] T008-018 [P] [US03] Write a Given/When/Then test for receiving alerts for stuck runs in `tests/e2e/observability-operations/us03.rs`.
- [x] T008-019 [US03] Implement the domain path and error handling for receiving alerts for stuck runs in `crates/api-http/src/routes/operations.rs`.
- [x] T008-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [x] T008-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [x] T008-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/observability-operations/`.

## Phase 6: US04 — Execute an audited operational action (P1)
**Independent test:** Given an operator with retry permission; When they execute a retry with justification; Then new attempts appear without erasing the previous attempt.
- [x] T008-023 [P] [US04] Write a Given/When/Then test for executing an audited operational action in `tests/e2e/observability-operations/us04.rs`.
- [x] T008-024 [US04] Implement the domain path and error handling for executing an audited operational action in `frontend/apps/studio/src/features/runs/Timeline.tsx`.
- [x] T008-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [x] T008-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [x] T008-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/observability-operations/`.

## Phase: Engineering design details (story-linked hardening)

- [x] T008-028 [US01] Instrument Admission→Scheduler→Worker→External spans and W3C propagation in `crates/telemetry/src/`.
- [x] T008-029 [US01] Create runbook for outbox stuck/compensation_failed in `docs/runbooks/`.
- [x] T008-030 [US01] Test projection/journal inconsistency and display lag in the Studio in `tests/integration/observability-operations/`.

## Final Phase: Hardening / rollout
- [x] T008-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/observability-operations/`.
- [x] T008-032 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [x] T008-033 Run the benchmark defined in `specs/008-observability-operations/plan.md` and publish the report in `tests/performance/reports/observability-operations.md`.
- [x] T008-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/008-observability-operations/quickstart.md`.
- [x] T008-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Evidence per task

Every `[x]` above has a test or an artifact; the evidence per story, the reviews and what could not be run are in [`quickstart.md`](./quickstart.md), per requirement in `docs/traceability.md`. Where the task named a path that the work did not use:

- T008-002: the contract validation is `tests/contract/observability-operations/main.rs` (the monitoring files against the code) and `rest/` (the new operations against the OpenAPI); the monitoring files are the fixtures, so there is no `fixtures/` directory.
- T008-009, T008-014: the paths named are where the work started; the derive of spans lives in `crates/application/src/telemetry.rs` and `crates/telemetry`, the services in `crates/application/src/operations.rs` and `runs.rs`.
- T008-024: the Studio's timeline is `features/runs/TimelineView.tsx` with the actions in `ops.ts` and `OperationDialog.tsx`.
- T008-029: `docs/runbooks/alerts.md#outbox-stuck` and the compensation sections of `sagas.md` that the rules point to.
- T008-030: the inconsistency is `tests/integration/observability-operations/projection.rs`; the lag as the Studio shows it is `RunDetailPage.test.tsx` and `e2e us01`.
- T008-031: the gRPC contract is unchanged (the lease already carried a `traceparent` field); the REST operations are in `rest/`, the alert webhook in the AsyncAPI suite of feature 004.
- T008-033: the report was measured on PostgreSQL 18 on one machine; its numbers are hypotheses.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004, 007.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.

## Studio operations completion

- [x] T008-036 Implement an Operations menu with namespace alerts, worker sessions and persistent
  identity drain/resume controls (FR-008-004/005, G181). Verify pagination, read/manage permission
  separation, scoped confirmation/reason, failure and late-response handling, and a real browser
  journey. Preserve existing backend audit, telemetry and tenant boundaries.
  — Implementation and 5 focused Operations scenarios pass; Studio and Playwright typechecks and
  focused lint pass. `frontend/e2e/tests/operations.spec.ts` is written with an isolated namespace,
  worker and guaranteed resume cleanup. The real Chromium/API journey passed on 2026-10-12
  (`target/usability/operations-browser.log`); it creates an explicit activity binding and removes it after cleanup.
