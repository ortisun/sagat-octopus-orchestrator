# Tasks — 028 developer tooling

**Input:** [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md), [data-model.md](data-model.md),
[contracts/](contracts/). **Status:** proposed; nothing is done. Format: `TNNN-xxx [P?] [US?] description — FRs —
evidence`. `[P]` = parallel with other `[P]` tasks of the same phase (different files). A task is ticked only with the
named evidence (a test and its result, a command and its output); a partly done task stays unticked with a dated note.

## Phase 1 — Setup

- [ ] T028-001 Record the owner's review of ADR-0063 and answers to O1400–O1402; coordinator integrates `docs/roadmap.md`, `docs/traceability.md`, `docs/owner-decisions.md`, `docs/adr/README.md`, `SPECIFICATION-INDEX.md`. — all — evidence: accepted ADR status line and owner-decision rows.
- [ ] T028-002 [P] Scaffold `apps/test-server`, `crates/infra-testing`, `crates/infra-git`; add to the workspace; extend `scripts/check-architecture.py` (only `apps/test-server` and `tests` may depend on `sagat-infra-testing`; only `tests` on `sagat-test-server`) with negative cases in `scripts/tests/test_check_architecture.py`; add `scripts/check-production-image.py` (CI job) that fails when the `cargo build -p` list or a `COPY --from=builder` line of `infra/docker/Dockerfile` names the test server, or when `cargo tree -p sagat-control-plane -p sagat-execution-engine` contains `sagat-infra-testing`. — FR-028-001, 002 — evidence: both checks pass on the tree and fail on each negative fixture.
- [ ] T028-003 [P] Add `gitsync:manage`, `gitsync:read` and the reserved actor `system:git-sync` in `crates/domain/src/identity.rs` with the role grants of plan.md; domain tests and the `AuthContext` contract case. `testing:clock` is **not** added here (T028-017). — FR-028-013, 030 — evidence: `cargo test -p sagat-domain identity`; a test that no production role lists `testing:clock`.
- [ ] T028-004 [P] Create `tests/{integration,fault,contract,e2e}/developer-tooling/` and register `integration_developer_tooling`, `fault_developer_tooling`, `contract_developer_tooling`, `e2e_developer_tooling` in `tests/Cargo.toml`. — FR-028-045 — evidence: empty targets compile.
- [ ] T028-005 Merge the contract drafts ([contracts/README.md](contracts/README.md)): new `specs/001-platform-foundation/contracts/testing.openapi.yaml`; Git sync and catalog paths, `PlatformVersion.capabilities` and alert kinds in `openapi.yaml`; alert kinds in `events.asyncapi.yaml` with the baseline regenerated on purpose; `sdks/error-catalog.json` v2 and its five copies; `template-index.schema.json`; `npm run api` in `frontend/apps/studio`; regenerate `docs/reference/api.md`. — FR-028-045 — evidence: `contract_platform_foundation`, `contract_asyncapi_events_webhooks_messaging`, `npm run check:api`, `contract_developer_experience`.

## Phase 2 — Foundational: two time classes (blocks every story)

