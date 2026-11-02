# Tasks: Schedules and bulk run operations

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md).

Status: Proposed — no task is started. A task is checked only when its named evidence exists and was
run; skips and failures are recorded (constitution VIII). Tests are mandatory (AGENTS.md rule 9) and
are written failing before each story's implementation.

Format: `- [ ] T018-NNN [P?] [USn?] Description — files — evidence — traces`.

## Phase 1: Setup

- [ ] T018-001 Owner review of ADR-0050 and O1000–O1002; re-run the plan's Constitution Check; record
  approved dependencies — `docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md`, `research.md`, `plan.md` — ADR status, recorded
  answers — FR-018-001..052.
- [ ] T018-002 [P] Register targets `integration_`, `fault_`, `contract_`, `e2e_schedules_bulk_operations`
  — `tests/Cargo.toml`, `tests/{integration,fault,contract,e2e}/schedules-bulk-operations/main.rs` —
  targets compile — NFR-018-001.

## Phase 2: Foundational (blocks every story)

- [ ] T018-003 Domain schedules: bounded spec/policy types, outcomes, pure `decide` (overlap, catch-up,
  remaining actions, tracking limit), deterministic jitter — `crates/domain/src/schedules.rs` — unit and
  property tests — FR-018-002..008, FR-018-012.
- [ ] T018-004 [P] Domain business ids: `BusinessId` (1–200 visible ASCII), policies, pure `admit`
  — `crates/domain/src/business_ids.rs` — policy × holder × window table — FR-018-018..020.
- [ ] T018-005 [P] Domain batch: selector, action set, item/job states, milli-item rate slot —
  `crates/domain/src/batch.rs` — state-machine tests — FR-018-027..034.
- [ ] T018-006 [P] Domain reset and events `run.start_delayed`, `run.start_released`, `run.reset`,
  `wait.satisfied.via`; eligibility over `RunState::replay` + suffix facts; `ResetPlan` —
  `crates/domain/src/execution/{reset.rs,event.rs,mod.rs}` — per-cause tests — FR-018-023, FR-018-035..039.
- [ ] T018-007 Permissions, audit actions and quota resources of the plan —
  `crates/domain/src/{identity.rs,audit.rs,quota.rs}` — role-grant and range tests — FR-018-041, FR-018-045.
- [ ] T018-008 Additive migrations (tables incl. `schedule_actions` PK with `origin`, `batch_jobs.selection_horizon xid8`,
  `schedule_trigger_handover`, and the instance-capability registry tables unless 029 or 031 shipped them; `runs` columns with the erasure-aware
  immutability trigger; `run_shard` partial indexes; ALWAYS triggers; grants; trigger-owned backfill;
  data-loss-refusing down) — `crates/infra-postgres/migrations/<timestamp>_schedules_bulk_operations.{up,down}.sql`
  — forward/rollback, backfill and erasure-trigger tests — FR-018-001, FR-018-017, FR-018-021, FR-018-046.
- [ ] T018-009 Classify new variable-size columns; register payload-referencing columns for 020's
  reference check; extend the runtime-privilege test — `crates/infra-postgres/storage-classification.json`,
  `tests/integration/platform-foundation/runtime_privileges.rs` — manifest test, `SAGAT_TEST_AS_RUNTIME_ROLE=1`
  — FR-018-042, FR-018-045.
- [ ] T018-010 Merge the REST fragment, AsyncAPI additions (intentional baseline regeneration) and
  reasons; assert `workflow.schema.json` and `worker.baseline.json` unchanged —
  `specs/001-platform-foundation/contracts/{openapi.yaml,events.asyncapi.yaml,events.baseline.json}` —
  `contract_schedules_bulk_operations` — FR-018-043, FR-018-046.
- [ ] T018-011 Ports `ScheduleStore`, `ScheduleCalendar`, `BusinessIdStore`, `BatchStore`, `ResetStore`;
  `NewRun` admissions `schedule`, `business_id`, `delay`, `reset` processed by `RunStore::create` in
  ADR-0050 order — `crates/application/src/{schedules.rs,batch.rs,runs.rs,runs/business_id.rs,runs/reset.rs}`
  — validation unit tests — FR-018-005, FR-018-021.
