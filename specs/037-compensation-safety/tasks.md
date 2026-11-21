# Tasks: Compensation safety — delivery 1 and qualification gate

**Input**: [plan](plan.md), [spec](spec.md), [research](research.md), [data model](data-model.md), [contracts](contracts/README.md).
**Status**: owner approved delivery 1 on 2026-11-01 (T037-001..031 and T037-034); delivery-1 implementation and local validation complete. US04 remains a separate gate.
Tests are mandatory under the Constitution/user request. Task descriptions identify required verification; completed entries cite the observed evidence.
Each task closes only with named commands/results recorded in validation-report.md. All paths are repository-relative.

## Phase 1: Setup

- [x] T037-001 Record approval scope and prepare an isolated feature checkout from the reviewed develop tree, preserving all accumulated work; inspect branches/worktrees and disk first. Files: `specs/037-compensation-safety/validation-report.md`, `docs/owner-decisions.md`. **Accept**: exact approved plan/tree recorded; no direct develop commit, destructive reset or duplicate feature. **Verify**: git status/branch/worktree and baseline hashes. **Depends**: owner approval of this plan and delivery 1.

## Phase 2: Foundational contract

- [x] T037-002 Synchronize the delivery-1 semantic contract and transition matrix in `sdks/SPEC.md`, `docs/architecture/sagas.md`, `specs/037-compensation-safety/contracts/README.md`, `specs/037-compensation-safety/data-model.md`. **Accept**: pending uses OUTCOME_UNKNOWN; NotApplied reviews; existing explicit fail and human provenance preserved; no new wire/property/qualification stub. **Verify**: contract review, link check, git diff --check. **Depends**: T037-001.

## Phase 3: US03 — Hold unknown effects (P1, prerequisite for review preservation)

Independent acceptance: root and exact-path uncertainty never dispatch blind retries or defensive compensation;
negative lookup and unknown compensation retain review through replay. No real provider is qualified.

- [x] T037-003 [US03] Write red tests `unknown_activity_defaults_to_review`, `negative_lookup_requires_finality`, `unknown_compensation_overrides_fail` in `crates/domain/tests/sagas.rs` and `crates/domain/tests/scoped_compensation.rs`. Cover absent/false highRisk, attempts remaining/exhausted and cancel already requested; assert replay. **Accept**: failures demonstrate current unsafe transitions, not setup errors. **Verify**: focused domain test command in quickstart, record named red output. **Depends**: T037-002.
- [x] T037-004 [US03] Implement source-aware uncertainty precedence and negative-review result in `crates/domain/src/execution/command.rs`, `crates/domain/src/execution/event.rs`, `crates/domain/src/execution/state_effects.rs`. **Accept**: T037-003 green; new RECONCILIATION_NOT_FINAL follows existing constraint `[A-Za-z0-9_.:-]`, 1–64 characters and remains unknown; lookup failures do not recursively reconcile. **Verify**: focused domain tests, including known failure/retry regressions. **Depends**: T037-003.
- [x] T037-005 [US03] Add red `unknown_effect_is_not_a_compensation_candidate` and forged-plan replay cases in `crates/domain/tests/sagas.rs`, `crates/domain/tests/scoped_compensation.rs`, `crates/domain/tests/bounded_path_planning.rs`. **Accept**: root/full/path reducers reject non-Succeeded candidates while known and human-resolved success remain eligible. **Verify**: named red test results and bounded/full parity. **Depends**: T037-004.
- [x] T037-006 [US03] Restrict plan candidates and replay validation in `crates/domain/src/execution/sagas.rs`, `crates/domain/src/execution/scoped_sagas.rs`, `crates/domain/src/execution/state.rs`. **Accept**: no unknown Failed/Cancelled candidate, irreversible completed effects still retained. **Verify**: T037-005 green plus complete sagas/scoped domain suites. **Depends**: T037-005.
- [x] T037-007 [US03] Add red `unknown_effect_retry_requires_resolution` and original/known-compensation controls in `crates/domain/tests/operator_actions.rs`, `tests/integration/sagas-compensations/review.rs`. **Accept**: unknown original and compensation cannot directly retry even with SagaOverride; authorized outcome-bearing resolve/skip still works; ordinary cancel is not override. **Verify**: named red tests; known original retry only while forward work is allowed; Resolve is distinct from Retry. **Depends**: T037-006.
- [x] T037-008 [US03] Enforce refusal at command/reducer/application/HTTP boundaries in `crates/domain/src/execution/command.rs`, `crates/domain/src/execution/state_effects.rs`, `crates/application/src/runs.rs`, `crates/api-http/src/routes/execution.rs`. **Accept**: deterministic typed refusal, unchanged execution state/no scheduled work or success audit/idempotency; existing permission-denial audit retained; typed conflict/request trace explains state refusal; no permission widening. **Verify**: T037-007 green under runtime role and replay. **Depends**: T037-007.
- [x] T037-009 [US03] Add red root output-refusal and retain scoped known-success controls in `tests/integration/durable-execution/runtime_quotas.rs`, `tests/integration/workflow-primitives/scoped_worker.rs` and `tests/integration/workflow-primitives/byte_ceilings.rs`. Adjust `crates/application/src/worker/output.rs`, `crates/application/src/worker/scoped_reports.rs` and `crates/domain/src/execution/command.rs` only after red evidence. **Accept**: root lost successful output holds review; scoped atomic stop+known-success remains known; no forward work leaks. **Verify**: relevant integration suites with runtime role, journal/projection/outbox consistency. **Depends**: T037-008. **Evidence (2026-11-01)**: `evidence/d1-review-correction-summary.md` records RED then GREEN output/refusal cases; durable integration 179 passed and parent hardening/quota rerun 27 passed; parent changed primitives modules 90 passed plus scoped blob module 8 passed.
- [x] T037-010 [US03] Cover `negative_lookup_never_leases_second_attempt` and stale reports in `tests/integration/sagas-compensations/reconcile.rs`, `tests/integration/workflow-primitives/scope_compensation_http.rs`; include collection and loop instance paths. **Accept**: no second lease; one durable lookup+review transaction; manual outcome still auditable. **Verify**: integration_sagas_compensations and integration_workflow_primitives with runtime role. **Depends**: T037-009. **Evidence (2026-11-01)**: `evidence/d1-final-workspace.txt`: integration_sagas_compensations 81 passed; `evidence/d1-parent-primitives-changes.txt`: 90 passed including scoped negative-lookup and stale-report controls. Full primitives completed 749 passed/1 fixture failure/22 explicit ignores; that fixture correction passed independently in `evidence/d1-parent-scoped-blob.txt` (8-case module).