- [ ] T028-006 Write `crates/infra-postgres/time-classification.json` (every time column, class, writers, readers) and a catalog test in `tests/integration/developer-tooling/time_classification.rs` that fails on an unclassified or stale entry, on a workflow-class column whose default is a built-in clock, and on a `pg_proc` body that compares a workflow-class column with a built-in clock without a listed reason; carry the cross-feature rows of [contracts/time-classes.md](contracts/time-classes.md) (decision C13) and merge its rule into `docs/architecture/execution-semantics.md`. — FR-028-003 — evidence: test passes; a mutated manifest, default or function fails it.
- [ ] T028-007 Migration with `sagat_workflow_from(ts timestamptz)` (`STABLE`, body `SELECT $1`), canonical comment, owned by the migration role, `EXECUTE` to `sagat_app`, a refusing `down`; runtime-role privilege audit (the runtime role cannot replace it); a test that the workflow and infrastructure instants of one post-lock sample are equal at offset zero. — FR-028-003 — evidence: migration suite, `runtime_privileges`, equality test.
- [ ] T028-008 `crates/domain/src/time.rs`: `WorkflowInstant`, `InfrastructureInstant`; move timer, deadline, retry, approval, loop and chain inputs of the reducer to `WorkflowInstant` and leases to `InfrastructureInstant`; compile-fail doc tests for a mixed comparison. — FR-028-003 — evidence: `cargo test -p sagat-domain`, doc tests.
- [ ] T028-009 Store refactor of every file of `crates/infra-postgres/src` that samples a clock at the baseline — `approval_progress`, `approval_scheduler`, `bindings`, `blobs`, `breaker`, `capacity_finish`, `chain_progress`, `connectors`, `content_usage`, `database`, `database/admission`, `dispatch`, `egress`, `events`, `execution`, `execution/database`, `execution/path`, `leaf_certification`, `loop_progress`, `maintenance`, `managed_secrets`, `operations`, `outbound`, `partitioning`, `primitives`, `quotas`, `scoped_leaf_finish`, `scripts/incident`, `secret_cleanup`, `secrets`, `telemetry`, `workers` (each classified, most staying on infrastructure time) — plus the readers of workflow columns (`scope_frontier`): one materialized post-lock sample with the workflow instant derived by `sagat_workflow_from`; workflow-due predicates bounded by `sagat_workflow_from(now())`; scheduler lag over workflow columns (`operations.rs`) and the telemetry purge over `runs.finished_at` (`telemetry.rs`) on workflow now; manifest-driven lint test; extend the ADR-0020 pass-file test to refuse a bare `now()` against `workflow_due` columns. — FR-028-003 — evidence: lint test, pass-file test.
- [ ] T028-010 Plan equality: `EXPLAIN (FORMAT JSON)` of every scheduler pass query before and after T028-009 on a seeded database, reviewed goldens in `tests/integration/developer-tooling/plans/`. — NFR-028-COMP, SC-028-003 — evidence: plan test passes.
- [ ] T028-011 `WorkflowClock` port beside `Clock` in `crates/application/src/clock.rs`; classify each `SystemClock` user. — FR-028-003 — evidence: `cargo test -p sagat-application`.
- [ ] T028-012 Zero-offset regression: the durable-execution, sagas, events and workflow-primitives integration and fault suites unchanged after the refactor. — FR-028-008 — evidence: suite results recorded in the PR.

**Checkpoint:** production behaviour proven unchanged; stories may start.

## Phase 3 — US01 Time-skipping test environment (P1)

