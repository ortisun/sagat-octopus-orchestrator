# Tasks: Security, isolation and multi-tenant governance

**Input:** `specs/009-security-multitenancy/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T009-001 Check FR-009-001..FR-009-008, SCs and ADRs; record gaps in `specs/009-security-multitenancy/research.md`. — gaps G35–G50 and decisions D42–D58 in `research.md`; the security review of the implementation is in `quickstart.md`.
- [x] T009-002 [P] Define fixtures and contract validation in `tests/contract/security-multitenancy/fixtures/`. — there is no `fixtures/` directory: valid bodies for every operation are built from the OpenAPI document by `tests/support/contract.rs` and validated against its schemas in `rest/conformance.rs` and `rest/battery.rs`.
- [x] T009-003 [P] Document types, transitions and invariants in `specs/009-security-multitenancy/data-model.md`. — `data-model.md` (tables, invariants, lock order, migrations).

## Phase 2: Foundation
- [x] T009-004 Create the necessary domain contracts/interfaces in `crates/domain/src/permissions.rs` and invariant tests. — the domain contracts live in `crates/domain/src/{identity,binding,quota,secrets,egress,audit}.rs` (the permissions are in `identity.rs`, from feature 001; there is no `permissions.rs`), with unit tests in each module and `crates/domain/tests/workflow.rs`.
- [x] T009-005 Implement ports/use cases in the appropriate module based on `specs/009-security-multitenancy/plan.md`. — `crates/application/src/{authz,quotas,bindings,secrets,egress}.rs` and the worker service.
- [x] T009-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — six additive migrations `20261010000000`…`20261010000500`, each with a `down`, checked by the rollback test of feature 001.
- [x] T009-007 Create PostgreSQL integration and per-tenant isolation tests in `tests/integration/security-multitenancy/`. — `tests/integration/security-multitenancy/{quotas,bindings,secrets,egress}.rs`.

## Phase 3: US01 — Prevent cross-tenant access (P1)
**Independent test:** Given tenant A with a valid token; When it tries to access tenant B's Run; Then reading, manipulation and discovery of existence are denied.
- [x] T009-008 [P] [US01] Write a Given/When/Then test for preventing cross-tenant access in `tests/e2e/security-multitenancy/us01.rs`. — `us01.rs` (worker protocol, leaked lease token) and `contract/.../rest/battery.rs` (every REST operation).
- [x] T009-009 [US01] Implement domain path and error handling for preventing cross-tenant access in `crates/domain/src/permissions.rs`. — in `crates/application/src/authz.rs` and `crates/domain/src/identity.rs` (there is no `permissions.rs`): another tenant's resource is an indistinguishable not-found, audited.
- [x] T009-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/` and `crates/infra-postgres/` (composite foreign keys, tenant-scoped queries).
- [x] T009-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — the counters are emitted where the decision is taken (`crates/application/src/worker.rs`, `crates/infra-postgres/src/identity.rs`, `crates/api-http/src/error.rs`), through the `metrics` facade that `crates/telemetry` installs; `e2e/.../telemetry.rs` checks they move and that no tenant, subject, run or secret is named.
- [x] T009-012 [US01] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US01 in `tests/fault/security-multitenancy/`. — `fault_security_multitenancy` (connections killed around quota, secret and binding transactions), `secrets::asking_again_within_an_attempt...` (duplicates), `secrets::the_capability_ends_with_the_lease...` (timeout), the battery (tenant boundary).

## Phase 4: US02 — Grant a capability to a script (P1)
**Independent test:** Given an author with access to a staging secret; When they publish a Script; Then the worker receives only a temporary capability and authorized secrets.
- [x] T009-013 [P] [US02] Write a Given/When/Then test for granting a capability to a script in `tests/e2e/security-multitenancy/us02.rs`. — `us02.rs`.
- [x] T009-014 [US02] Implement domain path and error handling for granting a capability to a script in `crates/application/src/authz.rs`. — `crates/application/src/{secrets,authz,worker}.rs`.
- [x] T009-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/` and `crates/infra-postgres/src/secrets.rs`.
- [x] T009-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — `secrets_handed_out_total`, `secret_resolution_denied_total`, `secret_output_detected_total`; values never in a log, row or response (`us02.rs`).
- [x] T009-017 [US02] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US02 in `tests/fault/security-multitenancy/`. — `fault_security_multitenancy` (secret writes and resolutions), the same duplicate and timeout tests as T009-012, `secrets::the_secrets_of_one_tenant_are_invisible_and_unchangeable_by_another`.

## Phase 5: US03 — Block unauthorized egress (P1)
**Independent test:** Given a script tries to access a forbidden endpoint; When the sandbox executes; Then the network is blocked and the incident is audited.
- [x] T009-018 [P] [US03] Write a Given/When/Then test for blocking unauthorized egress in `tests/e2e/security-multitenancy/us03.rs`. — Feature 005 is integrated: `us03.rs` executes JavaScript and Python in the production Docker sandbox, proves both direct and mediated network paths cannot reach a loopback canary, and observes one redacted `FORBIDDEN_ADDRESS` incident per attempt.
- [x] T009-019 [US03] Implement domain path and error handling for blocking unauthorized egress in `crates/infra-postgres/src/tenant_scopes.rs`. — `crates/domain/src/egress.rs`, `crates/infra-egress/`, `crates/infra-postgres/src/egress.rs` (there is no `tenant_scopes.rs`).
- [x] T009-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/egress.rs`, `crates/infra-postgres/src/egress.rs`.
- [x] T009-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — `security_denials_total{action="egress.denied"}` counts every refusal, also the repeats the audit trail collapses (`egress::a_refused_request_is_audited...`).
- [x] T009-022 [US03] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US03 in `tests/fault/security-multitenancy/`. — `egress.rs` runs six real-sandbox cases: repeated refusal, denial before timeout, runner loss/retry, cross-tenant policy isolation, live revocation and denial committed before lease loss. The durable incident endpoint is fenced and idempotent; an uncommitted incident fails closed instead of acknowledging a denial (D59).

