# Tasks: Script registry, code editor and inline execution

**Input:** `specs/005-inline-code-sandbox/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T005-001 Check FR-005-001..FR-005-008, SCs and ADRs; record gaps in `specs/005-inline-code-sandbox/research.md`. — gaps G96–G120 and decisions D95–D117 in `research.md`; the review of the implementation is D116 (35 findings, what was done about each).
- [x] T005-002 [P] Define fixtures and contract validation in `tests/contract/inline-code-sandbox/fixtures/`. — there is no `fixtures/` directory: the bodies of the requests are built inline in `tests/contract/inline-code-sandbox/rest/conformance.rs` and the contract is the OpenAPI file, which the suite drives and validates every response against.
- [x] T005-003 [P] Document types, transitions and invariants in `specs/005-inline-code-sandbox/data-model.md`. — `data-model.md` (tables, state machines, invariants, the artifact bundle, the build key).

## Phase 2: Foundation
- [x] T005-004 Create the necessary domain contracts/interfaces in `crates/script-registry/src/builds.rs` and invariant tests. — `crates/domain/src/scripts.rs` (ids, language, policy and its ceiling, limits, digest, failure codes, redaction) with its unit tests; there is no `crates/script-registry` (G109).
- [x] T005-005 Implement ports/use cases in the appropriate module based on `specs/005-inline-code-sandbox/plan.md`. — `crates/application/src/{scripts,sandbox,script_signing}.rs` (the author's service, the runner's service, the ports).
- [x] T005-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — `crates/infra-postgres/migrations/20261014000000_scripts.{up,down}.sql`: additive, reversible, checked with data present by `integration_inline_code_sandbox migration::*`.
- [x] T005-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/inline-code-sandbox/`. — `tests/integration/inline-code-sandbox/registry/` (41 tests; another tenant's script, version, test run and artifact are a `404` and the probe is audited).

## Phase 3: US01 — Edit and test a script in the browser (P1)
**Independent test:** Given an authorized author; When they write TS or Python code in Monaco and run a sandbox test; Then they receive filtered logs, result and inferred schema.
- [x] T005-008 [P] [US01] Write a Given/When/Then test for editing and testing a script in the browser in `tests/e2e/inline-code-sandbox/us01.rs`. — `tests/e2e/inline-code-sandbox/us01.rs` (10).
- [x] T005-009 [US01] Implement the domain path and error handling for editing and testing a script in the browser in `crates/script-registry/src/builds.rs`. — `crates/domain/src/scripts.rs`, `crates/application/src/scripts.rs`, `crates/infra-sandbox/src/docker.rs` and `apps/sandbox-runner` (not `crates/script-registry/src/builds.rs`, G109).
- [x] T005-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/scripts.rs`, `crates/infra-postgres/src/scripts.rs`, `crates/api-http/src/routes/scripts.rs`.
- [x] T005-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — `script_runs_total`, `script_start_seconds`, `script_run_seconds`, `script_memory_peak_bytes`, `script_denied_total`, `script_records_lost_total` on the runner; nothing carries a tenant, a script, a digest or a secret as a label; redaction of the log and the result (runbook §4).
- [x] T005-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/inline-code-sandbox/`. — `fault_inline_code_sandbox loss::*` (a test run whose runner dies is run again by another), `engine::*` (the engine down is a retryable failure of the platform), `e2e_inline_code_sandbox::us01` (a test run without secrets, the log filtered), `integration_inline_code_sandbox trials::*` (a completion accepted once).

## Phase 4: US02 — Publish an immutable artifact (P1)
**Independent test:** Given an approved script with a lockfile; When the author publishes a version; Then the artifact digest is signed/immutable and new Runs pin the version.
- [x] T005-013 [P] [US02] Write a Given/When/Then test for publishing an immutable artifact in `tests/e2e/inline-code-sandbox/us02.rs`. — `tests/e2e/inline-code-sandbox/us02.rs` (8).
- [x] T005-014 [US02] Implement the domain path and error handling for publishing an immutable artifact in `crates/script-registry/src/registry.rs`. — `crates/domain/src/scripts.rs`, `crates/application/src/{scripts,script_signing}.rs` (not `crates/script-registry/src/registry.rs`, G109).
- [x] T005-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/scripts.rs`, `crates/infra-postgres/src/scripts.rs` (triggers make versions and artifacts immutable).
- [x] T005-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — `script_builds_total{outcome}`, `script_build_seconds`, `script_bundles_refused_total`; the build log and the audit (`script.published`, `script.built`) carry no secret.
- [x] T005-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/inline-code-sandbox/`. — `fault_inline_code_sandbox loss::a_build_whose_runner_dies_is_taken_over…`, `integration_inline_code_sandbox builds::*` (a lease fenced by token hash, a late completion refused, a bundle that does not describe its version not signed, the wait before a build is offered again), `e2e_inline_code_sandbox::us02` (a forged signature and tampered bytes are not run).

## Phase 5: US03 — Run inline C# code (P1)
**Independent test:** Given a C# script with allowed NuGet references; When the author publishes; Then Roslyn compiles in a build worker and an isolated .NET host executes without a manual app deploy.
- [x] T005-018 [P] [US03] Write a Given/When/Then test for running inline C# code in `tests/e2e/inline-code-sandbox/us03.rs`. — A draft using the pinned Newtonsoft.Json package is tested, published, pinned by a workflow and executed by the real runner.
- [x] T005-019 [US03] Implement the domain path and error handling for running inline C# code in `crates/sandbox/src/policy.rs`. — `crates/domain/src/scripts.rs`, `crates/infra-sandbox/` and `runners/dotnet/`: Roslyn compiles in a build container and a distinct runtime image loads the signed v2 assembly bundle.
- [x] T005-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — The existing language-neutral build lease, artifact signing, persistence and step execution paths carry C# v2 artifacts; the E2E test crosses all of them.
- [x] T005-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — Existing runner metrics carry the `csharp` language label; container tests prove supervised host calls, secret redaction and direct-network denial without adding tenant or secret labels.
- [x] T005-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/inline-code-sandbox/`. — C# uses the same signed artifact/build-key, fenced lease and tenant-scoped queues; the fault suite takes over a lost C# test-run lease, and the container suite proves CPU timeout, cleanup and the runtime limits.

## Phase 6: US04 — Block a malicious script (P1)
**Independent test:** Given a script tries to access cloud metadata, the host, or exceed CPU; When the worker executes; Then egress and resources are denied, the failure result is audited.
- [x] T005-023 [P] [US04] Write a Given/When/Then test for blocking a malicious script in `tests/e2e/inline-code-sandbox/us04.rs`. — `tests/e2e/inline-code-sandbox/us04.rs` (8).
- [x] T005-024 [US04] Implement the domain path and error handling for blocking a malicious script in `crates/sandbox/src/supervisor.rs`. — `crates/infra-sandbox/src/docker.rs` (the supervisor of the container: the channel, the host calls, the limits) and `apps/sandbox-runner` (not `crates/sandbox/src/supervisor.rs`, G109).
- [x] T005-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/scripts.rs`, `crates/infra-postgres/src/scripts.rs` (`script.violation` audited once, with a cap).
- [x] T005-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — `script_denied_total{kind}`; the audit stores `fetch <scheme>://<host>` or `secret <name>`, never a path, a query, a credential or a value.
- [x] T005-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/inline-code-sandbox/`. — `integration_inline_code_sandbox_containers` (23, in real containers: no network, metadata service, engine socket, host files, fork bomb, memory bomb, CPU burn, disk and log floods, a script that does not read its replies), `e2e_inline_code_sandbox::us04` (8), `fault_inline_code_sandbox races::*` (a cancelled run stops its script and leaves no container).

## Phase: Engineering design details (story-linked hardening)

- [x] T005-028 [US01] Implement deterministic build digest source+deps+toolchain+policy in `crates/script-registry/src/builds.rs`. — the build key is the SHA-256 of the length-prefixed language, source, lockfile, toolchain, policy and schemas (`build_key` in `crates/application/src/scripts.rs`); `integration_inline_code_sandbox builds::an_identical_build_is_reused…`.
- [x] T005-029 [US01] Implement C# runner with Roslyn/NuGet in `runners/dotnet/` isolated per container. — Pinned .NET 10 SDK/runtime images, an offline exact Newtonsoft.Json 13.0.4 catalog, separate build/runtime hosts and a managed-assembly-only artifact v2; `integration_inline_code_sandbox_containers::csharp` runs the real images.
- [x] T005-030 [US01] Implement egress permissions and cgroups/seccomp in `crates/sandbox/src/policy.rs` and manifests in `infra/helm/`. — the container's network, read-only root, capabilities, seccomp, cgroups and user are in `crates/infra-sandbox/src/docker.rs` (`config`) and the egress policy in `apps/sandbox-runner/src/host.rs`; there is no `infra/helm` in the repository: the deployment is `infra/docker/compose.yaml` (profile `sandbox`) and `docs/runbooks/scripts.md` (G109).
- [x] T005-031 [US01] Add sandbox escapement/SSRF test in `tests/security/sandbox/`. — `tests/integration/inline-code-sandbox/sandbox/` (what a hostile script cannot do, in real containers) and `e2e_inline_code_sandbox::us04`; there is no `tests/security` target (G110).

## Final Phase: Hardening / rollout
- [x] T005-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/inline-code-sandbox/`. — `contract_rest_inline_code_sandbox` drives all 25 operations, normal and error paths, and validates each response against `openapi.yaml`; Redocly lint is valid with the 10 warnings of the baseline.
- [x] T005-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`. — `cargo clippy --workspace --all-targets -- -D warnings` clean; `scripts/check-architecture.py` passes (19 packages, no cycle, the domain imports no adapter); the control plane's manifest has no dependency on the container driver (checked by reading it; the script does not enforce that one).
- [x] T005-034 Run the benchmark defined in `specs/005-inline-code-sandbox/plan.md` and publish the report in `tests/performance/reports/inline-code-sandbox.md`. — `tests/performance/inline-code-sandbox/` and the report `tests/performance/reports/inline-code-sandbox.md` (commit f907b6f, release build, real containers and PostgreSQL): every hypothesis held (warm start p50 104 ms TypeScript and 327 ms Python, a build 1.3 s and 0.6 s, an identical build 46 ms with no container, a test run 1.8 s, four containers at once 3.5x one, a hostile script gone within 0.5 s of its limit). The machine was loaded (load average 9 at the start). The first two attempts failed in the log-flood scenario and found a bug in the Node shim (D118); the report says so.
- [x] T005-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/005-inline-code-sandbox/quickstart.md`. — two independent reviews, security and correctness, 35 findings, each checked against the code (D116); `docs/runbooks/scripts.md`, `docs/architecture/security-and-sandbox.md`, ADR-0013, `README.md`; evidence in `quickstart.md`.
- [x] T005-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — `docs/traceability.md` (the 005 section; C#, dependencies and the Studio editor are marked as not built).

## Dependencies & independent delivery

- This feature requires: 001, 002, 009.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.
