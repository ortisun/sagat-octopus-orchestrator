# Tasks: GitOps, versioning and definition promotion

**Input:** `specs/011-gitops-versioning/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T011-001 Check FR-011-001..FR-011-006, SCs and ADRs; record gaps in `specs/011-gitops-versioning/research.md`. — **evidence:** `research.md` D183–D205 and G193–G214
- [x] T011-002 [P] Define fixtures and contract validation in `tests/contract/gitops-versioning/fixtures/`. — **evidence:** `tests/contract/gitops-versioning/fixtures/manifests/` and the manifest and REST contract tests
- [x] T011-003 [P] Document types, transitions and invariants in `specs/011-gitops-versioning/data-model.md`. — **evidence:** `data-model.md`

## Phase 2: Foundation
- [x] T011-004 Create the necessary domain contracts/interfaces in `crates/domain/src/versioning.rs` and invariant tests. — **evidence:** `crates/domain/tests/versioning.rs` (11) and the permission matrix in `identity.rs`
- [x] T011-005 Implement ports/use cases in the appropriate module based on `specs/011-gitops-versioning/plan.md`. — **evidence:** `crates/application/src/publishing.rs`, `definitions.rs`
- [x] T011-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — **evidence:** `20261016000000_versioning` and `integration migration.rs` (back and forth with data)
- [x] T011-007 Create PostgreSQL integration tests and tenant isolation tests in `tests/integration/gitops-versioning/`. — **evidence:** `integration_gitops_versioning` (27, tenant isolation in `isolation.rs`)

## Phase 3: US01 — Publish immutable version (P1)
**Independent test:** Given a validated draft; When the author publishes workflow v3; Then the IR/deps digest is fixed and new runs can use v3.
- [x] T011-008 [P] [US01] Write Given/When/Then test for publishing an immutable version in `tests/e2e/gitops-versioning/us01.rs`. — **evidence:** `e2e us01` (2)
- [x] T011-009 [US01] Implement domain path and error handling for publishing an immutable version in `crates/domain/src/versioning.rs`. — **evidence:** `crates/domain/src/versioning.rs`, `crates/application/src/definitions.rs`
- [x] T011-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** `crates/application/src/publishing.rs`, `crates/infra-postgres/src/versioning.rs`, `activation.rs`
- [x] T011-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — **evidence:** `publication_*`, `version_activations_total`, `promotions_total`; `e2e telemetry.rs` (no one is named)
- [x] T011-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/gitops-versioning/`. — **evidence:** `fault history.rs`, `requests.rs`, `storm.rs`, `cli.rs`

## Phase 4: US02 — Keep old run (P1)
**Independent test:** Given run v2 is waiting for an event; When v3 is published; Then run v2 resumes with v2 semantics/artifacts.
- [x] T011-013 [P] [US02] Write Given/When/Then test for keeping the old run in `tests/e2e/gitops-versioning/us02.rs`. — **evidence:** `e2e us02`
- [x] T011-014 [US02] Implement domain path and error handling for keeping the old run in `crates/application/src/publishing.rs`. — **evidence:** `crates/application/src/publishing.rs` (runs are pinned; the pointer decides only where a new run starts)
- [x] T011-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** `integration activation.rs`
- [x] T011-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — **evidence:** as T011-011
- [x] T011-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/gitops-versioning/`. — **evidence:** `fault events.rs`, `storm.rs`

## Phase 5: US03 — Promote version across environments (P1)
**Independent test:** Given a manifest versioned in Git; When the pipeline promotes dev→stage→prod; Then history, approvals and dependencies remain traceable.
- [x] T011-018 [P] [US03] Write Given/When/Then test for promoting a version across environments in `tests/e2e/gitops-versioning/us03.rs`. — **evidence:** `e2e us03`, `e2e cli.rs`
- [x] T011-019 [US03] Implement domain path and error handling for promoting a version across environments in `crates/script-registry/src/registry.rs`. — **evidence:** done in `crates/application/src/publishing.rs`, not in a `script-registry` crate (G205)
- [x] T011-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** `integration promotion.rs` (7)
- [x] T011-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — **evidence:** as T011-011
- [x] T011-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/gitops-versioning/`. — **evidence:** `fault requests.rs`, `cli.rs` (cross-tenant from the command line)

## Phase 6: US04 — Perform safe rollback (P1)
**Independent test:** Given version v4 shows a regression; When the operator switches the active pointer to v3; Then new runs use v3 without rewriting v4's history.
- [x] T011-023 [P] [US04] Write Given/When/Then test for performing a safe rollback in `tests/e2e/gitops-versioning/us04.rs`. — **evidence:** `e2e us04`
- [x] T011-024 [US04] Implement domain path and error handling for performing a safe rollback in `crates/api-http/src/routes/versions.rs`. — **evidence:** done in `crates/api-http/src/routes/versioning.rs` (G205)
- [x] T011-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** `integration activation.rs`
- [x] T011-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — **evidence:** as T011-011
- [x] T011-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/gitops-versioning/`. — **evidence:** `fault history.rs`, `storm.rs`

## Phase: Engineering design details (story-linked hardening)

- [x] T011-028 [US01] Create canonical IR digest and immutable storage in `crates/application/src/publishing.rs`. — **evidence:** digest of the canonical document fixed at publication and version 1 byte for byte as it was: `e2e us01`, `us04`
- [x] T011-029 [US01] Implement promotion and rollback of the atomic active pointer in `crates/api-http/src/routes/versions.rs`. — **evidence:** `activate` and `promote` with the history: `integration activation.rs`, `promotion.rs`
- [x] T011-030 [US01] Create GitOps CLI diff/promote + approval records in `tools/cli/src/commands/publish.rs`. — **evidence:** done in `apps/cli` (G205): `apps/cli/tests` (14), `e2e` (11)

## Final Phase: Hardening / rollout
- [x] T011-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/gitops-versioning/`. — **evidence:** `contract_rest_gitops_versioning`, `contract_manifest_gitops_versioning`, the cross-tenant battery; no gRPC or AsyncAPI changes in this feature
- [x] T011-032 Apply lint and acyclic-dependency checks in `crates/` and product rules in `docs/architecture/`. — **evidence:** `cargo fmt --all -- --check`, workspace Clippy with `-D warnings`, architecture validation (20 crates, no cycles) and its 8 rule tests; see `quickstart.md` for the separate container-gate limitation.
- [x] T011-033 Run the benchmark defined in `specs/011-gitops-versioning/plan.md` and publish the report in `tests/performance/reports/gitops-versioning.md`. — **evidence:** `tests/performance/reports/gitops-versioning.md`
- [x] T011-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/011-gitops-versioning/quickstart.md`. — **evidence:** two independent reviews (security and correctness) with every finding's verdict and test, and a code and test review, in `quickstart.md`; the 011 suites; workspace run (1626 passed, the 63 failures are container suites); dependency audit, regenerated API types; committed-request recovery regression reproduced before the fix and passing afterwards.
- [x] T011-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — **evidence:** feature 011 evidence table and recovery regression; SC-011-003 remains partial because Docker-backed validation is unavailable locally.

## Dependencies & independent delivery

- This feature requires: 001, 002, 005, 007, 009.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
