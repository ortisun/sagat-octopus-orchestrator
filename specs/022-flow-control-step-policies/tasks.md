# Tasks: Flow control and step policies

**Input**: `specs/022-flow-control-step-policies/` (spec, plan, research, data-model, contracts, quickstart)

Status: Proposed — not started. A task is checked only when its named evidence has been run and recorded
(constitution VIII).

**Format**: `- [ ] T022-NNN [P?] [USn?] Description — requirements. Evidence.` `[P]` = parallelizable.
Stories: US1 cache, US2 concurrency, US3 failure handlers, US4 early return, US5 standalone, US6 draft test.
US7 rate limits and the release 0.3.0 slice (OD-2026-11-13-11) are in [Release 0.3.0 slice](#release-030-slice-us02-and-us07).

## Phase 1: Setup

- [ ] T022-001 Owner review of spec, ADR-0056 and O1160–O1162; the coordinator integrates shared indexes. All FR. Evidence: recorded owner answers; ADR status set by a person.
- [ ] T022-002 [P] IR fixtures in `tests/contract/flow-control-step-policies/fixtures/` (`order-intake.workflow.json` and one invalid document per `DefinitionProblem` rule of `contracts/errors.md`). FR-022-001,008,012,017,034. Evidence: used by T022-006 and T022-048..052.
- [ ] T022-003 [P] Test targets `integration_`, `fault_`, `contract_`, `e2e_`, `perf_flow_control_step_policies` in `tests/Cargo.toml` with `tests/{integration,fault,contract,e2e,performance}/flow-control-step-policies/main.rs`. Evidence: `cargo test -p sagat-tests --no-run --locked`.

## Phase 2: Foundational (blocks every story)

- [ ] T022-004 Domain policies and pure validation in new `crates/domain/src/workflow/flow_control.rs` and `crates/domain/src/workflow.rs` (cache, concurrency, handler, `EarlyReturn`, placement/region/effect/combination and `startAt` rules). FR-022-001,008,012,017,028. Evidence: unit and property tests; old definitions unchanged.
- [ ] T022-005 Capability gates `stepCache`, `earlyReturn`, `failureHandler`, `concurrencyKeys` in `crates/infra-ir/src/convert.rs`; configuration in `apps/control-plane` and `apps/execution-engine`. FR-022-031,036; SC-022-013. Evidence: previous corpus unchanged with gates on/off; disabled gate refuses before a version exists.
- [ ] T022-006 IR codec in new `crates/infra-ir/src/flow_control.rs` (closed shapes, problem codes, canonical key-expression text; with 031, its shared `EncodingCheck` for cache keys, concurrency keys and `earlyReturn.result` carriage, using 031's `ENCODING_*` codes). FR-022-001,002,008,012,017,034,036. Evidence: contract test over the T022-002 corpus; digest stability.
- [ ] T022-007 Keyed digests in new `crates/application/src/keyed_digest.rs` (`ring::hmac`, namespace separation, labels, key version) and key configuration; activation of a new key version refused while semaphore rows exist under the old one. FR-022-002,020,022; NFR-022-006. Evidence: unit tests; redacted Debug; log canary; rotation-refusal test.
- [ ] T022-008 Journal events and reducer rules in `crates/domain/src/execution/{event.rs,state.rs,command.rs}` and new `execution/flow_control.rs`; mocked completions excluded in `execution/{sagas.rs,plan.rs}`; closed snake_case `detail` keys. FR-022-003,009,013,018,027,038. Evidence: replay test per event of `contracts/journal-events.md`; codec test that every detail key is snake_case.
- [ ] T022-009 Additive migration in `crates/infra-postgres/migrations/` after `20261220000000` (tables and `runs` columns of `data-model.md`, origin CHECK, triggers, `sagat_app` grants, refusing down). FR-022-005,007,016,020,026. Evidence: up/down test; `tests/integration/platform-foundation/runtime_privileges` sweep.
- [ ] T022-010 [P] Entries in `crates/infra-postgres/storage-classification.json`. FR-022-007. Evidence: manifest test in `integration_security_multitenancy`.
- [ ] T022-011 [P] Permissions and grants in `crates/domain/src/identity.rs` and `crates/application/src/authz.rs`. FR-022-032. Evidence: catalog of 51 entries; machine roles hold none.
- [ ] T022-012 [P] Error refinements in `crates/api-http/src/error.rs` and application errors per `contracts/errors.md`. FR-022-034. Evidence: one contract test per row; `sdks/error-catalog.json` codes unchanged.
- [ ] T022-013 Quota resources and ceilings in `crates/domain/src/quota.rs`, `crates/application/src/quotas.rs`, `crates/infra-postgres/src/quotas.rs`. FR-022-034. Evidence: at-bound and over-bound test per ceiling; zero refuses (SC-022-011).
- [ ] T022-014 [P] Metrics and spans in `crates/telemetry/src/{metrics.rs,span.rs}` with closed labels. FR-022-033. Evidence: telemetry tests reject unknown labels.

**Checkpoint**: stories may proceed in parallel; ADR-0056 recommends US2 first.

## Phase 3: US1 — Reuse a step's recent result (P1)

- [ ] T022-015 [US1] Pure cache decision (fingerprint, scope, outcome) in `crates/domain/src/execution/flow_control.rs`. FR-022-002..004. Evidence: property tests; different key expressions never share entries.
- [ ] T022-016 [US1] Hit in the readiness transaction (T1) under the run lock, taking no shard fence (worker completions hold none), with the hit provenance row: new `crates/application/src/step_cache.rs`, new `crates/infra-postgres/src/step_cache.rs`, `crates/infra-postgres/src/execution.rs`. FR-022-003,004,037. Evidence: no lease on hit when readiness comes from a worker completion and from a scheduler pass; barrier against invalidation both orders; `verify_run`.
- [ ] T022-017 [US1] Store obligation (T2) and leased writer (T3) in new `crates/scheduler/src/flow_control.rs`. FR-022-005. Evidence: stored, duplicate, quota-full, blob and backlog cases leave step and run unchanged.
- [ ] T022-018 [US1] Secret-usability recheck on hit via `crates/application/src/worker/secrets.rs`. FR-022-004. Evidence: revoked publisher turns a hit into a miss; no secret resolved.
- [ ] T022-019 [US1] Invalidation and listing routes in new `crates/api-http/src/routes/step_cache.rs`; purge in `crates/scheduler/src/passes.rs`. FR-022-006,007. Evidence: REST tests; audit without key values; byte release.
- [ ] T022-020 [P] [US1] Fault suite `tests/fault/flow-control-step-policies/cache.rs` (crash around T1, T2→T3, duplicate obligation, concurrent misses, lost hint). SC-022-001,002. Evidence: passes with `assert_consistent`.

## Phase 4: US2 — Limit concurrent work per business key (P1)

- [ ] T022-021 [US2] Permit model in `crates/domain/src/execution/{flow_control.rs,state.rs}` (queued wait, acquire, release, timeout, dequeue). FR-022-018,019,021. Evidence: domain replay tests.
- [ ] T022-022 [US2] Acquire or enqueue at admission (T10) through one permit port used by every run creator of the plan's table (start delay acquires at release; a reset acquires a fresh permit or is refused, never queues; restored events ignored): ports in new `crates/application/src/concurrency.rs`, use in `crates/application/src/runs.rs`, semaphores at slot 7 of ADR-0050's merged admission order, SQL in `crates/infra-postgres/src/execution.rs`; key error ends the run (D1180). FR-022-018,021. Evidence: API, event, cron and child starts; queue-full codes.
- [ ] T022-023 [US2] Release, shard-fenced grant (T11) and queue timeout (T12) in `crates/scheduler/src/flow_control.rs` and new `crates/infra-postgres/src/concurrency.rs` (ADR-0056), with skip-ahead for waiters their own limit cannot admit and, with 019, (019 priority 1 first, enqueue time asc) order among them (D1602). FR-022-018,020. Evidence: admission-order grants; older `limit: 1` waiter does not block a younger `limit: 5` waiter; fence rejects a stale replica.
- [ ] T022-024 [US2] Step permits at claim (T13) in `crates/infra-postgres/src/workers.rs` and release at attempt end (T14) in `crates/application/src/worker.rs`; step queue timeout. FR-022-019. Evidence: no attempt spent while waiting; timeout uses the failure route.
- [ ] T022-025 [US2] Cancellation and run-deadline dequeue, pause keeps permit, paused waiters skipped, lowered limits, same-key child and limit-mismatch warnings, active-run accounting and the namespace `queued_runs` ceiling. FR-022-021. Evidence: one integration test per item.
- [ ] T022-026 [US2] `Run.concurrency` in new `crates/api-http/src/dto_flow_control.rs`; `GET .../concurrency-keys` in new `crates/api-http/src/routes/concurrency.rs`. FR-022-022. Evidence: REST tests; raw-key canary absent.
- [ ] T022-027 [US2] Invariants 1–2 of `data-model.md` in `verify_run` (`crates/infra-postgres/src/execution.rs`) and `assert_consistent` (`tests/support/engine.rs`). FR-022-020. Evidence: fails on seeded inconsistency, passes after every fault case.
- [ ] T022-028 [P] [US2] Race and fault suite `tests/fault/flow-control-step-policies/concurrency.rs` (grant vs timeout ×20, crash at T10/T11/T12/T14, takeover duplicate grant, lost hint, restart). SC-022-003,004. Evidence: results recorded as ADR-0056 evidence.
- [ ] T022-029 [P] [US2] Lock-order barriers `tests/integration/flow-control-step-policies/concurrency_locks.rs` for each per-path order of ADR-0056 decision 2 (admission, grant, timeout, terminal, claim against attempt end, cancellation, policy edits). FR-022-019,020. Evidence: no deadlock; reversed-order control waits as expected.

## Phase 5: US3 — Handle a failed workflow (P1)

- [ ] T022-030 [US3] Handler phase in `crates/domain/src/execution/{flow_control.rs,sagas.rs,state.rs}` (after compensation, region or `onFailure~h` child, pending status, cancellation, `MANUAL_REVIEW` wait; no handler for a run that never acquired its run permit). FR-022-012..014. Evidence: domain tests per `on` status and for a timed-out waiter.
- [ ] T022-031 [US3] T7/T8 persistence, handler columns and failure-context builder in `crates/infra-postgres/src/execution.rs`. FR-022-013,014. Evidence: integration tests; invariant 4.
- [ ] T022-032 [US3] Namespace handler configuration (T17) in new `crates/application/src/failure_handlers.rs`, `crates/infra-postgres/src/failure_handlers.rs`, `crates/api-http/src/routes/failure_handlers.rs`. FR-022-015. Evidence: REST tests incl. revision conflict and audit.
- [ ] T022-033 [US3] Dispatch obligation and dispatcher (T9) in `crates/scheduler/src/flow_control.rs`; `DEAD`, redrive, alert in `crates/scheduler/src/monitor.rs` and `crates/application/src/operations.rs`. FR-022-016. Evidence: exactly one handler run per failure; loop prevention.
- [ ] T022-034 [P] [US3] Fault suite `tests/fault/flow-control-step-policies/handlers.rs` (crash at T7/T8/T9, duplicate dispatcher, failed compensation first, handler failure and timeout, cancelled runs). SC-022-005,006. Evidence: passes with consistency checks.

## Phase 6: US4 — Return a result early (P2)

- [ ] T022-035 [US4] T6 in the reducer and `crates/infra-postgres/src/execution.rs` (result once, `result_source`, `end.reached` after). FR-022-008,009,011. Evidence: replay; invariant 3.
- [ ] T022-036 [US4] Early disclosure in `crates/application/src/payload_disclosure.rs`; run fields in `crates/api-http/src/dto_flow_control.rs`; SSE in `crates/api-http/src/streams.rs`. FR-022-010. Evidence: REST tests; permissions unchanged.
- [ ] T022-037 [US4] `run.result_returned` deliveries and hint in `crates/application/src/outbound.rs` and `crates/infra-postgres/src/outbound.rs`. FR-022-010. Evidence: delivery written in T6; payload-free; none for test trees.
- [ ] T022-038 [P] [US4] Tests in `tests/{integration,fault}/flow-control-step-policies/early_return.rs` (before terminal, later failure/cancel, crash at T6, placement refusals). SC-022-007. Evidence: suites pass.

## Phase 7: US5 — Execute one step without a workflow (P2)

- [ ] T022-039 [US5] Shared private-definition retention from `crates/application/src/runs/ad_hoc.rs`; new `crates/application/src/runs/standalone.rs`. FR-022-023,024. Evidence: unit tests; `tests/integration/database-connections` unchanged.
- [ ] T022-040 [US5] `POST .../step-executions` in new `crates/api-http/src/routes/step_executions.rs`; `database-executions` on the shared path. FR-022-023..025. Evidence: REST tests; `GET /runs?origin=STANDALONE_STEP`.
- [ ] T022-041 [P] [US5] Tests in `tests/{integration,fault}/flow-control-step-policies/standalone.rs` (lost response, duplicate, changed body, cancel, worker crash, script and database kinds, `compensateWith`). SC-022-008. Evidence: suites pass.

## Phase 8: US6 — Test a draft with pins and mocks (P2)

- [ ] T022-042 [US6] Test admission (T16) in new `crates/application/src/runs/test_runs.rs` (revision, `prod` refusal, `startAt`, `MISSING_PIN`, `TEST_RUN_UNMOCKED_EFFECT` for unpinned 021/024/025/030 effect nodes when those features exist). FR-022-026..028. Evidence: one test per refusal.
- [ ] T022-043 [US6] Pins: payloads, `test_run_pins`, `step.mocked`, no lease, compensation exclusion, in `crates/infra-postgres/src/execution.rs`. FR-022-027. Evidence: zero leases; compensation skips mocked steps.
- [ ] T022-044 [US6] `fromRun` pins via `crates/application/src/payload_disclosure.rs` (secret refusal, pii grant and audit, floor). FR-022-029. Evidence: each case tested.
- [ ] T022-045 [US6] Test-tree isolation (children, deliveries, triggers, dispatch, cache bypass, `DRAFT_TEST` semaphore pool, quotas, secrets per O1162); contract test with 018 that test-tree runs are refused as reset, rerun and batch sources (`RESET_SOURCE_TEST_RUN`). FR-022-030. Evidence: one test per rule; invariant 7 of `data-model.md`.
- [ ] T022-046 [US6] `POST /workflows/{workflowId}/test-runs` in new `crates/api-http/src/routes/test_runs.rs`. FR-022-026. Evidence: REST tests incl. replay.
- [ ] T022-047 [P] [US6] Tests in `tests/{fault,integration}/flow-control-step-policies/test_runs.rs` (crash at T16, draft edit race, `fromRun` tenant isolation). SC-022-009. Evidence: suites pass.

## Phase 9: SDKs, Studio, CLI, documentation and gates

- [ ] T022-048 [P] TypeScript in `sdks/typescript/src/{workflow.ts,client.ts}`. FR-022-035. Evidence: unit tests; canonical fixture byte-identical.
- [ ] T022-049 [P] Go in `sdks/go/workflow/workflow.go`, `sdks/go/client/runs.go`. FR-022-035. Evidence: as T022-048.
- [ ] T022-050 [P] Python in `sdks/python/sagat_octopus_orchestrator/{workflow.py,client.py}`. FR-022-035. Evidence: as T022-048.
- [ ] T022-051 [P] C# in `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/{Workflows/Flow.cs,SagatClient.cs}`. FR-022-035. Evidence: as T022-048.
- [ ] T022-052 [P] Java in `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/{workflow/WorkflowBuilder.java,SagatClient.java}`. FR-022-035. Evidence: as T022-048.
- [ ] T022-053 Six scenarios of `contracts/conformance.md` in `tests/contract/sdk-conformance/scenarios/` with reviewed goldens. SC-022-012. Evidence: `sdk_conformance -- --ignored` passes in five languages.
- [ ] T022-054 Studio authoring in `frontend/apps/studio/src/features/workflows/` (`NodeInspector.tsx`, new `FlowControlFields.tsx`, `canvasModel.ts`, `WorkflowSettings.tsx`) and the draft test panel. FR-022-035. Evidence: Vitest; same IR as the SDKs.
- [ ] T022-055 Studio run views in `frontend/apps/studio/src/features/runs/` (hit/mock badges, early result, handler phase, waits; unknown as unsupported, FR-007-010). FR-022-035. Evidence: Vitest and browser journeys.
- [ ] T022-056 Studio operations in `frontend/apps/studio/src/features/operations/` (keys, cache entries, invalidation, handler, dispatches) and standalone form; WCAG AA, table alternatives. FR-022-022,025,035. Evidence: browser journeys incl. keyboard.
- [ ] T022-057 [P] CLI in `apps/cli/src/` (`sagat step run`, `sagat workflow test`, `sagat cache invalidate`, `sagat namespace failure-handler`, `--await-result`, `--origin`). FR-022-025,026. Evidence: CLI tests; `contract_developer_experience`.
- [ ] T022-058 New routes in `tests/contract/security-multitenancy/rest/battery.rs`; raw-key, value, pin and failure-text canaries. SC-022-010; NFR-022-002. Evidence: battery and canaries pass.
- [ ] T022-059 Restart e2e `tests/e2e/flow-control-step-policies/restart.rs` (queued runs, pending obligations and dispatches, handler in progress). NFR-022-001. Evidence: passes with consistency checks.
- [ ] T022-060 Merge fragments into `specs/001-platform-foundation/contracts/{openapi.yaml,workflow.schema.json,events.asyncapi.yaml}`; regenerate `events.baseline.json` and the Studio client. FR-022-010,031. Evidence: contract lint and compatibility; `npm run check:api`.
- [ ] T022-061 [P] Runbook `docs/runbooks/flow-control.md` (alerts, digest key and G1163 rotation, redrive, stuck queues); updates to `docs/runbooks/alerts.md` and `docs/architecture/{execution-semantics.md,workflow-ir-and-sdk-modes.md,observability-slo.md}`. FR-022-033; NFR-022-005. Evidence: reviewed docs.
- [ ] T022-062 [P] Benchmark `tests/performance/flow-control-step-policies/` and report `tests/performance/reports/flow-control-step-policies.md`. SC-022-014; NFR-022-004; G1167. Evidence: report with environment, raw data and failures.
- [ ] T022-063 Erasure interplay with 020 (T18): delete entries by `source_run_id` and the run's obligations, keep hit provenance and pins discoverable by source, register `step_cache_entries`, `step_cache_hit_provenance` and pin provenance in 020's content-store registry `crates/infra-postgres/run-content-references.json` (hook: invalidate by `source_run_id`) and classify every column in `storage-classification.json`. FR-022-037; D1199. Evidence: SC-022-016 test against 020's manifest-driven reference check; next lookup misses after erasure.
- [ ] T022-064 Run-creator conformance `tests/integration/flow-control-step-policies/run_creators.rs`: one case per creator of the plan's Run creators table (with 018 for schedule, batch, reset, Signal/Update-with-Start, start delay), failing when a creator bypasses the permit port; plus the 018 reset corpus for every rule of the reset-rule table (refused handler-phase points, fresh permit or `CONCURRENCY_PERMIT_UNAVAILABLE`, provenance and result re-materialized, no inherited `handler_for`). FR-022-018,020,038; D1198, D1600. Evidence: SC-022-015.
- [ ] T022-065 Final gates (format, clippy, workspace tests, `scripts/check-architecture.py`, contracts, runtime-role sweep, SDK lints, compose smoke, quickstart demos); record results, skips and gaps; update statuses. All FR and SC. Evidence: verification record with commands and exits.

## Dependencies and execution order

- Phase 1 → Phase 2 → stories. T022-009 precedes persistence tasks, T022-008 reducer tasks, T022-005/006
  publication tests.
- Stories are independent after Phase 2; recommended order US2, US1, US3, US4, US5, US6. T022-044 uses the
  disclosure path extended by T022-036.
- SDK tasks start after T022-006; T022-053 needs the stories it exercises; Studio follows each story's API;
  T022-060 precedes T022-065. T022-063 needs 020's erasure port and T022-064 needs 018's creators; each
  case is enabled when its owner feature lands, and the missing ones are reported as pending, not passed.

## Parallel examples

- After T022-009: T022-010, T022-011, T022-012, T022-014.
- US2: T022-028 and T022-029 once T022-023 and T022-024 exist.
- Phase 9: T022-048..052; T022-057, T022-061, T022-062.

## Release 0.3.0 slice: US02 and US07

Added on 2026-11-13 for OD-2026-11-13-11 (D022-001). The task texts above are not edited (the feature was
approved by OD-2026-11-11-1; after approval only a checkbox, an evidence block and follow-up tasks
change). This section says which of them form the slice, how each is narrowed for it, and adds the
follow-up tasks T022-066..090. A narrowed task is ticked for the slice only with the evidence of its
narrowed scope, written under it with the date and the words "0.3.0 slice"; it stays open for the rest.
Story tag: US7 rate limits. 63 tasks: 38 existing, 25 new.

### Existing tasks in the slice

| Task | In the slice as | Narrowed to |
|---|---|---|
| T022-001 | Owner review | replaced for the slice by T022-066 (ADR-0056, ADR-0084, O1163–O1168); O1160–O1162 stay with their stories |
| T022-002 | Fixtures | the concurrency and rate-limit fixtures and one invalid document per rule of `contracts/errors.md` that names them |
| T022-003 | Test targets | as written |
| T022-004 | Domain policies | `ConcurrencyPolicy` and its combination rules; `RateLimitPolicy` is T022-068 |
| T022-005 | Capability gates | `concurrencyKeys`; `rateLimits` is T022-069 |
| T022-006 | IR codec | `concurrency` (top level and node); `rateLimit` is T022-069 |
| T022-007 | Keyed digests | as written; rotation refusal also covers rate rows (T022-070) |
| T022-008 | Journal events | the five `concurrency.*` events; `rate_limit.*` is T022-068 |
| T022-009 | Migration | the four concurrency tables only; no `runs` column, no origin value; rate tables are T022-070 |
| T022-010 | Classification | the concurrency tables |
| T022-012 | Errors | the rows of `contracts/errors.md` that name concurrency |
| T022-013 | Quotas | the concurrency rows of the ceilings table and `queued_runs` |
| T022-014 | Metrics and spans | `concurrency_*` and `workflow.concurrency.*` |
| T022-021..029 | US02 | as written; T022-022's creators are those that exist on the tree (plan, slice table) |
| T022-048..052 | Five SDKs | the `concurrency` builder only; Rust is T022-067 |
| T022-053 | Conformance | `flow-concurrency` only, in six languages |
| T022-054..056 | Studio | `concurrency` in the settings and the node inspector, the waiting state on a run, the keys list |
| T022-058 | Battery and canaries | `GET .../concurrency-keys` and the raw-key canary |
| T022-059 | Restart e2e | queued runs and waiting steps |
| T022-060 | Contract merge | the concurrency parts of the IR and OpenAPI fragments |
| T022-061 | Runbook | the queue, the digest key and `concurrency_queue_stuck` |
| T022-062 | Benchmark | permit acquisition with 1, 16 and 256 contenders and the grant delay |
| T022-064 | Run creators | the creators that exist; 018's cases when its branch is on the tree, else reported pending |
| T022-065 | Final gates | as written, for the slice |

Not in the slice: T022-011 (no new permission is needed), T022-015..020 (US1), T022-030..034 (US3),
T022-035..038 (US4), T022-039..041 (US5), T022-042..047 (US6), T022-057 (no CLI command of the slice),
T022-063 (020 erasure).

### Phase 10: follow-up tasks of the slice (T022-066..090)

- [ ] T022-066 Owner gate for the slice: OD-2026-11-13-11 present in `docs/owner-decisions.md`; answers to O1163–O1168 recorded there and in `research.md`; ADR-0056 and ADR-0084 accepted or changed by the owner. FR-022-017..022,039..050. Owner-gate: every task below that builds behaviour. Evidence: the recorded answers; ADR status set by a person.
- [ ] T022-067 [P] [US2] Rust SDK `concurrency` builder in `sdks/rust` (the sixth SDK, D022-016). FR-022-035,050. Evidence: unit tests; canonical fixture byte-identical to the other five.
- [ ] T022-068 [US7] `RateLimitPolicy` and its validation in `crates/domain/src/workflow/flow_control.rs`; events `rate_limit.queued/granted/wait_timed_out/dequeued`, wait kind `RATE_LIMIT` for runs and steps and their reducer rules in `crates/domain/src/execution/{event.rs,state.rs,flow_control.rs}`. FR-022-039,045. Evidence: unit and property tests; one replay test per event of `contracts/journal-events.md`; old definitions and journals unchanged.
- [ ] T022-069 [US7] IR codec for `rateLimit` and the `rateLimits` gate in `crates/infra-ir/src/{flow_control.rs,convert.rs}`, with `RATE_LIMIT_POLICY_INVALID`, `SCOPED_RATE_LIMIT`, `FLOW_CONTROL_COMBINATION` and the `RATE_LIMIT_MISMATCH` warning. FR-022-039,050; SC-022-013. Evidence: contract test over the fixture corpus of T022-002; previous corpus unchanged with the gate on and off; a disabled gate refuses before a version exists.
- [ ] T022-070 [US7] Additive migration for `rate_limit_windows`, `rate_limit_grants`, `rate_limit_waiters` (`data-model.md`) with `sagat_app` grants, a refusing down migration and entries in `storage-classification.json`; the digest key rotation refusal of T022-007 extended to them. FR-022-040. Evidence: up/down test; runtime-privilege sweep; manifest test.
- [ ] T022-071 [US7] The pure window decision in `crates/domain/src/execution/flow_control.rs`: which grants count for an acquirer (run scope from the grant, step scope from the lease or while unleased), admit or wait, position and estimate, as a function of instants passed in. FR-022-040,048; D022-003,005,014. Evidence: property tests that no sequence of decisions puts more than `limit` counted instants in any window, for mixed acquirer limits and windows; clock-backwards cases.
- [ ] T022-072 [US7] Grant or buffer at admission (T19) through the one admission-gate port, for every run creator on the tree; permit and slot together when both are declared; queue-full and key-error outcomes; start delay evaluated at release. `crates/application/src/{rate_limits.rs,concurrency.rs,runs.rs}`, `crates/infra-postgres/src/{rate_limits.rs,execution.rs}`. FR-022-041,042,044,047. Evidence: integration tests for API, event, cron and child starts; `RATE_LIMIT_QUEUE_FULL`; a run never commits with a permit and no slot or the reverse.
- [ ] T022-073 [US7] Grant pass (T20) and queue timeout (T21) in `crates/scheduler/src/flow_control.rs` and `crates/infra-postgres/src/rate_limits.rs`: shard fence, oldest admitted waiter first with skip-ahead, bounded grants per key and tenant rotation per pass, first commit wins against the timer. FR-022-041,042,047; ADR-0084. Evidence: admission-order grants; a fence rejects a stale replica; a waiter of another key and tenant is granted during a flood within two passes.
- [ ] T022-074 [US7] Step slots: ask at readiness (T22) for the first attempt and each retry, step `WAITING` (`RATE_LIMIT`) until granted, lease recording and slot return by the pass (T23), step queue timeout through the failure route. `crates/infra-postgres/src/{rate_limits.rs,execution.rs}`, `crates/scheduler/src/flow_control.rs`; `crates/infra-postgres/src/workers.rs` is not edited. FR-022-040,043; D022-005,008. Evidence: no attempt spent while waiting; with no worker for three windows at most `limit` grants are unleased; a returning worker leases at most `limit` in its first window (SC-022-021).
- [ ] T022-075 [US7] Lifecycle (T24): cancellation and run deadline of a waiter, pause skips a waiter, a lowered declaration and a lowered policy ceiling, a granted step cancelled before its lease, node-level `concurrency` with node-level `rateLimit`, a body `child` node starting a rate-limited workflow. FR-022-044,046. Evidence: one integration test per item (SC-022-022).
- [ ] T022-076 [US7] Hot-path budget: the budget of `tests/integration/claim-transition-efficiency/work_budget.rs` unchanged for a definition without the new fields; a new line for the claim of a rate-limited step equal to the plain claim; recorded lines for a completion whose next step declares `rateLimit` and for an admission that declares it. NFR-022-007; FR-038-001. Evidence: `integration_claim_transition_efficiency` passes; seen failing first with a statement added to the claim by hand. Lands before any call site in a completion.
- [ ] T022-077 [US7] Ceilings of the plan's rate-limit table in `crates/domain/src/quota.rs`, `crates/application/src/quotas.rs`, `crates/infra-postgres/src/quotas.rs`; buffered runs in `active_runs`, `tenant_active_runs` and `queued_runs`. FR-022-047; FR-022-034. Evidence: an at-bound and an over-bound test per ceiling; zero refuses.
- [ ] T022-078 [US7] `Run.rateLimit`, the step's waiting state and `GET /namespaces/{namespaceId}/rate-limit-keys` in `crates/api-http/src/{dto_flow_control.rs,routes/rate_limits.rs}`; position and estimate computed at read. FR-022-048. Evidence: REST tests; the estimate is absent in the cases of D022-014; raw-key canary absent.
- [ ] T022-079 [US7] Invariants 8–10 of `data-model.md` in `verify_run` (`crates/infra-postgres/src/execution.rs`) and `assert_consistent` (`tests/support/engine.rs`), with the journal window check. FR-022-040,045. Evidence: fails on a seeded extra grant and on a waiter without its event; passes after every fault case.
- [ ] T022-080 [P] [US7] Fault and race suite `tests/fault/flow-control-step-policies/rate_limits.rs`: grant versus timeout and grant versus cancel (20 rounds each, barriers, no sleeps), a crash at each commit point of T19–T24, a duplicate grant across a shard takeover, a lost hint, a restart after several windows, a flood of 1,000 starts on one key, the limit lowered while runs wait, replicas with offset process clocks, the database clock set back. SC-022-017,018,022,023; NFR-022-001,008. Evidence: results recorded as ADR-0084 evidence.
- [ ] T022-081 [P] [US7] Lock-order barriers `tests/integration/flow-control-step-policies/rate_limit_locks.rs` for each path of ADR-0084 decision 4 (admission, grant, timeout, readiness, lease recording, cancellation, purge), alone and with a semaphore. FR-022-044; SC-022-019. Evidence: no deadlock; the reversed-order control waits as expected.
- [ ] T022-082 [P] [US7] Metrics and spans of the plan in `crates/telemetry/src/{metrics.rs,span.rs}`; alerts `rate_limit_grant_lagging` and `rate_limit_queue_saturated` in `crates/scheduler/src/monitor.rs`. FR-022-049. Evidence: telemetry tests reject unknown labels; each alert raised and cleared in a test.
- [ ] T022-083 [P] [US7] `rateLimit` builder in the six SDKs (`sdks/typescript/src/workflow.ts`, `sdks/go/workflow/workflow.go`, `sdks/python/sagat_octopus_orchestrator/workflow.py`, `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/Flow.cs`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/WorkflowBuilder.java`, `sdks/rust`). FR-022-050. Evidence: unit tests and each SDK's lints; canonical fixture byte-identical in six languages.
- [ ] T022-084 [US7] Scenario `flow-rate-limit` of `contracts/conformance.md` in `tests/contract/sdk-conformance/scenarios/` with independent assertions and reviewed goldens. SC-022-023. Evidence: `SAGAT_SDK_SCENARIO=flow-rate-limit cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` passes in six languages, rerun without the bless switch.
- [ ] T022-085 [US7] Studio: `rateLimit` fields beside `concurrency` in `frontend/apps/studio/src/features/workflows/` (settings and node inspector), a run or step waiting for a rate slot with position and estimate in `features/runs/`, the rate-limit keys list in `features/operations/`; unknown capability shown as unsupported (FR-007-010); WCAG AA. FR-022-048,050. Evidence: Vitest; a browser journey including keyboard; same IR as the SDKs.
- [ ] T022-086 [US7] Run-creator cases of T022-064 extended to the rate gate, failing when a creator bypasses the port; with 018 on the tree, its creators, the reset refusal `RATE_LIMIT_SLOT_UNAVAILABLE` and the reset rules of the four `rate_limit.*` events registered in 018's registry (G1181). FR-022-041,046; FR-022-038. Evidence: `tests/integration/flow-control-step-policies/run_creators.rs`; the 018 cases reported pending while its branch is not merged.
- [ ] T022-087 [US7] Merge the rate-limit parts of the fragments into `specs/001-platform-foundation/contracts/{openapi.yaml,workflow.schema.json}` and regenerate the Studio client; the OpenAPI baseline is changed only after reading the diff. FR-022-039,048. Evidence: contract lint and compatibility tests; `(cd frontend/apps/studio && npm run api && npm run check:api)`.
- [ ] T022-088 [P] [US7] Rate-limit sections of `docs/runbooks/flow-control.md` (the two alerts, a stuck or saturated key, the clock, lowering a limit) and `docs/runbooks/alerts.md`; `docs/architecture/execution-semantics.md`; `docs/traceability.md`; the release notes of 0.3.0; the quickstart's section 7 run as a demo of the happy and the failure scenario. FR-022-049; NFR-022-005. Evidence: reviewed docs; `contract_developer_experience` without the bless switch.
- [ ] T022-089 [P] [US7] Benchmark `tests/performance/flow-control-step-policies/` for rate limits: grant lag at 1, 16 and 256 waiters on one key, one key with 10,000 waiters, 1,000 keys with one waiter each, admission cost with and without the declaration. NFR-022-004; G1174, G1178. Evidence: report with environment, raw data and failures; no figure presented as a commitment.
- [ ] T022-090 [P] [US7] `GET .../rate-limit-keys` and the new run fields in `tests/contract/security-multitenancy/rest/battery.rs`; raw-key canaries in logs, spans, metrics and listings. SC-022-023; NFR-022-002. Evidence: battery and canaries pass, also as the runtime role.

### Order of the slice

- T022-066 first. Then T022-002, 003; then T022-004..010, 012..014 (narrowed) and T022-068..070.
- US02: T022-021 → 022 → 023 → 024 → 025..027, with T022-028 and T022-029 once 023 and 024 exist.
- US07 after T022-022 (it extends the same port): T022-071 → 072 → 073 → 076 → 074 → 075, 077..079, with
  T022-080 and T022-081 once 073 and 074 exist. T022-076 precedes 074's call site in a completion.
- Surfaces after the story they show: T022-048..052, 067, 083 after the codec; T022-053, 084 after the
  stories; T022-054..056, 085 after T022-026 and 078; T022-060, 087 before T022-065.
- Parallel: T022-067 and 083 per language; T022-082, 088, 089, 090.