## Phase 6: US04 — Enforce quotas per namespace (P1)
**Independent test:** Given a namespace exhausts its limit of builds and tasks; When new requests arrive; Then they return quota exceeded without affecting other tenants.
- [x] T009-023 [P] [US04] Write a Given/When/Then test for enforcing quotas per namespace in `tests/e2e/security-multitenancy/us04.rs`. — `us04.rs` (the resource is runs; builds and tasks as resources are G37).
- [x] T009-024 [US04] Implement domain path and error handling for enforcing quotas per namespace in `crates/sandbox/src/policy.rs`. — `crates/domain/src/quota.rs` and `crates/application/src/quotas.rs` (there is no `crates/sandbox`).
- [x] T009-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/` and `crates/infra-postgres/src/quotas.rs`.
- [x] T009-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — `quota_exceeded_total{limit}`.
- [x] T009-027 [US04] Run and document tests for duplicates, timeout, worker failure and tenant boundary for US04 in `tests/fault/security-multitenancy/`. — `fault_security_multitenancy` (kills around quota-checked starts), `quotas::replicas_racing_for_the_last_places_never_overshoot_the_quota`, `quotas::the_quota_of_one_tenant_never_affects_another`.

## Phase: Engineering design details (story-linked hardening)

- [x] T009-028 [US01] Apply query scope `(tenant_id,namespace_id)` in `crates/infra-postgres/` and cross-tenant data tests. — composite `(tenant_id, …)` foreign keys on every table; the battery; `*_of_one_tenant_are_invisible_and_unchangeable_by_another` in each integration suite.
- [x] T009-029 [US01] Apply capability token, secret store and resource scopes policies in `crates/application/src/authz.rs`. — `crates/application/src/authz.rs`, the secret store, and the lease as the capability for a secret.
- [x] T009-030 [US01] Implement multi-replica quota enforcement with atomic operations in `crates/application/src/quotas.rs`. — `quotas::replicas_racing...`, `fault_security_multitenancy`; exact count under an advisory lock (D42).
- [x] T009-031 [US01] Enforce NetworkPolicy/pod security in Helm `infra/helm/templates/`. — The integrated Feature 010 chart now carries required, validated ingress/egress values and explicit HA examples. `contract_scalability_ha` (16/16) checks exact control-plane, engine and per-pool peers/ports, release-scoped worker ingress, optional NATS, unsafe-value refusals and policy disablement. `tests/security/sandbox/kubernetes-network-policy.sh` passed against the integrated `infra/helm` chart on real Cilium: restricted derived PodSpecs, positive DNS/dependency/internal API TCP paths and negative Kubernetes API, namespace, role and pool-isolation paths (G45).

## Final Phase: Hardening / rollout
- [x] T009-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/security-multitenancy/`. — `contract_rest_security_multitenancy` (5 tests: every governance operation against the OpenAPI, the cross-tenant battery, no secret value in any response, the 401 sweep), `contract_grpc_security_multitenancy` (TLS, mutual TLS and the connection limits over real sockets); OpenAPI lint at its 10-warning baseline and AsyncAPI validation (this feature adds no event).
- [x] T009-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`. — fmt, clippy `-D warnings`, `scripts/check-architecture.py` (15 crates, layers respected, no cycles), `cargo audit --deny warnings`.
- [x] T009-034 Run the benchmark defined in `specs/009-security-multitenancy/plan.md` and publish the report in `tests/performance/reports/security-multitenancy.md`. — `tests/performance/reports/security-multitenancy.md`.
- [x] T009-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/009-security-multitenancy/quickstart.md`. — the review and its finding-by-finding table are in `quickstart.md`; runbook `docs/runbooks/security-governance.md`; README.
- [x] T009-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — `docs/traceability.md` (evidence for 009).

## Dependencies & independent delivery

- This feature requires: 001, 002.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.

## FR-009-006 durable quota completion (ADR-0027)

- [ ] T009-037 Define resource-specific byte/rate/retention units and ranges, authoritative policy fences, durable usage/bucket state, and the exhaustive variable-column classification manifest; implement additive migration/backfill and schema coverage tests in `crates/{domain,application,infra-postgres}` and `tests/integration/security-multitenancy/`.
- [ ] T009-038 Enforce namespace/source event rates inside `infra-postgres/events.rs::admit_within` after dedupe, with atomic no-charge rollback and connector no-ack/checkpoint semantics; enforce event/script retention ceilings at authoritative new admission without rewriting stored expiry; update REST/CLI contracts and race/minimum-window tests.
- [ ] T009-039 Wire every classified content/log mutation and purge, including runtime payloads/child input, definitions/publications, script source/artifacts/SBOM/results/logs, events/dead letters, secrets/configuration and blob reservations; prove exact accounting, late-PUT safety, quota-induced fenced terminal failures and log truncation without blocking state/journal/audit/outbox.
- [ ] T009-040 Add shared durable API-rate admission after authenticated canonical namespace authorization, inventory every applicable API use case, and prove two-server shared buckets plus cross-tenant non-debit; complete fixed-label telemetry, governance runbook, measured quota benchmark and FR-009-006 evidence. Product tier/regulatory decisions G42 remain separately identified.