Checkpoint: review root/scoped/ordinary-workflow behavior and output-refusal distinction before changing stop logic.
No task above closes US04 or certifies a provider.

## Phase 4: US01 — Preserve the operator obligation (P1)

Independent acceptance: cancel/deadline before or after failure leaves the same recovery obligation;
one authorized decision wins, survives restart, and retains human provenance.

- [x] T037-011 [US01] Add red `stop_preserves_recovery_obligation` cases in `crates/domain/tests/sagas.rs`, `crates/domain/tests/scoped_compensation.rs`; include ordinary activity outside a saga, reconciliation companion pending/ready/running, loop and collection, before/after cancel/deadline. **Accept**: old give-up assertions replaced with recorded rationale and new red expectations. **Verify**: domain suites and replay. **Depends**: T037-010.
- [x] T037-012 [US01] Preserve unresolved review/lookup and compensation review in `crates/domain/src/execution/sagas.rs`, `crates/domain/src/execution/plan.rs`, `crates/domain/src/execution/scoped_sagas.rs`. **Accept**: shared obligation predicate protects drain and joined-leftover cancellation; ordinary timers/held forward work can still cancel; independent recovery proceeds. **Verify**: T037-011 green and existing drain regressions. **Depends**: T037-011.
- [x] T037-013 [US01] Test then correct continuation diagnostics in `crates/domain/tests/continuation_quiescence.rs`, `crates/domain/src/execution/continuation.rs`, `crates/infra-postgres/src/chain_progress.rs`; add real lock-time coverage in `tests/integration/workflow-primitives/chains.rs`. **Accept**: UnknownEffect/Compensation blockers are truthful and no segment bypasses review. **Verify**: domain continuation suite and runtime-role workflow-primitives integration. **Depends**: T037-012. **Evidence (2026-11-01)**: `evidence/resume-chains.txt` (22 passed, including `the_locked_handoff_cannot_bypass_unknown_or_compensation_review`) and `evidence/resume-domain-application.txt` (964 passed across domain/application/telemetry).
- [x] T037-014 [US01] Prove cancel/deadline, restart and two-decision race in `tests/integration/sagas-compensations/stopping.rs`, `tests/integration/sagas-compensations/review.rs`, `tests/integration/workflow-primitives/scope_compensation_http.rs`. **Accept**: held database lock/barrier, no sleeps; one human decision/audit wins, cancel cannot erase review; cross-tenant/revoked authority fails closed. **Verify**: both integration suites with runtime role and SQL consistency assertions. **Depends**: T037-013. **Evidence (2026-11-01)**: Saga integration 81 passed in `evidence/d1-final-workspace.txt`; parent scoped/primitives modules 90 passed in `evidence/d1-parent-primitives-changes.txt`; authority downgrade/removal control passed in 21-case saga E2E (`evidence/d1-saga-e2e-final.txt`).

