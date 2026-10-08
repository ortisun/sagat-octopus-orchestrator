# Tasks: Script registry, code editor and inline execution

**Input:** `specs/005-inline-code-sandbox/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [ ] T005-001 Check FR-005-001..FR-005-008, SCs and ADRs; record gaps in `specs/005-inline-code-sandbox/research.md`.
- [ ] T005-002 [P] Define fixtures and contract validation in `tests/contract/005-inline-code-sandbox/fixtures/`.
- [ ] T005-003 [P] Document types, transitions and invariants in `specs/005-inline-code-sandbox/data-model.md`.

## Phase 2: Foundation
- [ ] T005-004 Create the necessary domain contracts/interfaces in `crates/script-registry/src/builds.rs` and invariant tests.
- [ ] T005-005 Implement ports/use cases in the appropriate module based on `specs/005-inline-code-sandbox/plan.md`.
- [ ] T005-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`.
- [ ] T005-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/005-inline-code-sandbox/`.

## Phase 3: US01 — Edit and test a script in the browser (P1)
**Independent test:** Given an authorized author; When they write TS or Python code in Monaco and run a sandbox test; Then they receive filtered logs, result and inferred schema.
- [ ] T005-008 [P] [US01] Write a Given/When/Then test for editing and testing a script in the browser in `tests/e2e/005-inline-code-sandbox/us01.rs`.
- [ ] T005-009 [US01] Implement the domain path and error handling for editing and testing a script in the browser in `crates/script-registry/src/builds.rs`.
- [ ] T005-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T005-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`.
- [ ] T005-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/005-inline-code-sandbox/`.

## Phase 4: US02 — Publish an immutable artifact (P1)
**Independent test:** Given an approved script with a lockfile; When the author publishes a version; Then the artifact digest is signed/immutable and new Runs pin the version.
- [ ] T005-013 [P] [US02] Write a Given/When/Then test for publishing an immutable artifact in `tests/e2e/005-inline-code-sandbox/us02.rs`.
- [ ] T005-014 [US02] Implement the domain path and error handling for publishing an immutable artifact in `crates/script-registry/src/registry.rs`.
- [ ] T005-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T005-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`.
- [ ] T005-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/005-inline-code-sandbox/`.

## Phase 5: US03 — Run inline C# code (P1)
**Independent test:** Given a C# script with allowed NuGet references; When the author publishes; Then Roslyn compiles in a build worker and an isolated .NET host executes without a manual app deploy.
- [ ] T005-018 [P] [US03] Write a Given/When/Then test for running inline C# code in `tests/e2e/005-inline-code-sandbox/us03.rs`.
- [ ] T005-019 [US03] Implement the domain path and error handling for running inline C# code in `crates/sandbox/src/policy.rs`.
- [ ] T005-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T005-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`.
- [ ] T005-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/005-inline-code-sandbox/`.

## Phase 6: US04 — Block a malicious script (P1)
**Independent test:** Given a script tries to access cloud metadata, the host, or exceed CPU; When the worker executes; Then egress and resources are denied, the failure result is audited.
- [ ] T005-023 [P] [US04] Write a Given/When/Then test for blocking a malicious script in `tests/e2e/005-inline-code-sandbox/us04.rs`.
- [ ] T005-024 [US04] Implement the domain path and error handling for blocking a malicious script in `crates/sandbox/src/supervisor.rs`.
- [ ] T005-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`.
- [ ] T005-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`.
- [ ] T005-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/005-inline-code-sandbox/`.

## Phase: Engineering design details (story-linked hardening)

- [ ] T005-028 [US01] Implement deterministic build digest source+deps+toolchain+policy in `crates/script-registry/src/builds.rs`.
- [ ] T005-029 [US01] Implement C# runner with Roslyn/NuGet in `runners/dotnet/` isolated per container.
- [ ] T005-030 [US01] Implement egress permissions and cgroups/seccomp in `crates/sandbox/src/policy.rs` and manifests in `infra/helm/`.
- [ ] T005-031 [US01] Add sandbox escapement/SSRF test in `tests/security/sandbox/`.

## Final Phase: Hardening / rollout
- [ ] T005-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/005-inline-code-sandbox/`.
- [ ] T005-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`.
- [ ] T005-034 Run the benchmark defined in `specs/005-inline-code-sandbox/plan.md` and publish the report in `tests/performance/reports/005-inline-code-sandbox.md`.
- [ ] T005-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/005-inline-code-sandbox/quickstart.md`.
- [ ] T005-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`.

## Dependencies & independent delivery

- This feature requires: 001, 002, 009.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
