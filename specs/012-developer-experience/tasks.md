# Tasks: CLI, quickstarts, local testing and documentation

**Input:** `specs/012-developer-experience/spec.md`, `plan.md`, `research.md`, `data-model.md`, 001 contracts.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T012-001 Check FR-012-001..FR-012-006, SCs and ADRs; record gaps in `specs/012-developer-experience/research.md`. — **evidence:** `research.md` D206–D228 and G215–G237
- [x] T012-002 [P] Define fixtures and contract validation in `tests/contract/developer-experience/fixtures/`. — **evidence:** the fixtures are the six templates and the examples: `apps/cli/tests/templates.rs` (each is a document the codec accepts, with answers and a README), `contract_developer_experience` (the documents hold the commands and excerpts they show); there is no `fixtures/` directory
- [x] T012-003 [P] Document types, transitions and invariants in `specs/012-developer-experience/data-model.md`. — **evidence:** `data-model.md` (no table: the files the tool reads and writes, the mock file, the provider)

## Phase 2: Foundation
- [x] T012-004 Create the necessary domain contracts/interfaces in `tools/cli/src/main.rs` and invariant tests. — **evidence:** the tool is `apps/cli` (`sagat`, the name of feature 011), not `tools/cli` (D206): `apps/cli/src/{args,lib,client,dev,worker,mocks,…}.rs`; invariants in `apps/cli/tests` (50)
- [x] T012-005 Implement ports/use cases in the appropriate module based on `specs/012-developer-experience/plan.md`. — **evidence:** `apps/cli` is a client of the API; the server-side parts are `GET /v1/version` (`crates/api-http`) and `seed_dev_namespace` (`apps/control-plane`); `apps/dev-idp` is the provider
- [x] T012-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — **evidence:** none needed: the feature adds no table or column (`data-model.md`); the migration suites are unchanged
- [x] T012-007 Create PostgreSQL integration tests and tenant isolation tests in `tests/integration/developer-experience/`. — **evidence:** no new tables to isolate; tenant isolation of what the feature does: `fault another_tenant_cannot_see_run_bind_or_serve_what_is_not_its_own`; the seed against PostgreSQL: `e2e dev_environment` (idempotent, refused outside dev, two at once)

## Phase 3: US01 — Start first workflow quickly (P1)
**Independent test:** Given a developer follows the quickstart; When they run compose + CLI + SDK; Then a two-step workflow executes and is visible in Studio.
- [x] T012-008 [P] [US01] Write Given/When/Then test for starting the first workflow quickly in `tests/e2e/developer-experience/us01.rs`. — **evidence:** `e2e stories::us01_*` (2) — in `stories.rs`, not `us01.rs`
- [x] T012-009 [US01] Implement domain path and error handling for starting the first workflow quickly in `tools/cli/src/main.rs`. — **evidence:** `apps/cli/src/{new,validate,workflow,worker,doctor}.rs`
- [x] T012-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** `GET /v1/version` (`crates/api-http/src/routes/identity.rs`, OpenAPI `getPlatformVersion`, the REST conformance suite) and `seed_dev_namespace`; no persistence of its own
- [x] T012-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — **evidence:** scope: **no new metric** (the tool is a client and sends no telemetry, G238); the run it starts has a trace the tool shows and `e2e us03` holds every event of every language's run to it; the provider logs who a token was for and never the token (`apps/dev-idp/src/server.rs`); the tool never prints the token and filters what it prints (`fault no_command_prints_the_token_…`, `apps/cli/tests/terminal.rs`)
- [x] T012-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/developer-experience/`. — **evidence:** `fault_developer_experience` (14): `a_worker_that_stops_and_starts_again_…`, `a_run_waited_for_in_vain_…`, `the_same_request_twice_…`, `another_tenant_…`, `a_worker_told_to_stop_…`

## Phase 4: US02 — Test Saga locally (P1)
**Independent test:** Given a developer has a payment saga; When they inject a failure into the mock provider; Then they see reconciliation and compensation in the development emulator.
- [x] T012-013 [P] [US02] Write Given/When/Then test for testing a saga locally in `tests/e2e/developer-experience/us02.rs`. — **evidence:** `e2e stories::us02_*` (2) and `e2e saga` — in `stories.rs` and `saga.rs`, not `us02.rs`
- [x] T012-014 [US02] Implement domain path and error handling for testing a saga locally in `examples/payment-saga/`. — **evidence:** `examples/payment-saga/typescript`, `templates/compensation`
- [x] T012-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** none needed beyond T012-010: the saga runs on the engine as it is
- [x] T012-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — **evidence:** as T012-011
- [x] T012-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/developer-experience/`. — **evidence:** `fault a_run_that_waits_for_a_person_is_reported_at_once_…`, `a_worker_with_nothing_to_do_…` (`NO_MOCK` end to end), `an_answer_in_the_wrong_section_…`; `e2e stories::us02_a_lost_answer_…` (the lease runs out and the platform reconciles)