Checkpoint: operator history shows the actor/reason after manual success; this can yield COMPENSATED
without claiming provider confirmation. No new manual UI design is introduced.

## Phase 5: US02 — Truthful terminal compensation failure (P1)

Independent acceptance: known permanent failure under explicit fail ends only after eligible independent
work settles and blocked dependents are suppressed; unknown still reviews.

- [x] T037-015 [US02] Extend terminal/order invariants in `crates/domain/tests/sagas.rs`, `crates/domain/tests/scoped_compensation.rs`, `crates/domain/tests/execution_properties.rs`. **Accept**: terminal means no READY/LEASED/RUNNING/RETRY_WAIT/WAITING compensation; fail is explicit-saga only; known retryable then exhausted/nonretryable cases differ from uncertainty. **Verify**: named `terminal_failure_waits_for_independent_compensation` and property tests; retain already-passing evidence rather than invent red failures. **Depends**: T037-014.
- [x] T037-016 [US02] Repair only failures exposed by T037-015 in `crates/domain/src/execution/sagas.rs`, `crates/domain/src/execution/scoped_sagas.rs`, `crates/domain/src/execution/state.rs`; otherwise close with evidence of unchanged existing guards. **Accept**: no early terminal or post-terminal scheduling; retained failed/skipped/dependent state remains honest. **Verify**: domain invariant suites and replay; no speculative refactor. **Depends**: T037-015.
- [x] T037-017 [US02] Prove terminal SQL projection and independent work under forward quota exhaustion in `tests/integration/sagas-compensations/compensation.rs`, `tests/integration/durable-execution/runtime_quotas.rs`. **Accept**: recovery cannot be erased by forward capacity; no terminal active compensation; failure history retained. **Verify**: relevant integration suites with runtime role and final SQL assertions. **Depends**: T037-016. **Evidence (2026-11-01)**: Saga integration 81 passed; durable integration 179 passed (`evidence/d1-durable-output-budget-green.txt`), with parent hardening/runtime-quota rerun 27 passed (`evidence/d1-parent-durable-output.txt`).

## Phase 6: Client, fault and operational convergence (US01–US03)