- [ ] T018-012 Lock-order interleaving harness (barriers, deadlock counter) for the merged admission slots
  of ADR-0050 decision 7 and both aggregate orders (aggregate → run; run → bookkeeping row), with
  slot-order cases for the admission paths of 019, 021, 022, 024, 025, 026 and 030 as they merge —
  `tests/support/lock_order.rs` — self-test reproduces an opposite-order deadlock and passes the slot
  order — NFR-018-001.

## Phase 3: US01 — Run a workflow on a schedule (P1, MVP)

- [ ] T018-013 [P] [US1] `ScheduleCalendar` adapter (calendars, 004 cron grammar shared with `cron.rs`,
  intervals, exclusions, bounds, DST, next-N, spacing over 1,000) — `crates/infra-ir/src/schedule_calendar.rs`
  — ≥ 200-case table incl. `America/New_York`, `Europe/Lisbon` (SC-018-001) — FR-018-002, FR-018-003.
- [ ] T018-014 [US1] PostgreSQL `ScheduleStore` (revisions, head CAS, due scan by `(shard, next_wake_at)`
  bounded by `now()`, actions, purge) — `crates/infra-postgres/src/schedules.rs` —
  `tests/integration/schedules-bulk-operations/store.rs` — FR-018-001, FR-018-013, FR-018-014.
- [ ] T018-015 [US1] Schedule admission inside `RunStore::create` (fence, CAS, action + state after the
  initial reduction, refusal commits nothing) — `crates/infra-postgres/src/{execution.rs,schedules.rs}`
  — one-commit content test — FR-018-004, FR-018-005.
- [ ] T018-016 [US1] `ScheduleService` and `ScheduleFiring` (version policy, input check at action,
  `nominalTimeField`, business-id prefix, retry backoff) — `crates/application/src/schedules.rs` —
  tests over the real store — FR-018-001, FR-018-005, FR-018-016.
- [ ] T018-017 [US1] Schedule pass (owned shards, fairness, round ceiling, metrics) and engine wiring —
  `crates/scheduler/src/{schedules.rs,lib.rs,monitor.rs}`, `apps/execution-engine/src/lib.rs` — two-replica
  pass tests — FR-018-005, FR-018-044.
- [ ] T018-018 [US1] REST create/read/list/update/delete — `crates/api-http/src/routes/{schedules.rs,mod.rs}`,
  `crates/api-http/src/dto_schedules.rs` — contract tests — FR-018-001, FR-018-013, FR-018-043.
- [ ] T018-019 [P] [US1] US01 scenarios 1–5 — `tests/integration/schedules-bulk-operations/schedules.rs`
  — target output — FR-018-001..005, FR-018-016.
- [ ] T018-020 [US1] Faults: crash after each statement of transaction 2, stale fence, duplicate pass,
  lost hint, restart, skewed process clocks — `tests/fault/schedules-bulk-operations/firing.rs` — one run
  and one action per time, `verify_run` (SC-018-002) — FR-018-004, FR-018-005; NFR-018-001.
- [ ] T018-021 [US1] Authority and provenance (recheck under membership lock order, auto-pause + audit,
  `system:schedule:<id>`, 017 database authority, key replay) —
  `tests/integration/schedules-bulk-operations/authority.rs` — SC-018-004 — FR-018-015, FR-018-016, FR-018-041.
- [ ] T018-022 [US1] Schedule ceilings and quotas (count incl. zero, entries, spacing, input bytes,
  round size) — `tests/integration/schedules-bulk-operations/limits.rs` — one test per row —
  FR-018-003, FR-018-045; NFR-018-003.

## Phase 4: US02 — Operate a schedule (P1)

- [ ] T018-023 [US2] Pause/resume with note and audit — `crates/application/src/schedules.rs`,
  `crates/api-http/src/routes/schedules.rs` — paused times not caught up — FR-018-009.
