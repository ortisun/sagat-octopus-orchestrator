# Tasks: Visual Studio, Monaco and runtime graph

**Input:** `specs/007-workflow-studio/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T007-001 Review FR-007-001..FR-007-008, SCs and ADRs; record gaps in `specs/007-workflow-studio/research.md`.
- [ ] T007-002 [P] Define fixtures and contract validation in `tests/contract/007-workflow-studio/fixtures/`.
- [ ] T007-003 [P] Document types, transitions and invariants in `specs/007-workflow-studio/data-model.md`.

## Phase 2: Foundation
- [ ] T007-004 Create the necessary domain contracts/interfaces in `frontend/apps/studio/src/features/workflows/WorkflowCanvas.tsx` and invariant tests.
- [ ] T007-005 Implement ports/use cases in the appropriate module based on `specs/007-workflow-studio/plan.md`.
- [ ] T007-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T007-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/007-workflow-studio/`.

## Phase 3: US01 — Create a workflow without programming (P1)
**Independent test:** Given an author in the Studio; When they drag activity/condition/parallel/saga and configure parameters; Then the validator displays errors and saves the IR draft.
- [ ] T007-008 [P] [US01] Write a Given/When/Then test for creating a workflow without programming in `tests/e2e/007-workflow-studio/us01.rs`.
- [ ] T007-009 [US01] Implement the domain path and error handling for creating a workflow without programming in `frontend/apps/studio/src/features/workflows/WorkflowCanvas.tsx`.
- [ ] T007-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T007-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T007-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/007-workflow-studio/`.

## Phase 4: US02 — Edit code in a node (P1)
**Independent test:** Given a selected Script node; When the user edits TS/Python/C# and tests; Then Monaco shows the result in a sandbox and the version is only published after validation.
- [ ] T007-013 [P] [US02] Write a Given/When/Then test for editing code in a node in `tests/e2e/007-workflow-studio/us02.rs`.
- [ ] T007-014 [US02] Implement the domain path and error handling for editing code in a node in `frontend/apps/studio/src/features/workflows/NodeInspector.tsx`.
- [ ] T007-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T007-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T007-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/007-workflow-studio/`.

## Phase 5: US03 — Analyze a real execution (P1)
**Independent test:** Given a Run with retries and an untraversed branch; When the operator opens the graph; Then attempts, duration, states, compensations and timeline reflect the journal.
- [ ] T007-018 [P] [US03] Write a Given/When/Then test for analyzing a real execution in `tests/e2e/007-workflow-studio/us03.rs`.
- [ ] T007-019 [US03] Implement the domain path and error handling for analyzing a real execution in `frontend/apps/studio/src/features/runs/ExecutionGraph.tsx`.
- [ ] T007-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T007-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T007-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/007-workflow-studio/`.

## Phase 6: US04 — Investigate and act on a failure (P1)
**Independent test:** Given a failed run and an authorized operator; When they open the trace, adjust the operational decision and perform a retry; Then the action is audited and does not overwrite the history.
- [ ] T007-023 [P] [US04] Write a Given/When/Then test for investigating and acting on a failure in `tests/e2e/007-workflow-studio/us04.rs`.
- [ ] T007-024 [US04] Implement the domain path and error handling for investigating and acting on a failure in `frontend/apps/studio/src/features/runs/Timeline.tsx`.
- [ ] T007-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T007-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T007-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/007-workflow-studio/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T007-028 [US01] Implement canvas with IR validation and diff in `frontend/apps/studio/src/features/workflows/`.
- [ ] T007-029 [US01] Render runtime graph via journal projection cursor in `frontend/apps/studio/src/features/runs/ExecutionGraph.tsx`.
- [ ] T007-030 [US01] Add Monaco/test-run and per-type schema form in `frontend/apps/studio/src/features/scripts/`.
- [ ] T007-031 [US01] Implement manual review and compensation UI with double confirmation and RBAC in `frontend/apps/studio/src/features/runs/`.

## Final Phase: Hardening / rollout
- [ ] T007-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/007-workflow-studio/`.
- [ ] T007-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T007-034 Run the benchmark defined in `specs/007-workflow-studio/plan.md` and publish the report in `tests/performance/reports/007-workflow-studio.md`.
- [ ] T007-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/007-workflow-studio/quickstart.md`.
- [ ] T007-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