## Phase 5: US03 — Generate examples per SDK (P1)
**Independent test:** Given a new developer uses Go or C#; When they copy the example and run it; Then the example produces a contract/trace consistent with the other SDKs.
- [x] T012-018 [P] [US03] Write Given/When/Then test for generating examples per SDK in `tests/e2e/developer-experience/us03.rs`. — **evidence:** `e2e us03` (ignored: needs five toolchains; the `sdks` job of CI runs it)
- [x] T012-019 [US03] Implement domain path and error handling for generating examples per SDK in `examples/hello-world/`. — **evidence:** `examples/hello-world/{typescript,go,python,dotnet,java}`
- [x] T012-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** none needed: the examples use only the public API of the SDKs
- [x] T012-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — **evidence:** as T012-011
- [x] T012-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/developer-experience/`. — **evidence:** `fault` suite; and in `e2e us03`: a fresh token for each language, exit 2 without a token, no token in any output

## Phase 6: US04 — Diagnose publishing errors (P2)
**Independent test:** Given a manifest has an invalid node; When the CLI validate runs; Then they receive errors with path, remediation and docs links.
- [x] T012-023 [P] [US04] Write Given/When/Then test for diagnosing publishing errors in `tests/e2e/developer-experience/us04.rs`. — **evidence:** `e2e diagnostics.rs` (4) — `us04_*`
- [x] T012-024 [US04] Implement domain path and error handling for diagnosing publishing errors in `docs/quickstarts/`. — **evidence:** `docs/quickstarts/errors.md` and `apps/cli/src/diagnostics.rs`, held to each other by `apps/cli/tests/diagnostics.rs`
- [x] T012-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — **evidence:** none needed: the findings are the codec's, the tool adds the help
- [x] T012-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — **evidence:** as T012-011
- [x] T012-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/developer-experience/`. — **evidence:** `fault a_workflow_from_somewhere_else_…`, `the_token_is_not_sent_to_a_worker_endpoint_in_the_clear`, `e2e diagnostics.rs`

## Phase: Engineering design details (story-linked hardening)

- [x] T012-028 [US01] Create Rust CLI `df workflow validate|publish|run|inspect` in `tools/cli/src/`. — **evidence:** `apps/cli` (`sagat workflow validate|publish|run|runs|inspect|cancel`, `script test|publish`, `worker dev|bind`, `dev token`, `doctor`, `new`)
- [x] T012-029 [US01] Write TS/Go/Python/C#/Java quickstarts with mocks and test harness in `docs/quickstarts/`. — **evidence:** `docs/quickstarts/{README,typescript,go,python,dotnet,java,saga}.md`; the mocks are `worker dev`'s (D208) and the harness is the example (`e2e us03`); **no SDK ships a fake context for unit tests (G219)**
- [x] T012-030 [US01] Implement self-check of environment, broker and protocol version in `tools/cli/src/commands/doctor.rs`. — **evidence:** `apps/cli/src/doctor.rs`: `e2e doctor.rs` (7), `GET /v1/version`

## Final Phase: Hardening / rollout
- [x] T012-031 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/developer-experience/`. — **evidence:** `contract_developer_experience` (7) and `contract_rest_platform_foundation` (the `/version` scenarios); the directory is `tests/contract/developer-experience/`
- [x] T012-032 Apply lint and acyclic-dependency checks in `crates/` and product rules in `docs/architecture/`. — **evidence:** `cargo clippy --workspace --all-targets --locked -- -D warnings` clean; `scripts/check-architecture.py` passes (21 crates, no cycles)
- [x] T012-033 Run the benchmark defined in `specs/012-developer-experience/plan.md` and publish the report in `tests/performance/reports/developer-experience.md`. — **evidence:** `tests/performance/reports/developer-experience.md` — **the machine's part of the first day, not an evaluation with people (G215, G227)**
- [x] T012-034 Review security, redaction, RBAC, README and runbook; record evidence in `specs/012-developer-experience/quickstart.md`. — **evidence:** `quickstart.md` (two independent reviews, their findings and what was fixed); `docs/runbooks/developer-experience.md`; README
- [x] T012-035 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — **evidence:** `docs/traceability.md` section 012 (SC-012-001 is **not** proven: G215)

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004, 005, 006, 007.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