- [ ] T018-024 [US2] Trigger-now (origin `MANUAL`, key `manual-{µs}`, paused, overlap override, replay, no
  decrement, no collision with a scheduled time at the same instant) —
  `tests/integration/schedules-bulk-operations/trigger.rs` — FR-018-004, FR-018-010.
- [ ] T018-025 [US2] Backfill create/list/cancel and incremental pass steps with cursor, `ALREADY_ACTED`
  — `crates/scheduler/src/schedules.rs`, `crates/infra-postgres/src/schedules.rs` — crash resumes from
  cursor — FR-018-011.
- [ ] T018-026 [US2] `SKIP`, `BUFFER_ONE`, `BUFFER_ALL`, `ALLOW_ALL`, open-run tracking, buffered start —
  `tests/integration/schedules-bulk-operations/overlap.rs` — SC-018-003 table — FR-018-006, FR-018-007.
- [ ] T018-027 [US2] Remaining actions and `pauseOnFailure` — US02 scenario 6 — FR-018-007, FR-018-012.
- [ ] T018-028 [US2] Describe (next N, recent, counters), actions route, action purge keeping `BUFFERED`
  — `crates/infra-postgres/src/maintenance.rs` — contract + purge tests — FR-018-013.
- [ ] T018-029 [US2] Catch-up after a simulated outage (`ONE`, `ALL`, summary, trigger-owned 366 d) —
  `tests/integration/schedules-bulk-operations/catchup.rs` — SC-018-003 — FR-018-008.
- [ ] T018-030 [US2] `CANCEL_OTHER` obligations via the ordinary cancel use case; races with self
  completion and with failed compensation — `tests/fault/schedules-bulk-operations/cancel_other.rs` —
  first commit wins — FR-018-006, FR-018-007.
- [ ] T018-031 [US2] Revisions on update; delete → `DROPPED_DELETED`, name freed —
  `tests/integration/schedules-bulk-operations/revisions.rs` — FR-018-014.
- [ ] T018-032 [P] [US2] Studio schedules (list, detail, form with server-computed next times,
  pause/resume, trigger, backfill preview; tokens, WCAG AA, tables) —
  `frontend/apps/studio/src/features/schedules/`, `frontend/apps/studio/src/api/endpoints.ts` —
  component tests — FR-018-047.
- [ ] T018-033 [P] [US2] Schedule input disclosure (016 audiences, `no-store`, audit) —
  `tests/integration/schedules-bulk-operations/disclosure.rs` — per-level cases — FR-018-042.

## Phase 5: US03 — Business identifiers and start delay (P1)

- [ ] T018-034 [US3] `BusinessIdStore` (insert-or-lock, holder status, claim in creation transaction,
  lookup, `runs.business_id`, purge) — `crates/infra-postgres/src/{business_ids.rs,execution.rs}` —
  store tests — FR-018-018, FR-018-021, FR-018-022.
- [ ] T018-035 [US3] Start options on `POST /runs` (`200` for `USE_EXISTING`), binding lookup, list
  filters, rerun `KEEP`/`DROP` — `crates/application/src/runs/business_id.rs`,
  `crates/api-http/src/routes/execution.rs`, `crates/api-http/src/dto.rs` — contract tests —
  FR-018-019, FR-018-020, FR-018-022, FR-018-040.
- [ ] T018-036 [US3] 32-way race, policy table in/after window, terminal-commit race, chain continuity,
  key replay/reuse, harness vs signal paths — `tests/{integration,fault}/schedules-bulk-operations/business_ids.rs`
  — SC-018-006 — FR-018-018..021.
- [ ] T018-037 [US3] Start delay (events, fenced release pass with `apply_if`, cancel/deadline, signal
  refused, Update admitted, quota; with 022 run-level concurrency the release acquires the permit or
  enqueues the run, FR-022-018) — `crates/scheduler/src/passes.rs`, `crates/infra-postgres/src/execution.rs`,
  `tests/{integration,fault}/schedules-bulk-operations/delay.rs` — SC-018-007 — FR-018-023, FR-018-050.