- [x] T037-018 Add shared scenario `tests/contract/sdk-conformance/scenarios/compensation-safety.json`, expected `tests/contract/sdk-conformance/expected/compensation-safety.json` and `tests/contract/sdk-conformance/expected/compensation-safety.workflows.json`, and dispatch support in `tests/contract/sdk-conformance/main.rs`/`control.rs` only if needed. **Accept**: pending/unknown, negative lookup, known fail, cancellation and manual-provenance cases use real server; no fake provider qualification. **Verify**: scenario structure plus first language run; read all golden diffs, rerun without bless. **Depends**: T037-017. **Evidence (2026-11-01)**: `evidence/d1-six-checks.json`: shared compensation-safety scenario passed in all six SDKs without bless.
- [x] T037-019 [P] Add TypeScript scenario behavior in `sdks/typescript/conformance/runner.ts` and error-preservation checks in `sdks/typescript/test/worker.test.ts`. **Accept**: OUTCOME_UNKNOWN preserved, no implicit repeat; same shared expected output. **Verify**: TypeScript checks from sdks/AGENTS.md and scenario run. **Depends**: T037-018. **Evidence (2026-11-01)**: `evidence/d1-sdk-checks.json`: TypeScript npm test exit 0; `evidence/d1-six-checks.json`: four shared scenarios passed in all six SDKs.
- [x] T037-020 [P] Add Go scenario behavior in `sdks/go/conformance/worker.go`, `sdks/go/conformance/run.go`. **Accept**: same reserved-code/finality semantics and shared output. **Verify**: Go formatting/vet/tests and scenario run. **Depends**: T037-018. **Evidence (2026-11-01)**: `evidence/d1-sdk-checks.json`: Go race tests, vet and formatting exit 0; `evidence/d1-six-checks.json`: four shared scenarios passed.
- [x] T037-021 [P] Add Python scenario behavior in `sdks/python/conformance/runner.py` and `sdks/python/conformance/faults.py`. **Accept**: same reserved-code/finality semantics and shared output. **Verify**: Python mypy/ruff/tests and scenario run. **Depends**: T037-018. **Evidence (2026-11-01)**: `evidence/d1-python-final.json`: 264 pytest tests, mypy and ruff passed after installing the locked development tools; `evidence/d1-six-checks.json`: four shared scenarios passed.
- [x] T037-022 [P] Add C# scenario behavior in `sdks/dotnet/conformance/Behaviors.cs`, `sdks/dotnet/conformance/RunMode.cs`. **Accept**: same reserved-code/finality semantics and shared output. **Verify**: dotnet formatting/tests and scenario run. **Depends**: T037-018. **Evidence (2026-11-01)**: `evidence/d1-sdk-checks.json`: dotnet tests and format exit 0; `evidence/d1-six-checks.json`: four shared scenarios passed.
- [x] T037-023 [P] Add Java scenario behavior in `sdks/java/conformance/src/main/java/dev/sagatoctopusorchestrator/conformance/Behaviors.java`, `sdks/java/conformance/src/main/java/dev/sagatoctopusorchestrator/conformance/Runner.java`. **Accept**: same reserved-code/finality semantics and shared output. **Verify**: Maven verify and scenario run. **Depends**: T037-018. **Evidence (2026-11-01)**: `evidence/d1-sdk-checks.json`: Maven verify exit 0; `evidence/d1-six-checks.json`: four shared scenarios passed.
- [x] T037-024 [P] Add Rust SDK scenario behavior in `sdks/rust/conformance/src/runtime.rs`, `sdks/rust/conformance/src/main.rs`. **Accept**: same reserved-code/finality semantics and shared output. **Verify**: SDK cargo checks/tests and scenario run with private target. **Depends**: T037-018. **Evidence (2026-11-01)**: `evidence/d1-sdk-checks.json`: Rust SDK fmt, clippy and tests exit 0; `evidence/d1-six-checks.json`: four shared scenarios passed.
- [x] T037-025 Update Studio operation guards in `frontend/apps/studio/src/features/runs/ops.ts` and `frontend/apps/studio/src/features/runs/ops.test.ts`, `frontend/apps/studio/src/features/runs/operations.test.tsx`, `frontend/e2e/tests/us04-act.spec.ts`, `frontend/apps/studio/src/features/runs/OperationDialog.tsx` (remove obsolete retry-confirmation input). **Accept**: RECONCILIATION_NOT_FINAL is unknown; unknown original and compensation retry disabled even with override; permitted resolve/skip and evidence stay visible; cancellation wording preserves review. **Verify**: focused Studio unit tests plus type/build checks; real browser smoke if UI behavior changes. **Depends**: T037-017. **Evidence (2026-11-01)**: `evidence/d1-frontend-final-checks.json`: lint/types/API/build/tests exit 0 (1,194 Studio tests plus 11 tooling tests); `evidence/d1-browser-all.txt`: 38 real-browser tests passed, 2 optional performance tests skipped.
- [x] T037-026 Add REST/gRPC negative and review cases in `tests/contract/sagas-compensations/rest/conformance.rs`, `tests/contract/sagas-compensations/grpc/protocol.rs`. **Accept**: undeclared qualification/purity fields cannot activate exceptions; pending is not completion; new code reads through existing APIs, no wire-shape drift. **Verify**: both contract suites and canonical schema validation. **Depends**: T037-017. **Evidence (2026-11-01)**: `evidence/d1-final-workspace.txt`: REST saga contract 4 passed, gRPC saga contract 9 passed; `evidence/d1-contract-checks.json`: canonical schema/contract checks exit 0.
- [x] T037-027 Add transaction/crash/duplicate/lost-hint fault coverage in `tests/fault/sagas-compensations/provider.rs`, `tests/fault/sagas-compensations/crashes.rs`, `tests/fault/sagas-compensations/chaos.rs`, `tests/integration/workflow-primitives/scoped_worker.rs`, `tests/integration/sagas-compensations/review.rs`. **Accept**: failed required commit/audit dispatches nothing; identical report retry yields one journal decision; restart preserves identity; no sleeps for races. **Verify**: saga/workflow fault suites and affected runtime-role integration suites. **Depends**: T037-017. **Evidence (2026-11-01)**: `evidence/d1-final-workspace.txt`: fault_sagas_compensations 14 passed; parent fault_workflow_primitives 92 passed (`evidence/d1-parent-sandbox-workflow-faults.txt`), changed primitives modules 90 and corrected blob module 8 passed independently. Required audit/outbox rollback and held-lock races remain asserted.
- [x] T037-028 Add managed-database uncertainty controls in `tests/integration/database-connections/output.rs`. **Accept**: ambiguous external SQL under cancel/deadline remains review, no automatic repeat/defensive compensation; isolation and secret-free evidence intact. **Verify**: database-connections integration with runtime role. **Depends**: T037-017. **Evidence (2026-11-01)**: `evidence/d1-database-connections-full.txt`: runtime-role integration target passed all 7 wrappers, including managed-secret/output lifecycle cases.
- [x] T037-029 Validate recovery observability in `crates/telemetry/src/lib.rs`, `crates/telemetry/tests/metrics.rs`, `crates/telemetry/tests/span.rs`, `crates/application/tests/telemetry_derive.rs`, `tests/e2e/sagas-compensations/telemetry.rs`, `tests/integration/sagas-compensations/review.rs`; record evidence in `specs/037-compensation-safety/validation-report.md`. **Accept**: existing events/spans/counters expose review/refusal reasons without payloads or tenant/run labels; waiting review uses no lease or scheduler spin; measure bytes/indexes, queries/wakeups and list latency at 1/100/1,000 synthetic reviews on a stated fixture; stop on capacity leakage, busy loop, evidence loss or machine-budget exceedance. Add bounded reason instrumentation only if missing. **Verify**: telemetry tests plus idle-review integration observation; report measured environment, no invented SLO. **Depends**: T037-027. **Evidence (2026-11-01)**: `evidence/d1-review-query-binding.txt`: explicit 1/100/1,000-review observation passed; environment/bytes/index plans/list latency/idle SQL recorded in validation-report.md. Telemetry/domain gates and 21-case saga E2E passed; no SLO or provider finality inferred.
- [x] T037-030 Update follow-up and installation guidance in `docs/runbooks/sagas.md`, `sdks/SPEC.md`, `specs/037-compensation-safety/quickstart.md`, `docs/architecture/sagas.md`. **Accept**: retained evidence and operator responsibility clear; state journal/audit retention gaps G28/G182, actual controls or their absence and operator-owned backup/export; no post-terminal queue promise; fresh isolated install and no rollback into live 037 data; no real provider guarantee claimed. **Verify**: documented happy/failure demos and link/command checks. **Depends**: T037-025, T037-026, T037-029. **Evidence (2026-11-01)**: validation-report.md maps all six demonstrations to named passing tests and documents the unexecuted restore/real-provider rollback limits; `evidence/d1-smoke-checks.json` and `evidence/d1-extra-browser.json` prove isolated installation/onboarding; `evidence/d1-delivery-doc-links-final.txt` checks 100 local links in 14 delivery documents, zero missing.
- [x] T037-034 Update obsolete negative-lookup expectations in `tests/contract/sdk-conformance/scenarios/saga-compensation.json`, `tests/contract/sdk-conformance/expected/saga-compensation.json`; inspect collection/loop goldens and change only evidenced differences. **Accept**: the old scenario no longer waits for an automatic retry that delivery 1 forbids; explicitly resolves the review if subsequent steps require it; no unrelated golden churn. **Verify**: read diffs, run saga/collection/loop compensation scenarios in all six SDKs without bless; any additional files receive a traced small task amendment. **Depends**: T037-019, T037-020, T037-021, T037-022, T037-023, T037-024. **Evidence (2026-11-01)**: `evidence/d1-six-checks.json`: saga-, collection- and loop-compensation passed across all six languages without bless; reviewed golden differences retained.
- [x] T037-031 Run integrated delivery-1 gates from quickstart and record exact commands/results in `specs/037-compensation-safety/validation-report.md`, `specs/037-compensation-safety/analysis.md`, `docs/traceability.md`. **Accept**: six SDKs, domain/real PostgreSQL runtime-role/fault/contract/Studio and CI gates pass on the same tree; no silent missing/ignored language; independent reviewer findings resolved; no US04 closure. **Verify**: owner-readable evidence with commit/tree identity, runtime versions and limitations. **Depends**: T037-019, T037-020, T037-021, T037-022, T037-023, T037-024, T037-028, T037-030, T037-034. **Evidence (2026-11-01)**: final composed gate ledger in validation-report.md; parent full SDK E2E 6/fault 9 passed (`evidence/d1-parent-sdk-wrappers.json`), four corrected canonical scenarios passed across six languages without bless, all final static gates exit 0 (`evidence/d1-composed-static.json`). Broad initial REDs and their correction retests remain explicit; source manifest is `evidence/d1-source-tree.json`. US04 remains open.

