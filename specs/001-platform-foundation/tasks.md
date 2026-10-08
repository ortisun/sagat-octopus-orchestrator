# Tasks: Platform foundation and contracts

**Input:** `specs/001-platform-foundation/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T001-001 Check FR-001-001..FR-001-007, SCs and ADRs; record gaps in `specs/001-platform-foundation/research.md`.
- [ ] T001-002 [P] Define fixtures and contract validation in `tests/contract/001-platform-foundation/fixtures/`.
- [ ] T001-003 [P] Document types, transitions and invariants in `specs/001-platform-foundation/data-model.md`.

## Phase 2: Foundation
- [ ] T001-004 Create the necessary domain contracts/interfaces in `crates/domain/src/identity.rs` and invariant tests.
- [ ] T001-005 Implement ports/use cases in the appropriate module based on `specs/001-platform-foundation/plan.md`.
- [ ] T001-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T001-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/001-platform-foundation/`.

## Phase 3: US01 — Create a workspace and namespace (P1)
**Independent test:** Given an authenticated administrator; When they create a workspace and dev namespace; Then the workspace is persisted, and the scope appears in the API/Studio.
- [ ] T001-008 [P] [US01] Write a Given/When/Then test to create a workspace and namespace in `tests/e2e/001-platform-foundation/us01.rs`.
- [ ] T001-009 [US01] Implement the domain path and error handling to create a workspace and namespace in `crates/domain/src/identity.rs`.
- [ ] T001-010 [US01] Integrate adapters and persistence for the US01 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T001-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to the US01 scenario in `crates/telemetry/`.
- [ ] T001-012 [US01] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US01 in `tests/fault/001-platform-foundation/`.

## Phase 4: US02 — Authenticate an SDK client (P1)
**Independent test:** Given a valid token with tenant scope; When the client queries namespaces; Then the API returns only the authorized resources and a trace id.
- [ ] T001-013 [P] [US02] Write a Given/When/Then test to authenticate an SDK client in `tests/e2e/001-platform-foundation/us02.rs`.
- [ ] T001-014 [US02] Implement the domain path and error handling to authenticate an SDK client in `crates/application/src/identity.rs`.
- [ ] T001-015 [US02] Integrate adapters and persistence for the US02 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T001-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to the US02 scenario in `crates/telemetry/`.
- [ ] T001-017 [US02] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US02 in `tests/fault/001-platform-foundation/`.

## Phase 5: US03 — Bring up a complete local environment (P1)
**Independent test:** Given the cloned repository and prerequisites installed; When the developer starts docker compose and runs health checks; Then API, PostgreSQL and NATS are running and migrations are idempotent.
- [ ] T001-018 [P] [US03] Write a Given/When/Then test to bring up a complete local environment in `tests/e2e/001-platform-foundation/us03.rs`.
- [ ] T001-019 [US03] Implement the domain path and error handling to bring up a complete local environment in `crates/infra-postgres/src/identity.rs`.
- [ ] T001-020 [US03] Integrate adapters and persistence for the US03 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T001-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to the US03 scenario in `crates/telemetry/`.
- [ ] T001-022 [US03] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US03 in `tests/fault/001-platform-foundation/`.

## Phase 6: US04 — Deny cross-tenant access (P1)
**Independent test:** Given a user of tenant A; When they request a resource of tenant B; Then the API does not reveal tenant B data and records the attempt.
- [ ] T001-023 [P] [US04] Write a Given/When/Then test to deny cross-tenant access in `tests/e2e/001-platform-foundation/us04.rs`.
- [ ] T001-024 [US04] Implement the domain path and error handling to deny cross-tenant access in `apps/control-plane/src/main.rs`.
- [ ] T001-025 [US04] Integrate adapters and persistence for the US04 scenario in `crates/application/` and `crates/infra-postgres/`.
- [ ] T001-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to the US04 scenario in `crates/telemetry/`.
- [ ] T001-027 [US04] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US04 in `tests/fault/001-platform-foundation/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T001-028 [US01] Define the schema of `tenant` and `namespace` with scoped FKs in `crates/infra-postgres/migrations/` and reject tenant spoofing in tests.
- [ ] T001-029 [US01] Add OIDC/JWKS, authz and request context middleware in `crates/api-http/src/middleware/`.
- [ ] T001-030 [US01] Create CI for lint/clippy/fmt, migrations up and OpenAPI/protobuf validation in `.github/workflows/ci.yml`.

## Final Phase: Hardening / rollout
- [ ] T001-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/001-platform-foundation/`.
- [ ] T001-032 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T001-033 Run the benchmark defined in `specs/001-platform-foundation/plan.md` and publish the report in `tests/performance/reports/001-platform-foundation.md`.
- [ ] T001-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/001-platform-foundation/quickstart.md`.
- [ ] T001-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: none.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