- [ ] T018-038 [P] [US3] Studio business id, start fields, delayed status —
  `frontend/apps/studio/src/features/runs/` — component tests — FR-018-047.
- [ ] T018-039 [US3] Business-id canaries absent from logs, spans, metrics, audit reasons, outbox —
  `tests/integration/schedules-bulk-operations/privacy.rs` — SC-018-014 — FR-018-042.

## Phase 6: US04 — Signal-with-Start and Update-with-Start (P2)

- [ ] T018-040 [US4] Signal-with-Start (holder signal, creation delivery, full rollback incl. a new run 022
  would queue, delay refused, timeout race, no stored signal without a waiting step) —
  `crates/application/src/runs/with_start.rs`, `crates/api-http/src/routes/execution.rs`,
  `tests/integration/schedules-bulk-operations/with_start.rs` — SC-018-008 — FR-018-024, FR-018-050.
- [ ] T018-041 [US4] Update-with-Start (`USE_EXISTING`/`FAIL`, ADR-0031 receipts, `REJECTED` keeps the
  run, nothing on pre-admission failure, `no-store`) — same files — US04 cases + replay — FR-018-025.
- [ ] T018-042 [US4] With-start idempotency and authorization (fingerprint, replay after current auth,
  cross-tenant `NOT_FOUND`) — `tests/integration/schedules-bulk-operations/with_start.rs` — FR-018-026, FR-018-041.
- [ ] T018-043 [US4] Write `via` on `wait.satisfied`; retain Update arguments on `state.updated` only for
  Updates declared `reapplyOnReset` — `crates/application/src/{runs.rs,chain_progress.rs,declared_operations.rs}`,
  `crates/scheduler/src/events.rs`, `crates/infra-postgres/src/event_codec.rs` — old/new codec round-trip,
  no payload written for undeclared Updates, content charge — FR-018-039.
- [ ] T018-044 [US4] Error reasons and remediation rows — `crates/api-http/src/error.rs`,
  `docs/quickstarts/errors.md` — `errors-018` contract test — FR-018-043.

## Phase 7: US05 — Batch operations (P2)

- [ ] T018-045 [US5] Batch store, service and routes (create, preview, read, list, items, cancel) —
  `crates/infra-postgres/src/batch.rs`, `crates/application/src/batch.rs`,
  `crates/api-http/src/routes/batch.rs`, `crates/api-http/src/dto_batch.rs` — contract/store tests —
  FR-018-027, FR-018-028, FR-018-032.
- [ ] T018-046 [US5] Executor pass (horizon wait on `pg_snapshot_xmin`, keyset selection rounds, item order
  run → fence → job → item, rate slot, deferral, drain) with kill/restart, duplicate, lost-hint and
  late-commit faults — `crates/scheduler/src/batches.rs`, `tests/fault/schedules-bulk-operations/batch.rs`
  — SC-018-009 — FR-018-029..031, FR-018-033.
- [ ] T018-047 [US5] Item adapters for the eight actions through single-run use cases, skip reasons (incl.
  `TEST_RUN`), failed compensation of cancelled runs — `tests/integration/schedules-bulk-operations/batch_actions.rs`
  — one case per action and reason — FR-018-030, FR-018-031, FR-018-034, FR-018-050.
- [ ] T018-048 [US5] Batch authorization and ceilings (no `run:batch`, missing action permission,
  mid-job revocation, foreign ids, job/item ceilings, zero quota) —
  `tests/integration/schedules-bulk-operations/batch_security.rs` — SC-018-010 — FR-018-033, FR-018-034,
  FR-018-041, FR-018-045.
- [ ] T018-049 [US5] Lock monitor (`pg_locks` sampling: never two run locks) and harness interleaving with
  API cancel/signal — `tests/fault/schedules-bulk-operations/batch_locks.rs` — zero deadlocks —
  FR-018-030; NFR-018-001.