## Phase 7: US04 — Evidence/design gate, no registry implementation tasks

- [ ] T037-032 [US04] Collect one real provider/capability contract and immutable adapter test evidence; record selection, accountable approval and unresolved trust/finality/identity questions in `specs/037-compensation-safety/research.md`, `specs/037-compensation-safety/contracts/README.md`. **Accept**: applied/absent/late original, repeated/late/pending compensation and timeout-after-acceptance covered; simulated tests alone do not qualify. **Verify**: evidence provenance/digests and independent review; if unavailable, remain open. **Depends**: provider/adapter selection and access to authoritative evidence (not yet supplied); may be researched independently of delivery 1.
- [ ] T037-033 [US04] Present exact qualification publication/use/revocation/worker contract design in `specs/037-compensation-safety/plan.md`, `specs/037-compensation-safety/data-model.md`, `specs/037-compensation-safety/contracts/README.md`. **Accept**: scope/identity/current authority, in-flight revocation, retained reports, no original-repeat exception and tenant isolation specified; alternatives/risk/rollback reviewed. **Verify**: fresh EGM critique and explicit owner design approval before generating implementation tasks. **Depends**: T037-032.

## Dependencies and execution strategy

Setup → contract → US03 → US01 → US02 → client/fault/operations → delivery-1 evidence.
US04 evidence → approved exact design → future implementation tasks (not present here).

