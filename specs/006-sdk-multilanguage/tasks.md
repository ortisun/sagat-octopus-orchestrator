# Tasks: TypeScript, Go, Python, C# and Java SDKs

**Input:** `specs/006-sdk-multilanguage/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T006-001 Check FR-006-001..FR-006-008, SCs and ADRs; record gaps in `specs/006-sdk-multilanguage/research.md`.
- [ ] T006-002 [P] Define fixtures and contract validation in `tests/contract/006-sdk-multilanguage/fixtures/`.
- [ ] T006-003 [P] Document types, transitions and invariants in `specs/006-sdk-multilanguage/data-model.md`.

## Phase 2: Foundation
- [ ] T006-004 Create the necessary domain contracts/interfaces in `proto/sagat_octopus_orchestrator/v1/worker.proto` and invariant tests.
- [ ] T006-005 Implement ports/use cases in the appropriate module based on `specs/006-sdk-multilanguage/plan.md`.
- [ ] T006-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T006-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/006-sdk-multilanguage/`.

## Phase 3: US01 — Create an application run in C# (P1)
**Independent test:** Given an ASP.NET Core service; When it registers a client via DI and calls StartAsync; Then Run is created with idempotencyKey and trace context propagated.
- [ ] T006-008 [P] [US01] Write a Given/When/Then test for creating an application run in C# in `tests/e2e/006-sdk-multilanguage/us01.rs`.
- [ ] T006-009 [US01] Implement the domain path and error handling for creating an application run in C# in `proto/sagat_octopus_orchestrator/v1/worker.proto`.
- [ ] T006-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T006-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T006-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/006-sdk-multilanguage/`.

## Phase 4: US02 — Register workers in any SDK (P1)
**Independent test:** Given a developer with a TS/Go/Python/C#/Java activity; When they register a worker and receive a task; Then the server validates capabilities and completes the step via the common protocol.
- [ ] T006-013 [P] [US02] Write a Given/When/Then test for registering workers in any SDK in `tests/e2e/006-sdk-multilanguage/us02.rs`.
- [ ] T006-014 [US02] Implement the domain path and error handling for registering workers in any SDK in `sdks/typescript/src/index.ts`.
- [ ] T006-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T006-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T006-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/006-sdk-multilanguage/`.

## Phase 5: US03 — Write a typed Saga (P1)
**Independent test:** Given an activity with compensate/reconcile defined; When a workflow fails; Then all SDKs receive the same semantics and canonical errors.
- [ ] T006-018 [P] [US03] Write a Given/When/Then test for writing a typed saga in `tests/e2e/006-sdk-multilanguage/us03.rs`.
- [ ] T006-019 [US03] Implement the domain path and error handling for writing a typed saga in `sdks/go/client/client.go`.
- [ ] T006-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T006-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T006-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/006-sdk-multilanguage/`.

## Phase 6: US04 — Migrate the SDK version (P1)
**Independent test:** Given a compatible previous minor version; When the worker connects to the updated server; Then protocol negotiation supports progressive upgrade.
- [ ] T006-023 [P] [US04] Write a Given/When/Then test for migrating the SDK version in `tests/e2e/006-sdk-multilanguage/us04.rs`.
- [ ] T006-024 [US04] Implement the domain path and error handling for migrating the SDK version in `sdks/python/sagat_octopus_orchestrator/client.py`.
- [ ] T006-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T006-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T006-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/006-sdk-multilanguage/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T006-028 [US01] Establish `ProtocolConformance` fixtures and version matrix in `tests/contract/sdk-conformance/`.
- [ ] T006-029 [US01] Implement DI, worker hosting, ActivitySource and CancellationToken in `sdks/dotnet/src/`.
- [ ] T006-030 [US01] Implement typed IR builders without side effects in the five languages in `sdks/*/`.
- [ ] T006-031 [US01] Test idempotency, decimal serialization, UTC, signal and stale token in all SDKs.

## Final Phase: Hardening / rollout
- [ ] T006-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/006-sdk-multilanguage/`.
- [ ] T006-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T006-034 Run the benchmark defined in `specs/006-sdk-multilanguage/plan.md` and publish the report in `tests/performance/reports/006-sdk-multilanguage.md`.
- [ ] T006-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/006-sdk-multilanguage/quickstart.md`.
- [ ] T006-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