- [ ] T018-050 [P] [US5] Studio batch (from run-list filter, preview, confirm with reason, progress,
  items, cancel) — `frontend/apps/studio/src/features/batch/`, `frontend/apps/studio/src/features/runs/runList.ts`
  — component tests — FR-018-047.
- [ ] T018-051 [US5] Job/item purge; selection through 020's `RunSelector`
  (`specs/020-visibility-analytics-archival/contracts/list-query.md`) when enabled, `query` refused
  `QUERY_UNSUPPORTED_OPERATOR` while it is off; `listRuns` filters mapped to 020's `BusinessId`/`ScheduleId`
  system attributes; selection through the current authorization evaluator (029-ready) —
  `crates/infra-postgres/src/maintenance.rs`, `crates/application/src/batch.rs` — purge, mapping and
  refusal tests — FR-018-022, FR-018-027, FR-018-033.

## Phase 8: US06 — Reset to a point (P2)

- [ ] T018-052 [US6] Reset-rule registry as an exhaustive table keyed by `RunEvent::name()` (test fails on a
  type without a rule; `RESET_RULE_MISSING`), and eligibility/plan over a seeded corpus (sequential,
  parallel, saga, approval, collection, loop, child, unknown outcome, later compensation, one row per
  rule kind) — `crates/domain/src/execution/reset.rs`, `crates/domain/tests/reset.rs` — every point
  classified as the registry says — FR-018-035, FR-018-036.
- [ ] T018-053 [US6] Reset store (prefix read/digest outside, verify and re-emit inside with `restored`,
  `reset_of`, `run.reset`, `rerun_of` + `reset_sequence`, guard after business-id row, `purged_at`/`test_tree`
  read, 022 permit acquire-or-refuse, 020 attribute fold) — `crates/infra-postgres/src/{resets.rs,execution.rs}`
  — content test — FR-018-037, FR-018-050, FR-018-051.
- [ ] T018-054 [US6] Reset service and routes (points, preview, reset, list/get) —
  `crates/application/src/runs/reset.rs`, `crates/api-http/src/routes/resets.rs` — contract tests —
  FR-018-035..037.
- [ ] T018-055 [US6] Replay = projection at every eligible point (corpus incl. a 022 run-permit holder and
  020 search-attribute upserts); outputs and attribute rows equal the source at N; holder count = open
  non-restored acquisitions; ceiling; crash during reset — `tests/integration/schedules-bulk-operations/reset.rs`
  — SC-018-011 — FR-018-037, FR-018-050.
- [ ] T018-056 [US6] Reapply order, first-undeliverable stop, `LEGACY_EVENT`, ceiling, key replay, lost
  hint — `tests/integration/schedules-bulk-operations/reapply.rs` — report equals expectation — FR-018-039.
- [ ] T018-057 [US6] `CANCEL_SOURCE` pending reset (fenced completion, eligibility re-check, races,
  crash, failed source compensation, pending quota) — `crates/scheduler/src/passes.rs`,
  `tests/fault/schedules-bulk-operations/reset.rs` — SC-018-012 — FR-018-038, FR-018-040.
- [ ] T018-058 [US6] Telemetry projector skips the restored prefix and links the source span —
  `crates/application/src/telemetry.rs`, `crates/scheduler/src/telemetry.rs` — no duplicate attempt
  spans — FR-018-037, FR-018-044.
- [ ] T018-059 [P] [US6] Studio reset dialog and provenance links — `frontend/apps/studio/src/features/runs/`
  — component tests — FR-018-047.

## Phase 9: US07 — Existing cron triggers as schedules (P3)

- [ ] T018-060 [US7] Publication maintains trigger-owned schedules in the publishing transaction; a
  reconciliation pass keeps them equal to `workflow_triggers` (old control planes); the pass fires them
  with the 004 identity; spec edits refused — `crates/infra-postgres/src/{definitions.rs,dispatch.rs}`,
  `crates/application/src/schedules.rs`, `crates/scheduler/src/schedules.rs` — publish/republish/remove
  and reconciliation tests — FR-018-017.
