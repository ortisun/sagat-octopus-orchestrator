# Tasks: Security, isolation and multi-tenant governance

**Input:** `specs/009-security-multitenancy/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T009-001 Check FR-009-001..FR-009-008, SCs and ADRs; record gaps in `specs/009-security-multitenancy/research.md`.
- [ ] T009-002 [P] Define fixtures and contract validation in `tests/contract/009-security-multitenancy/fixtures/`.
- [ ] T009-003 [P] Document types, transitions and invariants in `specs/009-security-multitenancy/data-model.md`.

## Phase 2: Foundation
- [ ] T009-004 Create the necessary domain contracts/interfaces in `crates/domain/src/permissions.rs` and invariant tests.
- [ ] T009-005 Implement ports/use cases in the appropriate module based on `specs/009-security-multitenancy/plan.md`.
- [ ] T009-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T009-007 Create PostgreSQL integration and per-tenant isolation tests in `tests/integration/009-security-multitenancy/`.

## Phase 3: US01 — Prevent cross-tenant access (P1)
**Independent test:** Given tenant A with a valid token; When it tries to access tenant B's Run; Then reading, manipulation and discovery of existence are denied.
- [ ] T009-008 [P] [US01] Write a Given/When/Then test for preventing cross-tenant access in `tests/e2e/009-security-multitenancy/us01.rs`.
- [ ] T009-009 [US01] Implement domain path and error handling for preventing cross-tenant access in `crates/domain/src/permissions.rs`.
- [ ] T009-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T009-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T009-012 [US01] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US01 in `tests/fault/009-security-multitenancy/`.

## Phase 4: US02 — Grant a capability to a script (P1)
**Independent test:** Given an author with access to a staging secret; When they publish a Script; Then the worker receives only a temporary capability and authorized secrets.
- [ ] T009-013 [P] [US02] Write a Given/When/Then test for granting a capability to a script in `tests/e2e/009-security-multitenancy/us02.rs`.
- [ ] T009-014 [US02] Implement domain path and error handling for granting a capability to a script in `crates/application/src/authz.rs`.
- [ ] T009-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T009-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T009-017 [US02] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US02 in `tests/fault/009-security-multitenancy/`.

## Phase 5: US03 — Block unauthorized egress (P1)
**Independent test:** Given a script tries to access a forbidden endpoint; When the sandbox executes; Then the network is blocked and the incident is audited.
- [ ] T009-018 [P] [US03] Write a Given/When/Then test for blocking unauthorized egress in `tests/e2e/009-security-multitenancy/us03.rs`.
- [ ] T009-019 [US03] Implement domain path and error handling for blocking unauthorized egress in `crates/infra-postgres/src/tenant_scopes.rs`.
- [ ] T009-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T009-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T009-022 [US03] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US03 in `tests/fault/009-security-multitenancy/`.

## Phase 6: US04 — Enforce quotas per namespace (P1)
**Independent test:** Given a namespace exhausts its limit of builds and tasks; When new requests arrive; Then they return quota exceeded without affecting other tenants.
- [ ] T009-023 [P] [US04] Write a Given/When/Then test for enforcing quotas per namespace in `tests/e2e/009-security-multitenancy/us04.rs`.
- [ ] T009-024 [US04] Implement domain path and error handling for enforcing quotas per namespace in `crates/sandbox/src/policy.rs`.
- [ ] T009-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T009-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T009-027 [US04] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US04 in `tests/fault/009-security-multitenancy/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T009-028 [US01] Apply query scope `(tenant_id,namespace_id)` in `crates/infra-postgres/` and cross-tenant data tests.
- [ ] T009-029 [US01] Apply capability token, secret store and resource scopes policies in `crates/application/src/authz.rs`.
- [ ] T009-030 [US01] Implement multi-replica quota enforcement with atomic operations in `crates/application/src/quotas.rs`.
- [ ] T009-031 [US01] Enforce NetworkPolicy/pod security in Helm `infra/helm/templates/`.

## Final Phase: Hardening / rollout
- [ ] T009-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/009-security-multitenancy/`.
- [ ] T009-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T009-034 Run the benchmark defined in `specs/009-security-multitenancy/plan.md` and publish the report in `tests/performance/reports/009-security-multitenancy.md`.
- [ ] T009-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/009-security-multitenancy/quickstart.md`.
- [ ] T009-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