US01–US03 are individually testable but share engine files; execute sequentially, with red-first
pairs kept together. Client tasks T037-019..024 are the only explicit parallel group after T037-018;
respect the two-heavy-build machine limit. US04 can collect evidence independently, but cannot
activate an exception or be counted complete by delivery-1 tests. Integration failures needing
additional production files require a small traced task amendment before widening a task silently.

First implementation approval covers delivery-1 tasks T037-001..031 and T037-034 only. Review each checkpoint before starting the
next phase; request a new owner decision only for a material scope/design change, not routine fixes
within the approved plan. No commit, merge, deployment or deletion is authorized by task preparation.
Full feature acceptance requires US04 implementation and SC-037-004 later, not merely T037-033.

### Delivery-1 validation amendments (2026-11-01)

- T037-007/008: independently discovered stopped/ancestor-draining scoped manual retry gap;
  extend full/path regressions in `scoped_compensation.rs` and `bounded_path_planning.rs` before
  correcting the command guard. This preserves the approved refusal contract.
- T037-009: successful result over the logical run output ceiling must retain uncertainty even
  when tenant storage still has room. Cover root and loop; retain the existing flat collection
  atomic known-success fallback and internal/input `RUN_DATA_LIMIT` controls. The attempted nested
  output fixture was outside the accepted public subset and was removed with its failed evidence
  retained in research.md; no nested-output runtime acceptance is claimed. Nested Retry parity
  remains covered by full/path domain tests.
- T037-029: collect EXPLAIN from the exact observed review-list statement, plus host/load/free-disk
  context. Local exam guard: one disposable PostgreSQL fixture, at most 1,000 synthetic reviews,
  600 seconds setup per scale, 60 seconds per observation, at least 20 GiB free host disk and
  1 GiB total measured review relations. These are test abort thresholds, not product SLOs.