- [ ] T018-061 [US7] Handover gate: `schedule-trigger-handover` operator command, an activation of the
  instance-capability registry (T018-080) requiring `schedules-v1` from every live engine, legacy cron pass runs only in `LEGACY`, pause/resume/trigger refused before
  it; rolling upgrade with old and new engines on the same ticks; `cron_*` metrics kept —
  `crates/scheduler/src/events.rs`, `apps/control-plane/`, `apps/execution-engine/src/lib.rs`,
  `tests/fault/schedules-bulk-operations/upgrade.rs` — SC-018-005 — FR-018-017, FR-018-046; NFR-018-004.

## Phase 10: Polish, SDKs and gates

- [ ] T018-062 [P] Scenario files and expectations of `contracts/sdk-conformance.md` —
  `tests/contract/sdk-conformance/{scenarios,expected}/` — reviewed fixtures — FR-018-048.
- [ ] T018-063 [P] TypeScript SDK — `sdks/typescript/src/` — unit tests + scenarios — FR-018-048.
- [ ] T018-064 [P] Go SDK — `sdks/go/client/` — unit tests + scenarios — FR-018-048.
- [ ] T018-065 [P] Python SDK — `sdks/python/sagat_octopus_orchestrator/{client.py,sync.py}` — unit tests +
  scenarios — FR-018-048.
- [ ] T018-066 [P] C# SDK — `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/` — unit tests + scenarios — FR-018-048.
- [ ] T018-067 [P] Java SDK — `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/` — unit
  tests + scenarios — FR-018-048.
- [ ] T018-068 All five SDKs against one server build, skips stated — `sdk_conformance` output
  (SC-018-013) — FR-018-048.
- [ ] T018-069 [P] CLI `sagat schedule …`, `sagat batch …`, `sagat workflow {reset,signal-with-start,update-with-start}`,
  `--business-id`/`--if-exists`/`--start-delay` on `sagat workflow run` — `apps/cli/src/{args.rs,workflow.rs}`
  — CLI tests — FR-018-047.
- [ ] T018-070 Cross-tenant battery over every new route and pass — `tests/contract/security-multitenancy/rest/battery.rs`
  — battery output, audited probes — FR-018-041; NFR-018-002.
- [ ] T018-071 Observability gates (label closure, OTel export canary, audit rollback, alert rules
  `SagatSchedulesLate`/`SagatBatchesStalled`, alert kinds, quota exhaustion per resource) —
  `infra/observability/alerts.yaml`, `crates/domain/src/operations.rs`,
  `tests/integration/schedules-bulk-operations/observability.rs` — SC-018-014 — FR-018-044, FR-018-045; NFR-018-006.
- [ ] T018-072 Benchmark on a declared environment (firing lag at 1k/10k schedules, batch rate per job
  and 16 jobs, 32-way business-id claim, reset at 1k/20k events); adjust ceilings only from data —
  `tests/performance/schedules-bulk-operations/`, `tests/performance/reports/schedules-bulk-operations.md`
  — report with failures — NFR-018-005.
- [ ] T018-073 Runbooks/docs (`docs/runbooks/{schedules,batch-operations,run-reset}.md`, cron section of
  `docs/runbooks/events.md`, entry-wait pattern, business-id privacy warning) — review + link check —
  FR-018-049.
- [ ] T018-074 Real-browser e2e of schedule, batch and reset flows, keyboard only, WCAG AA, failure states
  — `tests/e2e/schedules-bulk-operations/` — SC-018-015 — FR-018-047.
- [ ] T018-076 Barrier test: every run-driven transition (completion, failure, timer, wait timeout,
  deadline, terminal, handoff) against each aggregate path; `pg_locks` never shows a run-driven
  transaction on schedule, business-id, job, item, obligation or pending-reset rows —
  `tests/fault/schedules-bulk-operations/run_driven_locks.rs` — SC-018-016 — FR-018-007, FR-018-021,
  FR-018-030.