- [ ] T028-013 [US01] `crates/domain/src/testing.rs`: clock rules (monotonic, ceilings, epochs), stop selection, quiescence verdict, skip outcomes; unit and property tests. — FR-028-004..007 — evidence: `cargo test -p sagat-domain testing`.
- [ ] T028-014 [US01] `crates/infra-testing`: test migration (marker, `sagat_test_clock`, advances, replaced function bodies, `SECURITY DEFINER` advance), `TestClockStore`, next-due query generated from the manifest (test: covers every `workflow_due` column), quiescence query; real PostgreSQL as the runtime role. — FR-028-004, 007 — evidence: `integration_developer_tooling clock_store`.
- [ ] T028-015 [US01] `DeploymentProfile` start-up check in `apps/control-plane` and `apps/execution-engine` (exit 2 on marker or replaced function), repeated by the engine's monitor round (stop on mismatch); tests in `apps/control-plane/tests/` and `apps/execution-engine/tests/`. — FR-028-002 — evidence: SC-028-002 refusal tests; a body replaced on a running engine stops it.
- [ ] T028-016 [US01] Skip algorithm in `crates/application/src/testing.rs` with the `PassDriver` port; unit tests with fakes; integration with the real `Scheduler::tick` and dispatcher. — FR-028-005..007 — evidence: `integration_developer_tooling skip`.
- [ ] T028-017 [US01] `apps/test-server`: `SAGAT_ENV=dev` only, empty-or-marked database, one tenant, composition of the control-plane and engine libraries with `DeploymentProfile::Test`, embedded issuer from `sagat-dev-idp` (loopback, key per start, token files `0600`), seeding, `/v1/testing/*` with authorization (the test server's own `testing:clock` check mapped to the admin and developer roles of its tenant), audit and `no-store`. — FR-028-001, 012, 013 — evidence: `apps/test-server/tests/`.
- [ ] T028-018 [US01] Contract tests of `testing.openapi.yaml` against the test server; every `/testing/*` path is `404 ROUTE_NOT_FOUND` on the production composition; test-server tokens refused by production. — FR-028-002, 013 — evidence: `contract_developer_tooling testing_api`.
- [ ] T028-019 [US01] `tests/fault/developer-tooling/clock.rs`: crash after the clock commit before the pass; two concurrent advances; `JUMP` over a leased attempt (late report `STALE_ATTEMPT`); timer vs signal at one stop (20 rounds); restart. — FR-028-004..008, SC-028-004 — evidence: fault suite.
- [ ] T028-020 [US01] Shifted-clock mode in `tests/support` (test-server composition, 400-day offset) over the durable-execution, sagas (including failed compensation) and workflow-primitives integration suites; daily cron `STEP` (30 runs) vs `JUMP` (1 run); a DST transition of an IANA zone. — FR-028-008, SC-028-003, SC-028-004 — evidence: suites equal to the zero-offset run.
- [ ] T028-021 [P] [US01] `ClientConfig.clock` in the five SDKs (duration options only); unit tests that heartbeat and backoff stay monotonic. — FR-028-010 — evidence: each SDK's unit suite.
- [ ] T028-022 [US01] `TestEnvironment` in TypeScript and Go (connect, capability check, clients, workers bound by name, `now`, `advance`, `skip`, `waitForCompletion`, `createNamespace`). — FR-028-009 — evidence: SDK unit suites.
- [ ] T028-023 [US01] `TestEnvironment` in Python, C# and Java. — FR-028-009 — evidence: SDK unit suites.
- [ ] T028-024 [US01] Conformance `time-skipping`, `time-skipping-blocked`, `time-skipping-cron`, `time-skipping-race` in `tests/contract/sdk-conformance/scenarios/`; the harness boots the test-server composition; goldens reviewed by a person. — FR-028-009, SC-028-001 — evidence: `sdk_conformance` passes in five SDKs.
- [ ] T028-025 [P] [US01] Activity test context in the five SDKs reusing the host's mapping; `tests/contract/sdk-conformance/fixtures/activity-context.json` with copy-equality tests. — FR-028-011 — evidence: five unit suites run the fixture.
- [ ] T028-026 [US01] CLI `sagat test-server start|stop|status|advance` (scratch database create/drop, `--json`) in `apps/cli/src/test_server.rs`; tests `apps/cli/tests/test_server.rs`. — FR-028-001, 029 — evidence: CLI tests.
- [ ] T028-027 [US01] `examples/time-skipping/` (reminder with timer, retries and approval expiry; a test per SDK), `docs/quickstarts/testing.md`, `docs/runbooks/test-server.md`; wall time per language in `tests/performance/reports/developer-tooling.md`. — SC-028-001 — evidence: example tests; report with environment.

**Checkpoint:** US01 demonstrable alone (quickstart §1).

## Phase 4 — US02 Interceptors (P1)

- [ ] T028-028 [US02] Merge [sdk-interceptors.md](contracts/sdk-interceptors.md) into `sdks/SPEC.md` §12 and add the catalog code to the five embedded catalogs. — FR-028-022..024 — evidence: catalog equality tests.
- [ ] T028-029 [P] [US02] TypeScript: client and worker interceptors (observe and decorate; no payload transform, decision C5) with their position on the application side of a data converter; unit tests for order, once per call, added headers, protected fields and headers, refused payload changes, failure mapping. — FR-028-019..023 — evidence: `npm test` in `sdks/typescript`.
- [ ] T028-030 [P] [US02] Go: same. — FR-028-019..023 — evidence: `go test -race ./...`.
- [ ] T028-031 [P] [US02] Python: same. — FR-028-019..023 — evidence: `pytest`, `mypy`.
- [ ] T028-032 [P] [US02] C#: same, with dependency-injection registration. — FR-028-019..023 — evidence: `dotnet test`.
- [ ] T028-033 [P] [US02] Java: same. — FR-028-019..023 — evidence: `./mvnw -q verify`.
- [ ] T028-034 [US02] Conformance `interceptors` and `interceptor-coverage` with `dropResponse` faults on `POST /runs` and `CompleteStep`; reviewed goldens. — FR-028-019..024, SC-028-005 — evidence: `sdk_conformance` in five SDKs.
- [ ] T028-035 [US02] Interceptor sections in `docs/runbooks/sdk-observability.md` and each SDK README; a note for feature 031 on the fixed order (chain, converter, transport). — FR-028-024, 045 — evidence: `contract_developer_experience` doc checks.

## Phase 5 — US03 History replay (P2; starts when feature 020's export is accepted)

- [ ] T028-036 [US03] Agree the field list of [history-replay.md](contracts/history-replay.md) with feature 020 and record the format version. — FR-028-016 — evidence: 020's accepted contract cites it.
- [ ] T028-037 [US03] `crates/domain/src/execution/replay_check.rs` and unit tests per divergence kind. — FR-028-014 — evidence: `cargo test -p sagat-domain replay_check`.
- [ ] T028-038 [US03] `crates/application/src/history_replay.rs`, the codec adapter in `crates/infra-testing`, route `/v1/testing/history-replays` with bounds and audit; integration tests (compatible, gap, unknown type, changed status, unsupported capability, oversize). — FR-028-014..016 — evidence: `integration_developer_tooling replay`.
- [ ] T028-039 [US03] Corpus test: export the conformance corpus runs with 020's exporter and replay with zero divergences. — SC-028-006 — evidence: `integration_developer_tooling replay_corpus`.
- [ ] T028-040 [US03] CLI `sagat history replay|answers|compare` (`apps/cli/src/history.rs`) with a classification canary (no undisclosed value written). — FR-028-017, 018 — evidence: `apps/cli/tests/history.rs`.
- [ ] T028-041 [US03] `replayHistory` in the five test environments; conformance `history-replay`. — FR-028-018 — evidence: `sdk_conformance` in five SDKs.

## Phase 6 — US04 VS Code extension (P2)

- [ ] T028-042 [US04] CLI `--json` (schema v1, `apps/cli/schemas/output.schema.json`) for the commands of [cli-output.schema.json](contracts/cli-output.schema.json), `workflow inspect --follow --json`, `sagat version --json`; contract test validates every output; existing human-output tests unchanged. — FR-028-029 — evidence: `apps/cli/tests/output.rs`.
- [ ] T028-043 [US04] Extract `frontend/packages/workflow-graph` from `features/workflows/canvasModel.ts` and `model/layout.ts`; add `packages/*` to `frontend/package.json` workspaces. — FR-028-026 — evidence: Studio `npm test` and browser journeys unchanged.
- [ ] T028-044 [US04] `frontend/apps/vscode-extension`: activation, settings (`cliPath`, `server`, `grpc`, `allowNonDev` read from user/machine scope only, in every workspace), workspace trust `limited`, secret storage keyed by server origin, CLI runner (token in env only, only to its origin), version check; unit tests. — FR-028-028 — evidence: extension unit suite, including a trusted workspace that names another server.
- [ ] T028-045 [US04] Diagnostics with pointer-to-range mapping; parity over `tests/contract/developer-experience/fixtures/diagnostics.json`. — FR-028-025, SC-028-007 — evidence: parity test.
- [ ] T028-046 [US04] Graph preview webview (CSP with nonce, closed message schema, table alternative, tokens); automated accessibility check. — FR-028-026, NFR-028-A11Y — evidence: webview tests.
- [ ] T028-047 [US04] Run/test commands (publish and run with the dev guard, `worker dev`, follow, cancel, test-server advance); `@vscode/test-electron` tests against a fake CLI that records argv and environment (token canary, untrusted workspace). — FR-028-027, 028, SC-028-007 — evidence: integration suite.
- [ ] T028-048 [US04] VSIX build in `.github/workflows/release.yml` per O1400; `docs/quickstarts/vscode.md`. — FR-028-045 — evidence: release dry run (`scripts/release-version.sh`).

## Phase 7 — US05 Git sync (P2)

- [ ] T028-049 [US05] `crates/domain/src/gitsync.rs`: settings validation, import decision table over (base, draft, repository, active), status derivation, conflict rules; exhaustive unit tests. — FR-028-034..036 — evidence: `cargo test -p sagat-domain gitsync`.
- [ ] T028-050 [US05] Migration of data-model §3 with grants, triggers, storage and time classification; runtime-role and up/down tests. — FR-028-030..036 — evidence: migration suite, `runtime_privileges`.
- [ ] T028-051 [US05] Export obligation inside the ADR-0018 transaction (`crates/infra-postgres/src/versioning.rs`); tests: a crash before commit leaves neither row; a publication commits with the provider down. — FR-028-031 — evidence: `fault_developer_tooling export_obligation`.
- [ ] T028-052 [P] [US05] `crates/infra-git`: GitHub (App RS256 assertion ≤ 10 min, installation token restricted to the repository, contents/trees/commits/refs/pulls) and GitLab (token; files/commits/branches/merge requests) through `crates/infra-egress`, bounded reads, redaction; recorded-fixture tests. — FR-028-032, 037 — evidence: `cargo test -p sagat-infra-git`.
- [ ] T028-053 [US05] Binding service and routes (`crates/application/src/gitsync.rs`, `crates/api-http/src/routes/gitsync.rs`): CAS and receipts, secret-access check, host allowlist (`GIT_DESTINATION_DENIED`), pause/resume/probe/delete, `RESYNC` on target change; the probe records whether the tracked branch requires an approving review (GitHub branch rules or protection, GitLab approval rules). — FR-028-030, 033 — evidence: contract and integration tests.
- [ ] T028-054 [US05] Export loop `crates/scheduler/src/gitsync.rs`: claim, lease and fence; render (canonical document and manifest); secret scan; pull-request and direct-push algorithms with lookup before create; record; backoff, `DEAD`, pause. — FR-028-032, 038 — evidence: integration with the provider stub.
- [ ] T028-055 [US05] Webhook route: rate limits per source address and per binding before any secret read, verification-secret cache (≤ 60 s), GitHub and GitLab signature verification, the same verification work for an unknown binding, uniform `401`, GitLab timestamp window, body cap, receipts, coalesced enqueue; legacy GitLab token refused. — FR-028-033, 037 — evidence: `integration_developer_tooling webhooks`, including a flood that reaches OpenBao no more than the cache allows and a timing comparison of known and unknown bindings.
- [ ] T028-073 [US05] Move the manifest rules out of `apps/cli/src/manifest.rs`: pure rules (closed schema, entry limits, paths below the directory, no links) to `crates/domain/src/manifest.rs`, parsing to `crates/infra-ir/src/manifest.rs` behind an application port; the CLI uses both. — FR-028-034 — evidence: `apps/cli/tests/manifest.rs` unchanged and passing; domain and infra-ir tests.
- [ ] T028-056 [US05] Import loop and head reconciliation: manifest and documents under the moved 011 rules (T028-073), per-workflow transactions, draft CAS with provenance, optional publication request, conflicts, removed entries. — FR-028-033..036 — evidence: `integration_developer_tooling import`.
- [ ] T028-074 [US05] Separation of duties for imported drafts: `DRAFT_AND_REQUEST` opens a request only after a positive review probe (else `GIT_REVIEW_NOT_VERIFIED`); publication of a draft last saved by `system:git-sync` needs the namespace's approvals plus one, checked in the ADR-0018 publication transaction; the request shows the unverified commit author and committer. — FR-028-034, SC-028-009 — evidence: integration tests with a provider stub that reports protected and unprotected branches; an attempt by the commit author's platform identity to be the only approver fails.
- [ ] T028-057 [US05] Conflict resolution and status APIs; races with a Studio edit and a superseding commit. — FR-028-035, 036 — evidence: race tests.
- [ ] T028-058 [US05] `tests/fault/developer-tooling/gitsync.rs`: crash at each export boundary, duplicate claim, stale lease, lost webhook, crash between import entries, provider rate limit, restart; one-branch/one-pull-request invariant. — SC-028-008, SC-028-009 — evidence: fault suite.
- [ ] T028-059 [US05] Spans, metrics, audit actions and `GIT_SYNC_*` alerts in the monitor; tests for closed labels and credential canaries in logs. — FR-028-043 — evidence: telemetry tests.
- [ ] T028-060 [US05] Studio `features/gitsync`: binding settings, status, jobs and redrive, three-way conflict view reusing `DiffView`, draft provenance; component tests and one browser journey. — FR-028-030..036, NFR-028-A11Y — evidence: `npm test`, `e2e_workflow_studio journeys`.
- [ ] T028-061 [US05] Real-provider journeys against GitHub and GitLab test repositories (ignored without credentials, run in CI with secrets); `docs/runbooks/git-sync.md`. — SC-028-008, SC-028-009 — evidence: CI job result.

## Phase 8 — US06 Template hub (P3)

- [ ] T028-062 [US06] `crates/domain/src/templates.rs`: version, expiry, validity, digest and kind rules; unit tests. — FR-028-040 — evidence: `cargo test -p sagat-domain templates`.
- [ ] T028-063 [US06] Migration of data-model §4 with grants and classification. — FR-028-039..041 — evidence: migration suite.
- [ ] T028-064 [US06] Index refresh (bundled and operator indexes through the egress guard), Ed25519 verification (`ring`), storage; tests for tampered, unpinned, rolled-back, expired and oversized indexes. — FR-028-039, 040, SC-028-010 — evidence: `integration_developer_tooling catalog`.
- [ ] T028-065 [US06] Import service and routes: re-verify, codec validation, draft with provenance, requirements, idempotency, quotas and rate. — FR-028-041, 042 — evidence: integration and contract tests.
- [ ] T028-066 [US06] Release-bundled catalog in `templates/catalog/` (the six `templates/` directories, `examples/order-processing/workflows/*.workflow.json` as `order-routing`, `order-fulfillment`, `order-reminder`, and the reminder of T028-027), signed in the release job, embedded in the control plane; CI verifies the bundled index. — FR-028-039 — evidence: release dry run.
- [ ] T028-067 [US06] Studio `features/templates` (browse, detail, import dialog), CLI `sagat template list|show|import`, `docs/runbooks/templates.md`. — FR-028-042 — evidence: component tests, `apps/cli/tests/template.rs`.

## Phase 9 — Gates

- [ ] T028-068 Extend `tests/contract/security-multitenancy/rest/battery.rs` to every new operation; credential canaries (App key, installation token, GitLab token, editor token); independent `security-reviewer` read. — NFR-028-SEC, SC-028-011 — evidence: battery result and review notes.
- [ ] T028-069 Ceilings at their limits (typed refusals) and closed metric labels for every new resource. — FR-028-044, SC-028-012 — evidence: `integration_developer_tooling ceilings`.
- [ ] T028-070 `tests/performance/reports/developer-tooling.md`: production pass latency before/after the workflow-time function, skip stops per second, publish-to-commit latency, catalog refresh, with environment and failures. — NFR-028-PERF — evidence: the report.
- [ ] T028-071 Documentation: time classes in `docs/architecture/execution-semantics.md`, notes in `observability-slo.md` and `security-and-sandbox.md`, runbooks and quickstarts listed above, `docs/traceability.md`. — FR-028-045 — evidence: `contract_developer_experience` doc checks.
- [ ] T028-072 Run the [quickstart](quickstart.md) happy and failure demos from a clean checkout; `correctness-reviewer` and `architecture-reviewer` reads; final gate list with every unrun test stated. — SC-028-013 — evidence: demo record.

## Dependencies

Phase 2 blocks every story. US01 and US02 are independent after Phase 2 (US02 needs only T028-005). US03 waits for
feature 020 and uses the test server of US01. US04 needs T028-042 and uses US01's CLI commands for the test-server
actions. US05 and US06 need Phase 1 and T028-005 only (US05 also T028-073); US06's bundled catalog includes the US01
example. Feature 031 depends on this feature only for the order of its data converter relative to the interceptor
chains (decision C5); neither blocks the other.