- T037-031: unblock the existing frontend advisory gate with a narrowly scoped import bridge to
  the patched matcher, verified through actual locked tooling and Node 22 workspace-filtered
  Docker installation. No advisory suppression, lifecycle patch script or new product capability.

- T037-031 validation fixture amendment: `scripts/tests/test_compose_diagnostics.py` writes its
  synthetic pipe concurrently with the reader; observed macOS preload deadlock and final suite
  results are recorded. No production diagnostic behavior changed.

- T037-029/T037-031 E2E amendment: `tests/e2e/sagas-compensations/telemetry.rs`, `us02.rs`
  and `us03.rs` contained prior automatic-repeat expectations after negative lookup/expired
  compensation. Observed REDs precede explicit human resolution and no-repeat assertions.
- T037-014 adds `tests/e2e/sagas-compensations/us04.rs` for same-token role demotion and membership
  revocation against the actual review route; this is a feature-003 story file, not feature-037 US04.

- T037-026/T037-031: update `tests/contract/durable-execution/grpc/{blobs,protocol}.rs` after
  workspace REDs to preserve blob-receipt replay after GC and stale-lease rejection through human
  resolution, without an automatic second lease.
- T037-027/T037-031: update obsolete crash/negative-lookup E2E expectations in
  `tests/e2e/durable-execution/{stories,stream_recovery,engine}.rs` and
  `tests/e2e/developer-experience/stories.rs` after observed workspace failures. Synchronize the
  superseded takeover wording in `specs/002-durable-execution/spec.md` with the already approved
  ordinary-activity uncertainty rule; no new runtime capability or provider exception.

- T037-027/T037-031: observed full-workspace REDs extend test-only convergence to
  `tests/fault/failover/workers.rs`, `tests/fault/inline-code-sandbox/{loss,races}.rs` and
  `tests/fault/security-multitenancy/egress.rs`. Lost originals retain review rather than automatic
  replacement; surviving/fresh work and stale fencing remain asserted. Script cancellation uses
  a durable real-script barrier and bounded owner-specific cleanup observation, not fixed sleeps.
- T037-031 environment correction: unset inherited `RUST_LOG=warn` for collector-based gates.
  The variable made the engine fixture install a global logger before the collector. The same
  acquisition case fails with that variable and passes without it; no Collector assertion or
  production logging behavior was changed. Interrupted-suite database teardown errors are not
  treated as new product defects; a clean complete run is required.

- T037-031 verification environment: unset inherited `RUST_LOG` for collector-based tests;
  reproduce the logging conflict without changing collector assertions. Run the CI opt-in
  SDK user stories/faults and onboarding/database browser targets as well as the four selected
  compensation scenarios; record each result separately.

- T037-031/T037-034 scope amendment from the full opt-in CI RED: audit SDK user-story
  assertions in `tests/e2e/sdk-multilanguage/us03.rs`, fault assertions in
  `tests/fault/sdk-multilanguage/workers.rs`, and shared `failures-and-retries`,
  `lease-lost`, `worker-shutdown` scenarios/goldens. Preserve known-failure retry controls;
  replace only evidenced obsolete unknown-effect automatic-repeat expectations, with
  review/no-second-attempt assertions and explicit simulated human provenance where needed.
  Inspect candidate golden diffs before updating, then rerun all six languages without bless.

- T037-031 late broad-gate amendments: preserve revoked-worker/incident assertions in
  `tests/integration/security-multitenancy/bindings.rs` and `incidents.rs` while removing only
  obsolete automatic-takeover assumptions. In `tests/integration/scalability-ha/fairness.rs`,
  exercise the existing owner-only scheduling-weight command through the owner fixture,
  and assert the runtime role remains read-only; do not grant runtime write permissions.
  Verify each complete target and record the original RED separately.

- T037-034 final SDK-reference amendment: the `lease-lost.workflows.json` and
  `worker-shutdown.workflows.json` goldens change only to add the distinct successor released
  after simulated human resolution. Runtime goldens cover refusal of direct retry, one original
  attempt and actor/reason provenance. `saga-compensation` adds the same explicit audit evidence;
  `failures-and-retries` replaces its fixed wait with a journal barrier and preserves known retries.
  Reviewed source/golden diffs are retained under `evidence/d1-sdk-*-review.diff`; no SDK core
  implementation or provider-qualification exception is added.