- [ ] T018-077 Feature 020/022 integration once they merge: every run creator through 022's T10 (schedule
  action, starts, with-start, batch rerun, rerun, reset), reset `CONCURRENCY_PERMIT_UNAVAILABLE`,
  test-tree refusals, restored rows skipped by 020 analytics, attribute fold on reset, `RUN_PURGED`
  before and during reset, `business_id` erasure — `tests/integration/schedules-bulk-operations/flow_visibility.rs`
  — SC-018-011, SC-018-012 — FR-018-050, FR-018-051, FR-018-021.
- [ ] T018-078 IR field `reapplyOnReset` on Update declarations (capability-gated, ADR-0032) in the domain,
  IR codec and five SDK builders, merged from `contracts/workflow-ir.fragment.schema.json` —
  `crates/domain/src/workflow.rs`, `crates/infra-ir/src/`, `specs/001-platform-foundation/contracts/workflow.schema.json`,
  `sdks/*/` — schema/codec tests and builder fixtures — FR-018-039, FR-018-046.
- [ ] T018-079 `REPIN` batch action when feature 019 is enabled (`BATCH_ACTION_UNAVAILABLE` otherwise;
  `deployment:manage` + `run:operate`; item = 019 single-run repin in slot order run → 019 fences → job →
  item; skip reasons) — `crates/application/src/batch.rs`,
  `tests/integration/schedules-bulk-operations/batch_repin.rs` — SC-018-010 — FR-018-034.
- [ ] T018-080 Instance-capability registry (shared with 029 and 031): registration before serving under
  the epoch share lock, 10 s heartbeat on the database clock, 30 s liveness, epoch re-check in the control
  plane's API admission and the engine's scheduler rounds, the activation definer function and operator
  command pattern, `BINARY_CAPABILITY_MISSING` — `crates/infra-postgres/src/instances.rs`,
  `apps/control-plane/`, `apps/execution-engine/src/lib.rs`, `tests/fault/schedules-bulk-operations/instances.rs`
  — activation-vs-registration race, stale-instance expiry, epoch change while serving (SC-018-017) —
  FR-018-052.
- [ ] T018-075 Quickstart demos, then fmt, clippy `-D warnings`, workspace tests,
  `scripts/check-architecture.py`, contract checks, runtime-role sweep, Studio build; record results —
  `specs/018-schedules-bulk-operations/verification.md` (created at implementation) — FR-018-046,
  FR-018-049; all SC.

## Dependencies & execution order

- Phase 1 → Phase 2 → stories. T018-008 and T018-011 block every story; T018-012 blocks T018-036 and
  T018-049.
- US01 is the MVP. US02 and US07 depend on US01; US03 on Phase 2 only; US04 on US03; US05 on Phase 2
  (US03 for `RERUN` with `KEEP`); US06 on Phase 2, US03 (`KEEP`) and T018-043 (`via`, retained Update
  arguments).
- Studio tasks (T018-032, -038, -050, -059) follow their story's routes; SDK tasks (T018-063..067)
  follow T018-062 and the merged contract; T018-068 needs all five. T018-076 follows US05/US06;
  T018-077 needs features 020 and 022 merged (until then it is recorded as blocked, not passed);
  T018-078 precedes T018-043's reapply test and T018-056; T018-079 needs feature 019 merged; T018-080 precedes T018-061 (and 029's and 031's
  activations). T018-075 is last.

## Parallel examples

- Phase 2: T018-004, T018-005, T018-006 together once T018-003 fixes the module layout.
- US01: T018-013 with T018-014; T018-019 written first.
- Polish: T018-063..067 (one per SDK), then T018-069 with T018-070.

## Implementation strategy

1. MVP = Phases 1–3 behind a disabled-by-default admission flag; demo `quickstart.md` §2.
2. US02, US03; each story is enabled only after its tests pass.
3. US04, US05, US06 as dependencies allow; US07 after the rolling-upgrade test.
4. Polish and gates; nothing is advertised to SDK users before T018-068.

Task count: 80 (Setup 2, Foundational 10, US01 10, US02 11, US03 6, US04 5, US05 7, US06 8, US07 2,
Polish and cross-feature 19).
