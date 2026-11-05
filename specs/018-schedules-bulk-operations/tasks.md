# Tasks: Schedules and bulk run operations

**Input**: [spec.md](spec.md), [plan.md](plan.md), [research.md](research.md),
[data-model.md](data-model.md), [contracts/](contracts/), [quickstart.md](quickstart.md).

Status: Approved 2026-10-29 — in implementation. A task is checked only when its named evidence exists and was
run; skips and failures are recorded (constitution VIII). Tests are mandatory (AGENTS.md rule 9) and
are written failing before each story's implementation.

Format: `- [ ] T018-NNN [P?] [USn?] Description — files — evidence — traces`.

## Phase 1: Setup

- [x] T018-001 Owner review of ADR-0050 and O1000–O1002; re-run the plan's Constitution Check; record
  approved dependencies — `docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md`, `research.md`, `plan.md` — ADR status, recorded
  answers — FR-018-001..052.
  *Done 2026-10-29.* Evidence: the owner accepted ADR-0050 and the recommended O1000–O1002 in the session
  that started this work (`docs/owner-decisions.md`, 2026-10-29; ADR-0050 now `Accepted`; answers in
  `research.md` §5). The Constitution Check was re-run against constitution 1.1.0 (principles I–VIII
  unchanged in number and meaning; the only amendment, 1.1.0, adds the Rust SDK to principle IV): every row
  still passes; principle IV now also covers the Rust SDK, recorded as D1040 and T018-081. Approved
  dependencies: only the existing 004/009/017 services and the six SDKs; 019, 020 and 022 are NOT approved
  and every task that needs them (T018-037's permit, T018-051, T018-053's permit, T018-055, T018-077,
  T018-079) stays blocked, not passed, until they are accepted and merged.
- [x] T018-002 [P] Register targets `integration_`, `fault_`, `contract_`, `e2e_schedules_bulk_operations`
  — `tests/Cargo.toml`, `tests/{integration,fault,contract,e2e}/schedules-bulk-operations/main.rs` —
  targets compile — NFR-018-001.
  *Done 2026-10-29.* Evidence: `cargo test -p sagat-tests --locked --test integration_schedules_bulk_operations
  --test fault_schedules_bulk_operations --test contract_schedules_bulk_operations --test
  e2e_schedules_bulk_operations` — all four compile and run, 0 tests each (the modules land with their tasks);
  `check-agent-config.py`, `check-architecture.py` and `cargo fmt --check` clean.

## Phase 2: Foundational (blocks every story)

- [x] T018-003 Domain schedules: bounded spec/policy types, outcomes, pure `decide` (overlap, catch-up,
  remaining actions, tracking limit), deterministic jitter — `crates/domain/src/schedules.rs` — unit and
  property tests — FR-018-002..008, FR-018-012.
  *Done 2026-10-29.* Evidence: `cargo test -p sagat-domain --test schedules --test schedules_properties --locked`
  — 39 + 8 passed (the properties also at `PROPTEST_CASES=2000`, three runs); whole domain suite 647 passed,
  0 failed; `cargo fmt --check`, `cargo clippy -p sagat-domain --all-targets -- -D warnings` and
  `scripts/check-architecture.py` clean. The tests were seen failing first (unresolved module), a mutation of
  the catch-up window edge was caught, and a property found a real gap (D1043). Decisions D1041–D1044.
  Not in this task and still open: the calendar arithmetic and the spacing/never-fires checks
  (T018-013), and the pass that bounds the due slice of a long outage (T018-017).
- [x] T018-004 [P] Domain business ids: `BusinessId` (1–200 visible ASCII), policies, pure `admit`
  — `crates/domain/src/business_ids.rs` — policy × holder × window table — FR-018-018..020.
  *Done 2026-10-29.* Evidence: `cargo test -p sagat-domain --test business_ids --locked` — 13 passed (the two
  properties also at `PROPTEST_CASES=2000`, three runs); clippy `-D warnings`, `cargo fmt --check` clean. A
  mutation of the window edge (`>=` to `>`) was caught by `the_id_is_free_from_the_instant_the_window_ends`.
  A terminal holder with no finish time stays held (fails closed); `Debug` of a `BusinessId` prints no text.
- [x] T018-005 [P] Domain batch: selector, action set, item/job states, milli-item rate slot —
  `crates/domain/src/batch.rs` — state-machine tests — FR-018-027..034.
  *Done 2026-10-29.* Evidence: `cargo test -p sagat-domain --test batch --locked` — 18 passed (properties also
  at `PROPTEST_CASES=2000`, three runs); fmt, clippy `-D warnings` and `check-architecture.py` clean. Mutating
  `next_slot` was caught by the example and, after widening its generator, by the property too. The job
  machine is the diagram of `data-model.md`; a cancelling job does not fail (the cancel wins). The retry
  backoff (5 s doubling to 5 min, 10 attempts within 1 h) is D1045. Not here: selection, the horizon wait and
  the item adapters (T018-045..047).
- [ ] T018-006 [P] Domain reset and events `run.start_delayed`, `run.start_released`, `run.reset`,
  `wait.satisfied.via`; eligibility over `RunState::replay` + suffix facts; `ResetPlan` —
  `crates/domain/src/execution/{reset.rs,event.rs,mod.rs}` — per-cause tests — FR-018-023, FR-018-035..039.
  *Deferred 2026-10-29 (reordered, not dropped).* It is not on the US01 path (it serves US03 start delay, US04 `via`
  and US06 reset) and it adds variants to `RunEvent`, which is matched exhaustively in about nine crates (event codec,
  projection, DTOs, telemetry) and in `execution/{mod,state}.rs`, files another session was changing for 014 (PR #59 has
  since merged). It follows US01 and US02, after a fresh merge of `develop`.
- [ ] T018-007 Permissions, audit actions and quota resources of the plan —
  `crates/domain/src/{identity.rs,audit.rs,quota.rs}` — role-grant and range tests — FR-018-041, FR-018-045.
  *Part done 2026-10-29 (task stays open).* Done: the four permissions (`schedule:read|write|operate`,
  `run:batch`) with the role matrix of the plan, and the 16 audit actions. Evidence: `cargo test -p sagat-domain
  --test schedule_permissions --locked` — 6 passed; the whole domain suite 684 passed, 0 failed (the hand-written
  matrix of `crates/domain/tests/identity.rs` was updated on purpose: 48 → 52 permissions); `cargo check
  --workspace --all-targets` compiles. *Update 2026-10-29:* the quota resources `schedules` (200, hard 10,000) and
  `schedule_backfills` (4, hard 16) are done with migration `20261230000100_schedule_quota_resources` (it widens
  `quota_ceilings_resource_valid`, `quota_limits_resource_valid` and `quota_limits_tenant_scoped`; its `down` refuses while a
  limit or ceiling for them is set), their usage counts in `infra-postgres/src/quotas.rs`, the OpenAPI enums and the generated
  Studio types. Evidence: `schedule_quotas` 4 passed, the whole domain suite 695 passed, `integration_schedules_bulk_operations`
  14 passed, `integration_security_multitenancy` 126 passed, `integration_platform_foundation` 101 passed, workspace clippy
  clean. Still open: `batch_jobs`, `tenant_batch_jobs` and `pending_resets`, which land with the tables they count (their
  usage cannot be written before those tables exist).
  *Update 2026-11-04:* `batch_jobs` (namespace, 2 by default, hard 16) and `tenant_batch_jobs` (tenant, 8, hard 64) are
  done with the batch tables (migration `20270102000000_batch_operations`): the domain resources
  (`domain/tests/batch_quotas.rs` 3 passed), the lock of the tenant and then of the namespace and the count of the jobs
  that have not finished in `infra-postgres/src/quotas.rs`, the usage report, the `CHECK`s of the two ceilings tables, the
  OpenAPI enums and the generated Studio types (`npm run check:api` and `npm run typecheck` clean). `pending_resets`
  stays open with US06.
- [ ] T018-008 Additive migrations (tables incl. `schedule_actions` PK with `origin`, `batch_jobs.selection_horizon xid8`,
  `schedule_trigger_handover`, and the instance-capability registry tables unless 029 or 031 shipped them; `runs` columns with the erasure-aware
  immutability trigger; `run_shard` partial indexes; ALWAYS triggers; grants; trigger-owned backfill;
  data-loss-refusing down) — `crates/infra-postgres/migrations/<timestamp>_schedules_bulk_operations.{up,down}.sql`
  — forward/rollback, backfill and erasure-trigger tests — FR-018-001, FR-018-017, FR-018-021, FR-018-046.
  *Part done 2026-10-29 (task stays open).* Migration 1 of the feature, `20261230000000_schedules` (a free range
  after a grep of every branch: 014 has `…226000000`, 036 reserves `…226000000/100/200`, 035 `20270115000000`): `schedules`,
  `schedule_revisions`, `schedule_actions`, `schedule_backfills`, `schedule_obligations`, `schedule_trigger_handover`, with
  the deferred head-revision key, the ALWAYS guards (identity, append-only revisions, BUFFERED → final actions, a waiting
  action never purged), content accounting on the name, note, spec and action, grants to `sagat_app` (`UPDATE` so it can lock
  rows, no `DELETE`) and a `down` that refuses to drop schedules or a recorded handover. No key points at `workflow_triggers`
  so an older control plane that republishes keeps working. Evidence: `cargo test -p sagat-tests --test
  integration_schedules_bulk_operations --locked` — 12 passed (constraints, immutability, runtime privileges and a row lock
  as `sagat_app`, rollback refused with data and after the handover, drain, re-apply), also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`; `integration_platform_foundation` 101 passed (migrations, runtime privileges), both
  ways; a mutation of the action guard was caught. Not done: business ids and the `runs` columns with their erasure-aware
  trigger, batch jobs and items, resets, the five quota resources with the `quota_ceilings_resource_valid` CHECK, the
  instance-capability registry, the trigger-owned backfill and the purge functions.
  *Update 2026-11-04:* the batch tables are done: migration `20270102000000_batch_operations` (`batch_jobs` with the
  selection bound, the `xid8` horizon of the creating snapshot, the durable rate slot and a generated shard;
  `batch_items` with one row per run; `ALWAYS` guards that keep what a job is fixed, a finished job finished and an ended
  item as it ended; content accounting of the justification and the selector; grants without `DELETE`; a `down` that
  refuses while a job exists or a limit for the resources is set). Tested in `batch::` (guards, privileges, rollback and
  applying again). Still open: the business-id table and the `runs` columns, resets, the instance-capability registry, the
  trigger-owned backfill and the purge functions of jobs and items.
- [ ] T018-009 Classify new variable-size columns; register payload-referencing columns for 020's
  reference check; extend the runtime-privilege test — `crates/infra-postgres/storage-classification.json`,
  `tests/integration/platform-foundation/runtime_privileges.rs` — manifest test, `SAGAT_TEST_AS_RUNTIME_ROLE=1`
  — FR-018-042, FR-018-045.
  *Part done 2026-10-29 (task stays open).* The 28 variable-size columns of migration 1 are classified in
  `storage-classification.json` (6 tables; `name`, `note`, `spec`, `action` are content, charged by ALWAYS triggers; the rest
  protected, each with the CHECK that bounds it) and `crates/infra-postgres/src/schedules.rs` exists as their owner, marked
  PENDING until T018-014. Evidence: `manifest_covers_every_variable_size_column_and_every_accounted_table_has_always_triggers`
  and the 8 `durable_quotas` tests pass. Not done: the runtime-privilege audit of the stores' statements (no statement exists
  yet) and the columns of the later migrations.
  *Update 2026-11-04:* the 11 variable-size columns of `batch_jobs` and `batch_items` are classified (the selector and the
  justification as content, the rest protected with the `CHECK` that bounds it) and `infra-postgres/src/batch.rs` owns
  them; `manifest_covers_every_variable_size_column_and_every_accounted_table_has_always_triggers` passes. The accounting
  triggers take the content columns in the order of the manifest (selector, then reason), which the test enforces.
- [ ] T018-010 Merge the REST fragment, AsyncAPI additions (intentional baseline regeneration) and
  reasons; assert `workflow.schema.json` and `worker.baseline.json` unchanged —
  `specs/001-platform-foundation/contracts/{openapi.yaml,events.asyncapi.yaml,events.baseline.json}` —
  `contract_schedules_bulk_operations` — FR-018-043, FR-018-046.
  *Part done 2026-10-30 (task stays open).* The REST operations built so far are merged into
  `specs/001-platform-foundation/contracts/openapi.yaml` with `x-implemented: '018'`: `listSchedules`, `createSchedule`,
  `getSchedule`, `updateSchedule`, `deleteSchedule`, `pauseSchedule`, `resumeSchedule`, `triggerSchedule`, `listScheduleBackfills`, `createScheduleBackfill`, `cancelScheduleBackfill` (the spec and policies schemas are
  inlined as `ScheduleSpec`/`SchedulePolicies`, because the canonical document has no external references; business-id
  fields are left out until US03; G1015 names what the draft listed and is not built). `docs/reference/api.md` regenerated
  after reading the diff (11 operations, group Schedules) and `frontend/apps/studio/src/api/schema.d.ts` regenerated
  (`npm run check:api` clean, `npm run typecheck` clean). Evidence: `contract_schedules_bulk_operations` 1 passed (every
  response of the eleven operations, 2xx and 4xx, validated against the schema, plus a check that all eleven were
  exercised); the cross-tenant battery (`contract_rest_security_multitenancy`, 17 passed) now holds a victim schedule and
  a valid body for each write; `contract_payload_classification` 18, `contract_developer_experience` 8,
  `contract_rest_platform_foundation` 9 and the other REST contract suites pass. Not done: the AsyncAPI additions and the
  baseline regeneration, the operations of US02..US07.
- [ ] T018-011 Ports `ScheduleStore`, `ScheduleCalendar`, `BusinessIdStore`, `BatchStore`, `ResetStore`;
  `NewRun` admissions `schedule`, `business_id`, `delay`, `reset` processed by `RunStore::create` in
  ADR-0050 order — `crates/application/src/{schedules.rs,batch.rs,runs.rs,runs/business_id.rs,runs/reset.rs}`
  — validation unit tests — FR-018-005, FR-018-021.
  *Part done 2026-10-29 (task stays open).* `ScheduleStore` (create, read, list) and the records it speaks
  (`ScheduleRecord`, `NewSchedule`, `StoredInput`, origin and state) in `application/src/schedules.rs`, beside the
  `ScheduleCalendar` port. The strict JSON codec lives in `application/src/schedule_codec.rs` (moved from `infra-ir` so that
  `api-http` and `infra-postgres` share it without an edge between adapters). Not done: `BusinessIdStore`, `BatchStore`,
  `ResetStore`, and the `NewRun` admissions processed by `RunStore::create`.
  *Update 2026-11-04:* `BatchStore` (create with the ceilings and the request key, read, list, items, request cancel,
  preview) is built in `application/src/batch.rs`, with the records it speaks and the codec of the selector and the
  parameters (`application/tests/batch_codec.rs` 5 passed); `PgBatchStore` implements it. Not done: `ResetStore`,
  `BusinessIdStore` and the `NewRun` admissions.
- [ ] T018-012 Lock-order interleaving harness (barriers, deadlock counter) for the merged admission slots
  of ADR-0050 decision 7 and both aggregate orders (aggregate → run; run → bookkeeping row), with
  slot-order cases for the admission paths of 019, 021, 022, 024, 025, 026 and 030 as they merge —
  `tests/support/lock_order.rs` — self-test reproduces an opposite-order deadlock and passes the slot
  order — NFR-018-001.
  *Part done 2026-11-04 (task stays open for the slot-order cases of the admission paths of 019, 021, 022, 024, 025,
  026 and 030, which exist only as those features merge).* The harness is built: `interleave` runs plans of locks at once,
  each in a transaction and a connection of its own, and holds every plan after each lock until the others have taken
  theirs or are seen waiting on a lock in `pg_stat_activity` (a plan queued behind another one cannot keep the holder at a
  barrier); `Slot` and `assert_ascending` state the ten slots of the decision and refuse a plan that returns to a lower
  one; `RowLockMonitor` samples the row locks of a table with `pgrowlocks`, because `pg_locks` does not list them, and
  reports any transaction that holds more than one. Evidence, `fault_schedules_bulk_operations` `batch_locks::` (7
  passed): two plans that take two rows in opposite orders give exactly one deadlock victim and one commit (and the
  database counter `pg_stat_database.deadlocks` rises by one), in the same order both commit; the same for a ring of
  three advisory locks, and for three plans in one order; a plan in the order of the slots is accepted and one that goes
  from a run back to a schedule row is refused by name; the monitor sees a transaction that holds two run locks. A first
  version of the barrier hung when a plan was queued behind the other (found by running it), and sampling on the
  connection that holds the locks hung it too; it samples from another connection now.

## Phase 3: US01 — Run a workflow on a schedule (P1, MVP)

- [x] T018-013 [P] [US1] `ScheduleCalendar` adapter (calendars, 004 cron grammar shared with `cron.rs`,
  intervals, exclusions, bounds, DST, next-N, spacing over 1,000) — `crates/infra-ir/src/schedule_calendar.rs`
  — ≥ 200-case table incl. `America/New_York`, `Europe/Lisbon` (SC-018-001) — FR-018-002, FR-018-003.
  *Done 2026-10-29.* Evidence: `cargo test -p sagat-infra-ir --test schedule_calendar --locked` — 27 passed, among
  them 730 daily occurrences for each of `Europe/Lisbon` and `America/New_York` (1,460 cases, SC-018-001) checked against
  an oracle written from the zones' daylight-saving rules and civil-calendar arithmetic, not from `chrono-tz`; gaps and
  repeated times for both policies; exclusions judged in the zone; start inclusive, end exclusive; the spacing, never-fires
  and unknown-zone refusals; a property that occurrences are ordered, bounded and resumable. Three mutations (start
  exclusive, exclusions in UTC, the later of an ambiguous time) were each caught. `infra-ir` + `application` suites 470
  passed, clippy `-D warnings`, fmt and `check-architecture.py` clean. The port is `sagat_application::schedules::
  ScheduleCalendar` (also part of T018-011). G1013: the `dst` policy does not reach cron strings.
- [x] T018-014 [US1] PostgreSQL `ScheduleStore` (revisions, head CAS, due scan by `(shard, next_wake_at)`
  bounded by `now()`, actions, purge) — `crates/infra-postgres/src/schedules.rs` —
  `tests/integration/schedules-bulk-operations/store.rs` — FR-018-001, FR-018-013, FR-018-014.
  *Part done 2026-10-29 (task stays open).* `PgScheduleStore` (`infra-postgres/src/schedules.rs`) creates, reads and lists
  schedules with their head revision: one transaction takes the namespace fence (advisory lock), checks the name and the
  quota (active and paused count, deleted do not), stores the input as a content-addressed payload after checking its
  digest, inserts the schedule, its revision and the audit record, and commits (the head-revision key is deferred).
  Evidence: `cargo test -p sagat-tests --test integration_schedules_bulk_operations --locked` — 25 passed (11 of the store:
  read-back equality, revision and audit committed together, a refused name leaves nothing, quota, a 6-way race for the
  last 3 places, content charged, shared input payload, forged digest refused, another tenant sees nothing, paging, runtime
  role), also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`; `integration_platform_foundation` 101 passed (the runtime-privilege audit
  reads these statements); removing the fence makes the race test fail (6 created instead of 3).
  *Update 2026-10-29:* `revise_schedule` (a new revision under a compare-and-swap on the head, rename with the name check,
  conflict leaves nothing, six racing revisions have one winner), `set_schedule_state` (pause or resume with the note, actor
  and time; the resumer becomes the authority; not a revision) and `delete_schedule` (tombstone, waiting actions become
  `DROPPED_DELETED`, the name is free, a second delete is a no-op). Evidence: `integration_schedules_bulk_operations` 34
  passed (9 in `changes`), also as the runtime role; `integration_platform_foundation` 101; dropping the head check fails 2
  tests. Not done: the due scan by `(shard, next_wake_at)`, actions, purge, backfills, obligations.
  *Reconciled and ticked 2026-11-04.* The "not done" above was written before the later tasks and is out of date: the due
  scan is `PgScheduleStore::due_schedules` (the shards the replica owns, `ACTIVE`, `next_wake_at <= clock_timestamp()`,
  oldest first, bounded; tested through the pass in `pass::` and `fault_schedules_bulk_operations`), the actions are
  written with the decisions and the admissions (T018-015, T018-024..027, T018-030) and read by `describe`, the purge is
  T018-028, the backfills T018-025 and the obligations T018-030. Retention of the revisions and tombstones is G1954.
- [x] T018-015 [US1] Schedule admission inside `RunStore::create` (fence, CAS, action + state after the
  initial reduction, refusal commits nothing) — `crates/infra-postgres/src/{execution.rs,schedules.rs}`
  — one-commit content test — FR-018-004, FR-018-005.
  *Done 2026-10-29.* `create` takes the shard fence (`scheduler_shards` `FOR SHARE`, expiry by the database clock), then the
  schedule row `FOR UPDATE` with the compare-and-swap on `state_version` (ADR-0050 decision 7), right after the replay check;
  the run (with its four provenance columns, migration `20261230000200`, immutable by an `ALWAYS` trigger), the action row and
  the new state of the schedule commit together. A replay of the key of a time is `StateConflict` (another process acted
  first), never a second `Ok`. Evidence: `integration_schedules_bulk_operations` 82 passed (12 in `admission`:
  `a_scheduled_action_starts_its_run_and_moves_the_schedule_in_one_commit`, `the_same_nominal_time_never_starts_a_second_run`,
  `a_decision_made_on_a_stale_schedule_commits_nothing`, `a_paused_schedule_admits_no_scheduled_run`,
  `a_run_the_admission_refuses_takes_the_action_and_the_schedule_change_with_it`, the three lease tests,
  `a_scheduled_run_is_audited_as_started_by_the_schedule`, `the_provenance_of_a_run_never_changes`, and for migration
  `20261230000200`: `a_rollback_is_refused_while_a_run_carries_the_provenance_of_a_schedule` (refused, drained by an operator,
  rolled back, migrated forward with the run present) and `the_provenance_of_a_run_is_all_or_nothing_and_of_a_known_kind`;
  without the guard in `down.sql` the first fails); also as the runtime role; `integration_platform_foundation` 101,
  `integration_security_multitenancy` 126, `integration_durable_execution` 178; workspace clippy
  `-D warnings`, `cargo fmt --check`, `check-architecture.py` and `check-agent-config.py` clean.
- [ ] T018-016 [US1] `ScheduleService` and `ScheduleFiring` (version policy, input check at action,
  `nominalTimeField`, business-id prefix, retry backoff) — `crates/application/src/schedules.rs` —
  tests over the real store — FR-018-001, FR-018-005, FR-018-016.
  *Part done 2026-10-29 (task stays open).* `ScheduleService` is built: create, get, list, update, delete, pause and resume,
  in `application/src/schedules.rs`. Writers hold `schedule:write` + `run:start`, operators `schedule:operate` (+ `run:start` to
  resume), readers `schedule:read`; the namespace and the target workflow (and a pinned version) must exist; the calendar
  validates the spec and its refusals carry the contract's reasons (`SCHEDULE_ZONE_UNKNOWN`, `SCHEDULE_NEVER_FIRES`,
  `SCHEDULE_TOO_FREQUENT`, `SCHEDULE_SPEC_INVALID`, `SCHEDULE_INPUT_INVALID`); the next nominal time and the jittered actual
  time are computed from the schedule id (SHA-256 of `jitter_seed`), an update continues from the first time after it and a
  resume from the first time after the resume with nothing caught up; the updater or resumer becomes `authorizedBy`; every
  change is audited without payload or note; a cron-trigger-owned schedule answers `SCHEDULE_TRIGGER_OWNED` /
  `SCHEDULE_TRIGGER_HANDOVER_PENDING`; new `AppError::InvalidSchedule` / `ScheduleConflict` map to 422 / 409 with those
  reasons. Evidence: `integration_schedules_bulk_operations` 51 passed (17 in `service`), also as the runtime role;
  `integration_platform_foundation` 101; workspace clippy `-D warnings`, `check-architecture.py` and `cargo fmt --check` clean;
  api-http + application + domain + infra-ir 1,251 passed. Not done: the input check against the target version's schema
  (`SCHEDULE_INPUT_INVALID` today means "not an object" or too large; the schema check belongs to the action time and to
  the definitions port), `nominalTimeField` against the input schema (`NOMINAL_TIME_FIELD_INVALID`), `ScheduleFiring`
  (version policy, input check, business-id prefix, retry backoff at the action).
  *Update 2026-10-29:* `ScheduleFiring` is built (`application/src/schedule_firing.rs`): one step per call — reconcile the
  tracked runs that ended (pause-on-failure with a system note and audit), recheck the authority (a revoked membership pauses
  the schedule with an `AUTHORITY_REVOKED` row), release the oldest waiting action when nothing is open, wake a waiting action
  every 5 s while a run holds it back, plan the recovery of the due times and act (start, skip, buffer, drop, coalesce); a
  refused admission keeps the time owed and retries it under the same key with a delay of 5 s to 5 min; the nominal time goes
  into the declared input field. The business-id prefix is refused with `NotAllowed` until US03. Evidence: 15 tests in
  `firing.rs`, 82 passed in all, also as the runtime role. Not done: the input check against the schema of the target
  version, `nominalTimeField` against that schema, business-id prefix.
  *Decision 2026-11-04 (D1956, gap G1955): the input is not checked against the input schema of the target version.* The
  workflow IR declares an `inputSchema`, but nothing on the platform enforces it: a run started by the API, a signal or
  a worker is not validated against it either, so a schedule that refused the input a person may start the same
  workflow with would be inconsistent, and the check needs a validator port that the run start would use as well. What a
  schedule checks stays: an object, at most 64 KiB. The business-id prefix waits for US03. `nominalTimeField` is
  already written into the input at the action.
- [x] T018-017 [US1] Schedule pass (owned shards, fairness, round ceiling, metrics) and engine wiring —
  `crates/scheduler/src/{schedules.rs,lib.rs,monitor.rs}`, `apps/execution-engine/src/lib.rs` — two-replica
  pass tests — FR-018-005, FR-018-044.
  *Part done 2026-10-29 (task stays open).* `SchedulePass` (`scheduler/src/schedules.rs`) looks at the due schedules of the
  shards the replica owns (`batch` per round), takes at most `steps_per_schedule` steps per schedule per round, and acts under
  the lease of the keeper's placement; the engine runs it behind `SAGAT_SCHEDULES_ENABLED` (off by default, with
  `SAGAT_SCHEDULES_BATCH` and `SAGAT_SCHEDULES_POLL_MS`). Evidence: 4 tests in `pass.rs` (one replica starts the run; a
  replica with no shard does nothing; two replicas split twelve schedules by shard and each time starts one run; a replica
  whose lease was taken is refused where it acts and the time stays owed), 2 in `apps/execution-engine/tests/config.rs`;
  removing the fence from the pass makes the lease test fail (1 step, 0 refused instead of 0 and 1). Not done: fairness
  between tenants, the gauges (`scheduler_lag_seconds{pass="schedules"}`, `schedule_actions_total`) and the monitor alert.
  *Update 2026-11-04 (T018-022):* the round ceiling is built: `round_actions` (100, hard 1,000) bounds the steps one round
  takes over all its schedules and backfills.
  *Update 2026-11-03:* the engine now gives the firing a `DatabaseClock` synced before every round (D1802, T018-020), so
  due-ness and the planning of the times agree on the database's time.
  *Update 2026-11-04 (T018-029):* the pass that bounds the due slice of a long outage, which T018-003 left to this task,
  is built: a recovery is planned once, remembered on the schedule and carried out a step at a time (D1951).
  *Done 2026-11-05 (the rest of the task: fairness, the gauges and the alert).* **Fairness.** The pass took the oldest
  `batch` of any tenant, so one tenant's backlog (an outage on a thousand per-minute schedules) held back every schedule
  that was due later. It now shares the batch by `fairness::select` like the passes of the runs (`due_schedules` takes
  a `Window` with the shards and the tenants to leave out; `SAGAT_SCHEDULER_DISCOVERY_REST_MS` rests the search;
  `scheduler_fairness_limited_total` and `_exhausted_total` carry `pass="schedules"`). Seen failing first:
  `pass::a_tenant_with_a_backlog_does_not_hold_back_another_tenants_due_schedule` (twelve schedules of one tenant due two
  hours ago, one of another due one hour ago, batch of four: the second tenant had 0 runs after the round, 1 after the
  change) and `a_lone_tenant_with_a_backlog_still_gets_the_whole_batch` (four of four: the policy is work-conserving).
  **Gauges and counters.** `scheduler_lag_seconds{pass="schedules"}` is how long the oldest active schedule has been
  due, `{pass="batches"}` how long the oldest job the batch pass would take has waited (a job held back by an older open
  transaction is not due, so it is not lag: no replica added would change that), `batch_jobs_active`; and
  `schedule_actions_total{kind,outcome}` once a decision is committed (a decision the fence or the compare-and-swap
  refused counts nothing; `admission_retry` for a time left owed), `schedule_action_lag_seconds{kind}` (how long after
  its time a scheduled action started), `batch_items_total{action,outcome}` once an outcome is committed (cancelled items
  of a drain included) and `batch_item_duration_seconds{action}`. No label is a tenant, schedule, job or run. **Alert.**
  `SagatSchedulerLate` is `max(scheduler_lag_seconds) > 60` with no `pass` matcher, so it covers the two new passes, and
  so does the KEDA query of the chart; the runbooks (`execution-engine.md`, `scaling.md`, `observability.md`) list them.
  Evidence (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`): `integration_schedules_bulk_operations` 224 passed, also
  with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (224); new: `lag::` 2, `counted::` 3 (a recorder of the test's own on a one-thread
  runtime), `pass::` 2; `fault_schedules_bulk_operations` 30; `contract_schedules_bulk_operations` 2;
  `integration_scalability_ha` 44; `contract_scalability_ha` 25 (it caught four comparisons with `clock_timestamp()` in
  the new sample SQL, which the planner cannot use as the bound of a range: now `now()`); `integration_observability_operations` 20;
  `contract_observability_operations` 9; `contract_developer_experience` 8; `integration_platform_foundation` 101 (the
  runtime-privilege audit reads the new statements); unit suites of application, scheduler, infra-postgres,
  execution-engine, domain and telemetry; `cargo fmt --check` and `cargo clippy --workspace --all-targets --locked -- -D
  warnings` clean. Mutations, each reverted: 11 on the metrics (a job held by an older transaction counted as late, a
  paused schedule counted as late, finished jobs counted as active, the batch lag showing the schedule lag, a started run
  and a decision not counted, the lag measured from the wrong end, a refused decision counted, cancelled items not
  counted, a succeeded item counted as skipped, the item duration not recorded) and 2 on the fairness (the tenant
  exclusion dropped from the query, the pass ignoring it): all caught; the one that survived (the duration recorded as 0,
  the test only counted samples) got an assertion on the sum. Not done: `schedules_paused_by_system_total`; the
  `delayed_starts` and `resets` passes belong to the features that build them (US03, US06).
- [x] T018-018 [US1] REST create/read/list/update/delete — `crates/api-http/src/routes/{schedules.rs,mod.rs}`,
  `crates/api-http/src/dto_schedules.rs` — contract tests — FR-018-001, FR-018-013, FR-018-043.
  *Part done 2026-10-30 (task stays open).* Built behind `SAGAT_SCHEDULES_ENABLED` (control plane, off by default):
  `ScheduleUseCase` (a trait object over `ScheduleService`, so the HTTP layer needs no new generic parameter), the document
  decoder `decode_document` (the ceilings of the service; a refusal names `details.field` as a JSON pointer), `describe`
  (next times from the calendar, counters and latest decisions from the store), an atomic delete at `expectedRevision`,
  the listing newest first, and the routes with their `ApiOperation`s (`ScheduleRead`, `ScheduleWrite`, `ScheduleOperate`,
  `ScheduleResume`). Admission authorizes for the namespace of the path first; a schedule of another namespace or tenant
  is `404`. The Idempotency-Key is required but not replayed (G1014). Evidence: `integration_schedules_bulk_operations`
  (10 in `rest`: the whole life, the input never returned, refusals with reason and pointer, name conflict, newest-first
  paging, key required, pause and resume, roles, namespace and tenant isolation, malformed ids; 3 in `service` for
  `describe` and delete at a revision; `crates/application/tests/schedule_document.rs` 4); without the namespace check of
  the route the isolation test fails. Not done: the list filters `origin`, `state`, `workflowId`, `name` and `inputBytes`
  (G1015).
  *Done 2026-11-05 (the filters).* `GET /namespaces/{id}/schedules` takes `state` (`ACTIVE`, `PAUSED`; a deleted schedule is
  never listed), `origin` (`API`, `TRIGGER`), `workflowId` (the workflow the head revision starts) and `name` (names that
  start with it, at most 128 characters); every part given has to hold, a value that is none of the listed ones is `400`, and
  the order and the cursor are those of the unfiltered listing. No new index: the scan is bounded by the `schedules` quota
  (G1015 keeps `inputBytes` and the index as open). The Studio list narrows by state, workflow and name (asked for once
  typing has rested). Seen failing first: `rest::a_listing_can_be_narrowed_by_state_workflow_and_name_and_pages_keep_to_the_filter`
  (the filters were ignored: five schedules where one was asked for) and `a_filter_that_is_not_one_is_refused_in_words`
  (`state=DELETED` answered `200` with an empty page). Evidence: `store::a_listing_is_narrowed_by_every_part_of_the_filter_...`
  (five schedules over two workflows, one trigger-owned, one paused: each part alone, all of them together, a name matched
  by its start and not inside, and paging one at a time through each filter gives the listing as a whole), the two `rest::`
  tests above, the contract test (the filtered and refused requests conform to the schema), 13 Studio tests of the list. Gates on the final tree: `integration_schedules_bulk_operations` 227 passed, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (227); `fault_schedules_bulk_operations` 30; `contract_schedules_bulk_operations` 2; `contract_rest_platform_foundation` 9; `contract_rest_security_multitenancy` 17; `contract_developer_experience` 8 (the reference lists no query parameter, so it did not change); `integration_platform_foundation` 101; unit suites of application, api-http, infra-postgres, scheduler and domain; Studio from `frontend/`: lint, typecheck, `npm test` (95 files, 1,321 tests), build and `check:api` clean; fmt and clippy clean.
  Mutations, each reverted: 7 on the store and the route (each predicate dropped, a name matched anywhere, the filters ORed
  with the cursor, a deleted state accepted, the name length unbounded) and 7 on the Studio list (state, workflow or name
  not sent, a letter asked for at a time, an untrimmed name, one cache for every filter, the wrong empty message): all caught
  (the first run of the cursor one survived and got the page-through-every-filter assertion). Still open from this task
  and registered: G1014 (the key of a write is required and not replayed; the spec asks for replay only for trigger-now and
  the with-start operations).
- [x] T018-019 [P] [US1] US01 scenarios 1–5 — `tests/integration/schedules-bulk-operations/schedules.rs`
  — target output — FR-018-001..005, FR-018-016.
  *Done 2026-11-04.* The scenario file walks the five scenarios of the story and says, for each, which test proves it.
  These are tests of behaviour that already existed, so none was red first and none found a defect (the defects of the
  story were found by T018-020..022, 029). Scenario 1: a spec with a calendar, a cron string and an interval, the zone
  `Europe/Lisbon`, `startAt`, `endAt` and ten minutes of jitter is revision 1, its first nominal time is the earliest
  of the three kinds on or after the start, the actual time moves by less than the setting, the next ten times are in
  order and inside the bounds, nothing runs on 1 November and one run starts for 2 November. Scenario 2: the replica
  that lost the shard considers the time first and is fenced with nothing started, the holder starts exactly one
  run (`startedBy = system:schedule:<id>`, schedule id, nominal time, kind and revision on the run, key
  `nominal-{ms}` in scope `schedule:{id}`), and a second look by the former finds nothing; the engine-level proof is
  `pass::two_replicas_split_the_schedules_by_shard_and_every_time_starts_one_run`; "counted" waits for the gauges
  of T018-017. Scenario 3: the 28-write kill matrix of T018-020 (`fault_schedules_bulk_operations`). Scenario 4: four
  specs that never fire or fire too often are refused with `SCHEDULE_NEVER_FIRES` / `SCHEDULE_TOO_FREQUENT` and a
  before-and-after count of schedules, revisions, payloads, audit records and charges is unchanged. Scenario 5:
  `catchup::` (a full quota, then the window ends; no published version, with a backoff). Evidence:
  `integration_schedules_bulk_operations` 169 passed, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (169); `cargo fmt --check`
  and `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
- [x] T018-020 [US1] Faults: crash after each statement of transaction 2, stale fence, duplicate pass,
  lost hint, restart, skewed process clocks — `tests/fault/schedules-bulk-operations/firing.rs` (+ `clocks.rs`,
  `tripwire.rs`) — one run
  and one action per time, `verify_run` (SC-018-002) — FR-018-004, FR-018-005; NFR-018-001.
  *Done 2026-11-03 for the start and skip transactions (gap G1018 names what is not killed at each write).*
  Evidence: `cargo test -p sagat-tests --locked --test fault_schedules_bulk_operations` — 11 passed, also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  *Crash at each statement:* a statement-level trigger on every table kills its own backend after the Nth writing statement
  (`tripwire.rs`). The watched start is one transaction of **28 writing statements on 13 tables** (`audit_records`,
  `command_dedupe`, `event_outbox`, `payloads`, `quota_transaction_deltas`, `run_active_counts`, `run_events`,
  `run_instance_usage`, `runs`, `schedule_actions`, `schedules`, `steps`, `telemetry_pending`) and the skipped time one of **3
  writes on 3 tables** (`quota_transaction_deltas`, `schedule_actions`, `schedules`). For each position the step is
  refused as an admission the store could not take (or fails) and is never reported done; an md5 digest of the content of
  every other table is unchanged; the schedule's next time, remaining actions, buffer and tracked runs are unchanged; and after
  the backoff the next pass acts exactly once: for a start one run with `startedBy = system:schedule:<id>`, the provenance
  columns, scope `schedule:{id}` and key `nominal-{epoch ms}` in `command_dedupe`, one `STARTED` action and `verify_run`
  clean; for a skip one `SKIPPED_OVERLAP` beside the one `STARTED`. A step armed one write past the last is not cut off
  (the matrix is the whole transaction), and a firing split over two transactions fails the one-transaction assertion
  (`a_process_killed_after_any_write_of_a_start_…`, `…_of_a_skip_…`). *Restart / lost reply:* the firing is rebuilt from the
  database alone (`a_reply_that_never_arrives_does_not_make_the_restarted_process_start_the_time_again`). *Duplicates and stale
  fence:* `eight_passes_racing_over_a_backlog_start_each_time_once` (8 passes released by a barrier on a multi-thread
  runtime; 3 times, 3 runs, 3 actions; 15 steps lost the swap in the recorded run) and
  `a_replica_with_a_stale_lease_and_a_duplicate_pass_leave_one_run_and_one_action` (the usurped replica is refused with
  nothing recorded; the same pass twice at once acts once; a lease that ran out by the database clock is
  `admission::a_lease_that_has_run_out_fences_its_owner_off_by_the_database_clock`). *Lost hint:* not applicable by
  construction — a schedule is found by the pass's own poll of the database and no broker message announces it; the pass
  tests run with no broker. *Skewed clocks:* a real defect, found by the first of these tests failing against a stub — the
  engine fed the firing `SystemClock` although the module doc says the database's time, so a process 3 h ahead swallowed
  times 2.5 h in the future into a summary (decision D1802). Fixed by `DatabaseClock` (`infra-postgres/src/clock.rs`) in the
  engine (`loops::schedule_round`, synced before every round; a clock never measured does nothing) and in the control plane
  (synced at start and every 5 s). Tests: `…_runs_hours_ahead_does_not_act_on_times_the_database_has_not_reached`,
  `…_hours_behind_still_acts_on_the_times_that_are_owed`, `a_synced_clock_reads_the_database_whatever_the_process_clock_says`,
  `a_clock_that_cannot_reach_the_database_keeps_the_last_time_it_knew`,
  `the_round_of_the_engine_acts_on_the_database_time_of_a_replica_whose_clock_is_hours_fast` (calls `schedule_round`; removing
  its sync fails it, run as a mutation) and `a_round_whose_clock_was_never_measured_does_nothing`. An independent
  correctness review of the first version found the control plane unfixed, the fail-open first round, the pool wait inside
  the measurement and weak assertions; all four are fixed above. Not done: killing the other decision-only steps (G1018), a
  mutation that splits the firing in two transactions, and any test that boots the control plane with a skewed clock.
  Gates on the final tree (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`, Testcontainers PostgreSQL 16), each also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`: `fault_schedules_bulk_operations` 11 passed; `integration_schedules_bulk_operations` 123
  passed (regression of the fixture move to `tests/support/schedules.rs`, shared by both binaries);
  `integration_platform_foundation` 101 passed (the runtime-privilege audit); `integration_security_multitenancy` 126 passed;
  unit suites of `sagat-execution-engine`, `sagat-control-plane`, `sagat-infra-postgres`, `sagat-scheduler` and
  `sagat-application` green; `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `check-architecture.py` and `check-agent-config.py` clean. Not run: the whole-workspace test run (CI runs it) and the SDK
  checks (no SDK or contract changed).
- [x] T018-021 [US1] Authority and provenance (recheck under membership lock order, auto-pause + audit,
  `system:schedule:<id>`, 017 database authority, key replay) —
  `tests/integration/schedules-bulk-operations/authority.rs` — SC-018-004 — FR-018-015, FR-018-016, FR-018-041.
  *Done 2026-11-04 (decision D1950, gap G1950).* Two real defects, both found by a test that failed first. (1) The
  authority was read once outside the transaction that admits the run, so a revocation that committed in between did not
  stop the run: `ScheduleAdmission.authority` now names the subject the action acts for and `RunStore::create` locks its
  membership `FOR SHARE` (`sagat_lock_memberships`) and checks `run:start` again, after the shard fence and the schedule
  row and before the quota checks; the refusal is `StateConflict(AUTHORITY_REVOKED)`, which the firing turns into the
  `AUTHORITY_REVOKED` action plus the pause (a scheduled time, a waiting action), the stop of a backfill, or `Forbidden`
  for trigger-now; a replay of the key is answered before the check. (2) A workflow with a managed database step could
  never be started by a schedule: the 017 admission looked for the database authority of `system:schedule:<id>`, which no
  membership has (`fire` answered `NotFound` and the time stayed owed for ever); `database::authority_of` now stands the
  run on the member `authorizedBy`, and the action requires of that member `database:use` (and `database:write` when a step
  writes), taking the catalog lock before the membership as 017 does. Evidence, all on the final tree with a private
  `CARGO_TARGET_DIR` and `RUST_TEST_THREADS=2`: `integration_schedules_bulk_operations` 144 passed (134 before; 10 new:
  6 in `authority`, 2 in `admission`, 1 in `trigger`, 1 in `backfill`), also with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (144);
  `fault_schedules_bulk_operations` 11 passed, both ways; `integration_platform_foundation` 101 (the runtime-privilege audit
  reads the new statement); `integration_security_multitenancy` 126; `integration_database_connections` 7;
  `contract_schedules_bulk_operations` 1; unit suites of domain, application, infra-postgres, scheduler, api-http and
  execution-engine green; `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `check-architecture.py` and `check-agent-config.py` clean. Seen failing first: the race (a run started on revoked
  authority) and the database step (`NotFound`). The three race tests written after the fix (trigger-now, backfill,
  the waiting action) were run with `recheck_authority` switched off as a mutation: all six race tests failed, and the
  mutation was reverted. The race is made with a held row lock and `pg_stat_activity` (no sleeps): another transaction
  holds the schedule row, the action waits for it, the membership is revoked and committed, the row is released.
- [x] T018-022 [US1] Schedule ceilings and quotas (count incl. zero, entries, spacing, input bytes,
  round size) — `tests/integration/schedules-bulk-operations/limits.rs` — one test per row —
  FR-018-003, FR-018-045; NFR-018-003.
  *Done 2026-11-04.* Evidence: `cargo test -p sagat-tests --locked --test integration_schedules_bulk_operations limits::`
  — 11 passed; the whole suite 134 passed, also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`; `fault_schedules_bulk_operations` 11 passed;
  `cargo test -p sagat-scheduler -p sagat-execution-engine` green; `cargo fmt --check`, `cargo clippy --workspace --all-targets
  --locked -- -D warnings`, `check-architecture.py` and `check-agent-config.py` clean (the integration runs took 427-525 s on a
  loaded machine against 68 s on the previous day; no test timed out). One test per row of the plan's table that the first story
  meets: schedules per namespace with a limit of **zero** (a new one is refused with nothing stored, charged or audited —
  a before/after count of `schedules`, `schedule_revisions`, `payloads`, `audit_records`, `quota_transaction_deltas` — and the
  existing one still fires); spec entries 10 accepted / 11 refused and exclusions 10 / 11 with nothing stored; the hard
  ceiling of 32 entries holds whatever a namespace asks for; spacing 60 s accepted, 30 s `SCHEDULE_TOO_FREQUENT`, the 10 s
  floor; input exactly 64 KiB (65,536 bytes) accepted and one byte more `PayloadTooLarge` with nothing stored; note 512 / 513;
  remaining actions 1 and 1,000,000 accepted, 0 and 1,000,001 refused; a ceiling lowered after the fact refuses a new
  five-entry spec while the five-entry schedule made before it is read back and fires (NFR-018-003); and **round size**: the
  row "schedule actions per pass round per replica, 100 / 1,000" was not enforced by `SchedulePass`
  (the round-ceiling item of T018-017), so the test was seen failing first (five due schedules, a ceiling of three: the
  round took all five) and `ScheduleSettings::round_actions` (default 100, hard 1,000, `effective()`) now stops the round
  there; the next round carries on (3 + 2 steps, then 0; every schedule started once); `SAGAT_SCHEDULES_ROUND_ACTIONS`
  (1..=1,000, refused outside) in the engine config (`apps/execution-engine/tests/config.rs`), and a unit test of
  `effective()` (0 → 1, above 1,000 → 1,000). Not covered here, and not a US01 row: jitter (its test is in `service.rs`),
  catch-up and buffer ceilings (domain tests), backfill ceilings (`backfill.rs`), and the rows of US03..US06.

## Phase 4: US02 — Operate a schedule (P1)

- [x] T018-023 [US2] Pause/resume with note and audit — `crates/application/src/schedules.rs`,
  `crates/api-http/src/routes/schedules.rs` — paused times not caught up — FR-018-009.
  *Done 2026-10-30.* The service has paused and resumed since T018-016 (the note, actor and time stored with the schedule;
  the resumer becomes `authorizedBy`; audited without the note; a resume continues from the first time after it and
  catches nothing up); the routes are `pauseSchedule` and `resumeSchedule`. Evidence:
  `rest::pausing_and_resuming_keep_a_note_and_the_times`, `rest::roles_decide_who_may_read_write_and_operate`,
  `service::operators_pause_and_resume_with_a_note_and_the_next_time_follows_the_resume`,
  `service::pausing_needs_a_note_and_the_operate_permission` and
  `service::every_change_is_audited_with_who_did_it_and_never_a_payload`, the conformance scenarios of both operations
  (200, 403, 404, 422), also as the runtime role.
- [x] T018-024 [US2] Trigger-now (origin `MANUAL`, key `manual-{µs}`, paused, overlap override, replay, no
  decrement, no collision with a scheduled time at the same instant) —
  `tests/integration/schedules-bulk-operations/trigger.rs` — FR-018-004, FR-018-010.
  *Done 2026-10-30.* `ScheduleFiring::trigger` (the run is started as `system:schedule:<id>` under the role of the caller,
  D1800) and `POST /namespaces/{id}/schedules/{id}/trigger` (202, `triggerSchedule`), served by `ScheduleApi`, which
  puts the administration of a schedule and its actions behind the one `Arc<dyn ScheduleUseCase>`. The action is
  `TRIGGERED`/`MANUAL` at the request's microsecond, works while paused (the decision path of the store takes the
  paused lock for a manual action too), never uses up a remaining action (a waiting trigger is released without
  decrementing), leaves a time that is owed alone (`retry_at`), and is deduplicated: the `Idempotency-Key` is
  written to `command_dedupe` in the transaction of the action and the answer, first or repeated, is read back
  from what was stored. Evidence: `integration_schedules_bulk_operations` 107 passed (9 in `trigger`: paused
  start and skip, remaining actions, overlap rule and override, same instant as a scheduled time, replay and
  `IDEMPOTENCY_KEY_REUSED`, two requests with one key at once, roles/tenant/trigger-owned/deleted, an owed time, a
  waiting trigger; 3 more in `rest`), also as the runtime role; the conformance test validates the operation's
  202/400/401/403/404/422 and the battery sends it with a valid body. Removing the paused lock of the decision path,
  the preservation of `retry_at` or the no-decrement of a released trigger each makes exactly its test fail. Not done:
  `CANCEL_OTHER` still buffers without owing the open runs a cancellation (T018-030 owns the obligations); G1017.
- [x] T018-025 [US2] Backfill create/list/cancel and incremental pass steps with cursor, `ALREADY_ACTED`
  — `crates/scheduler/src/schedules.rs`, `crates/infra-postgres/src/schedules.rs` — crash resumes from
  cursor — FR-018-011.
  *Done 2026-10-30.* `ScheduleService::create_backfill`/`backfills`/`cancel_backfill` and the REST operations
  `listScheduleBackfills`, `createScheduleBackfill` (202), `cancelScheduleBackfill`; `ScheduleFiring::backfill_step`,
  which the pass (`SchedulePass`) runs for the backfills of the shards it holds. A step is one transaction: the first
  time of the range after the cursor, acted on under the overlap rule of the backfill (`BUFFER_ALL` by default), with
  the cursor, the counters and the state of the backfill committed with the run or the recorded decision (D1801). A
  time that already has an action is counted `ALREADY_ACTED` and never gets a second run; a backfill whose buffer is
  full waits instead of dropping times; one whose requester lost `run:start` is stopped; deleting the schedule stops
  its backfills. One backfill runs at a time on a schedule and a namespace has its `schedule_backfills` quota; a range
  holds at most 1,000 times. Evidence: `integration_schedules_bulk_operations` 123 passed (11 in `backfill`: the
  record and its defaults, `BACKFILL_RANGE_INVALID` and `BACKFILL_LIMIT`, the schedule and namespace limits, stopping
  is final and idempotent, roles and delete, the steps in order, `ALREADY_ACTED`, a paused schedule and a stopped
  backfill, a revoked requester, waiting instead of dropping; in `pass`, the pass works through the backfills of its
  shards and `a_backfill_resumes_after_the_committed_cursor_when_another_replica_takes_over`: two steps, the leases
  lapse, another replica finishes it, three runs for three times; in `rest`, two); the conformance test covers the
  three operations (202/200/422/429/403/404) and the battery sends them with valid bodies. Without the
  `ALREADY_ACTED` check its test fails. Not done: the Studio form (T018-032), the metrics of the pass.
- [x] T018-026 [US2] `SKIP`, `BUFFER_ONE`, `BUFFER_ALL`, `ALLOW_ALL`, open-run tracking, buffered start —
  `tests/integration/schedules-bulk-operations/overlap.rs` — SC-018-003 table — FR-018-006, FR-018-007.
  *Done 2026-10-30; the scenarios live in `firing.rs`, which already holds the fixture and the clock, and no
  `overlap.rs` was made.* The decision table is proved in the domain (`crates/domain/tests/schedules.rs` and the
  property test `schedules_properties.rs`, which found D1043) and end to end over PostgreSQL by
  `skip_records_the_time_it_skipped_while_a_run_is_open`,
  `buffer_one_keeps_one_waiting_drops_the_next_and_starts_the_waiting_one_when_the_run_ends`,
  `buffer_all_keeps_each_time_up_to_its_limit_and_starts_them_in_order` (limit 2: two wait, the third is
  `DROPPED_BUFFER_FULL`, the waiting ones start in the order the times came, each when the run before it ends),
  `allow_all_starts_beside_an_open_run` and `two_processes_acting_on_the_same_time_start_exactly_one_run`;
  tracking forgets an ended run and the buffered start releases the oldest waiting action. `integration_schedules_bulk_operations`
  109 passed, also as the runtime role. The tracking ceiling (`TRACKING_LIMIT`) is proved in the domain only.
  `CANCEL_OTHER` is T018-030.
- [x] T018-027 [US2] Remaining actions and `pauseOnFailure` — US02 scenario 6 — FR-018-007, FR-018-012.
  *Done 2026-10-30.* Evidence in `firing.rs`: `a_schedule_with_two_actions_left_stops_after_them` (the third time
  records nothing and the schedule has no next action), `a_failed_run_pauses_a_schedule_that_asks_for_it_with_a_system_note`
  (system note, `schedule.auto_paused` audit), `a_cancelled_run_does_not_pause_a_schedule_that_pauses_on_failure`; in
  `trigger.rs`, `trigger_now_does_not_use_up_a_remaining_action` and
  `a_waiting_trigger_starts_when_the_run_ends_and_uses_up_nothing` (only a scheduled start decrements).
- [x] T018-028 [US2] Describe (next N, recent, counters), actions route, action purge keeping `BUFFERED`
  — `crates/infra-postgres/src/maintenance.rs` — contract + purge tests — FR-018-013.
  *Done 2026-11-04 (the purge first, the route after).* The route `listScheduleActions` (`GET
  /namespaces/{id}/schedules/{id}/actions`, `schedule:read`, `cursor`, `limit`, `outcome`) lists the decisions of a schedule
  newest first, by the instant of the decision and then by the key of the action (origin, nominal time), so a page ends
  where the next begins even when several decisions were made in the same instant; an outcome this version does not know
  is refused 400 (the contract's enum), as is a cursor this build did not hand out. It is merged into the platform
  contract (`x-implemented: '018'`, `ScheduleActionPage`), the reference and the Studio types are regenerated.
  Evidence: `actions::` 3 passed in `integration_schedules_bulk_operations` (six decisions with a three-way tie on the
  instant, listed 1, 2, 3, 4 and 100 at a time: the same listing every time, none skipped, none twice, in the order of the
  key; one outcome at a time; another tenant is `NotFound` and a viewer reads); `action_cursor` 2 passed (what is handed
  out comes back, thirteen strings that were not are not cursors); the schedule contract test now exercises the twelfth
  operation (a page of one, the next page, the outcome filter, a viewer, another tenant, an anonymous caller, a bad cursor, a
  bad outcome, an unknown schedule); the cross-tenant battery covers it. Removing the tie-breaking from the keyset makes
  the paging test fail. The earlier work of this task follows. Nothing deleted actions. Now
  `schedule_purge_actions(schedules, rows)` keeps an action while it is among the newest 1,000 of its schedule or younger
  than 30 days and never deletes a `BUFFERED` one, runs through the housekeeping of the scheduler (a few schedules at a
  time, at most 5,000 rows a call) and visits schedules in turn (decision D1955, migration
  `20261230002000_schedule_action_purge`, gap G1954 for the revisions and tombstones). Evidence: `purge::` 7 passed in
  `integration_schedules_bulk_operations` (1,300 days of actions: the 300 beyond the last thousand go, the oldest first,
  and a second purge finds nothing; a schedule with 10 ancient actions keeps them; a waiting action 2,000 days old
  stays; with one schedule per call the busy schedule is reached although a quiet one sorts first; at most the rows
  given are deleted and the next call goes on; the runtime role may run the function and has no `DELETE` on the table;
  a rollback drops the function and applying again brings it back). Seen failing first against a stub (nothing was
  purged, the function did not exist), and the test of the rotation fails when the function orders by id alone.
  Gates on the final tree (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`): `integration_schedules_bulk_operations`
  166 passed and `fault_schedules_bulk_operations` 16 passed, each also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
  `integration_platform_foundation` 101 (migrations, runtime-privilege audit); `integration_security_multitenancy` 126;
  `integration_database_connections` 7; `contract_schedules_bulk_operations` 1; unit suites of domain, application,
  infra-postgres, scheduler, api-http and execution-engine green; `cargo fmt --check` and
  `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
- [ ] T018-029 [US2] Catch-up after a simulated outage (`ONE`, `ALL`, summary, trigger-owned 366 d) —
  `tests/integration/schedules-bulk-operations/catchup.rs` — SC-018-003 — FR-018-008.
  *Part done 2026-11-04 (task stays open: the trigger-owned schedule with its 366-day window is US07, T018-060).* The
  outage tests found that the limits of a recovery held for one slice of the backlog and never for the backlog itself
  (decision D1951, migration `20261230001000_schedule_recovery`): with a ceiling of 5 and 20 owed times, mode `ALL`
  started 20 runs and mode `ONE` started 3; a 14-day outage on a per-minute schedule never became idle in 50 steps (a
  step summarized 102 times); and a workflow with no published version was retried on every round with no backoff
  (D1952). Seen failing first, then fixed: a recovery is planned once and remembered on the schedule
  (`recovery_until`, `recovery_acts`), carried out one step at a time by the pure `plan_recovery_step`, summarized
  10,000 times at a step and counted past the slice. Evidence: `catchup::` 8 passed (ceiling in mode `ALL` and `ONE`;
  the limits kept between steps, read from the columns; a 20,000-minute outage with a one-day window in 3 steps and
  18,560 + 1 + 1,439 times accounted for; US01 scenario 5, a time owed until the window ends and then missed; no
  published version backs off; the migration refuses a rollback in the middle of a recovery, rolls back and applies
  again; the constraints of the two columns); `schedules_properties` 9 passed, among them
  `a_recovery_in_steps_keeps_its_ceiling_and_counts_every_time_once` (random backlogs, windows, modes and ceilings; a
  mutation that stops remembering the recovery fails it with "25 actions, at most 2"); `schedules` 45. Gates on the
  final tree (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`): `integration_schedules_bulk_operations` 152 passed,
  also with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (152); `fault_schedules_bulk_operations` 11, both ways;
  `integration_platform_foundation` 101; `integration_security_multitenancy` 126; `integration_database_connections` 7;
  `contract_schedules_bulk_operations` 1; unit suites of domain, application, infra-postgres, scheduler, api-http and
  execution-engine green; `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`
  clean. Gaps G1951 (the count stops after a million times) and G1952 (a refused backfill step has no backoff).
- [x] T018-030 [US2] `CANCEL_OTHER` obligations via the ordinary cancel use case; races with self
  completion and with failed compensation — `tests/fault/schedules-bulk-operations/cancel_other.rs` —
  first commit wins — FR-018-006, FR-018-007.
  *Done 2026-11-04 (decisions D1953, D1954; gap G1953; owner question O1950).* The domain already decided
  `Buffer { cancel_open }` and nothing acted on it: the action waited and the open run was never cancelled. Now the
  decision transaction records a debt per open run (`schedule_obligations`, key `schedule:{id}:cancel:{µs}:{run}`)
  with the waiting action and makes the schedule due at once; the pass pays the oldest debt before anything else,
  through `RunService::operate` (Cancel) as `system:schedule:<id>`, and records it `DONE` under the shard fence; the
  waiting action then starts by the existing release. The cancel of a run that ended by itself or is gone leaves the
  debt paid (first commit wins); `Forbidden` is the loss of the authority; the cancel is keyed, so a lease lost
  between the cancel and the record costs a no-op repeat. Cancelling runs needs `run:operate`: creating, updating
  or resuming such a schedule, a backfill that asks for the rule and the override of trigger-now need it, and so does
  every action (a developer cannot write one: O1950). Seen failing first (7 integration tests: no debt was recorded,
  nothing paid it, no permission was asked), then fixed. Evidence: `cancel_other::` 7 passed in
  `integration_schedules_bulk_operations` (the debt in the same transaction, paid through the ordinary cancel with
  one audited cancellation by the schedule, a run that ended by itself first, one debt per open run, trigger-now with
  the rule overridden, who may write the rule, an authority that can no longer operate runs pauses the schedule) and
  5 in `fault_schedules_bulk_operations` (a run that really finishes through a worker just before the cancel is left
  `SUCCEEDED` and uncancelled; a cancel that committed before the debt was recorded is not asked twice; a replica
  with a lost lease cannot record the debt and the new owner pays it once; a run that waits for review keeps the new
  time waiting and is not cancelled again; a compensation that failed pauses a schedule that pauses on failure). Two
  mutations were caught: recording the debt without the fence fails the lease test, and not treating `StateConflict`
  as paid fails the self-completion race. The failed-compensation outcomes are set on the run row (the saga
  machinery is proved in its own suites); G1953: a waiting action older than the catch-up window still starts when
  the runs end. Gates on the final tree (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`):
  `integration_schedules_bulk_operations` 159 passed and `fault_schedules_bulk_operations` 16 passed, each also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`; `integration_platform_foundation` 101; `integration_security_multitenancy` 126;
  `integration_database_connections` 7; `contract_schedules_bulk_operations` 1; unit suites of domain, application,
  infra-postgres, scheduler, api-http and execution-engine green; `cargo fmt --check` and
  `cargo clippy --workspace --all-targets --locked -- -D warnings` clean.
- [x] T018-031 [US2] Revisions on update; delete → `DROPPED_DELETED`, name freed —
  `tests/integration/schedules-bulk-operations/revisions.rs` — FR-018-014.
  *Done 2026-10-30; the tests are in `changes.rs`, `service.rs` and `firing.rs` (no `revisions.rs`).* Evidence:
  `changes::a_revision_moves_the_head_and_keeps_the_old_one`, `a_stale_expected_revision_is_a_conflict_that_changes_nothing`,
  `of_two_revisions_from_the_same_revision_exactly_one_wins`, `a_revision_can_rename_but_not_to_a_name_that_is_taken`,
  `deleting_leaves_a_tombstone_drops_waiting_actions_and_frees_the_name` (two waiting actions become
  `DROPPED_DELETED`), `a_deleted_schedule_cannot_be_paused_or_revised`;
  `service::an_update_is_a_new_revision_from_the_first_time_after_it`, `deleting_is_for_writers_and_leaves_nothing_to_read`,
  `a_delete_at_a_stale_revision_deletes_nothing_and_at_the_head_deletes`; and
  `firing::a_waiting_action_keeps_the_revision_it_was_taken_under` (an update while an action waits: the run starts under
  revision 1 and the head is 2).
- [x] T018-032 [P] [US2] Studio schedules (list, detail, form with server-computed next times,
  pause/resume, trigger, backfill preview; tokens, WCAG AA, tables) —
  `frontend/apps/studio/src/features/schedules/`, `frontend/apps/studio/src/api/endpoints.ts` —
  component tests — FR-018-047.
  *Done 2026-11-04, except the backfill preview (G1966).* `/schedules` (list, paged by cursor), `/schedules/new` and
  `/schedules/:id/edit` (one form: an interval or cron expressions, time zone, jitter, bounds, overlap and catch-up
  rules, limit on actions, note), `/schedules/:id` (the platform's own next times with the jitter shown, the open runs,
  the counters, the decisions by outcome and page, the backfills with start and cancel, pause, resume, run now, delete),
  a `Schedules` entry in the menu (`Shell.test.tsx`, `smoke.spec.ts`), a README. The Studio computes no time. Each control
  is behind the permissions the platform asks for; a retry keeps its idempotency key and a changed request has a new one;
  an edit saves against the revision it was opened at (the copy is never refetched under the person nor kept after the
  page); what the form does not show (catch-up window, buffer limit, DST rule, interval offset, pinned version, run
  timeout) is carried, not reset; a schedule with calendars, exclusions or several rules, or one a cron trigger owns, is
  not edited there; the input is never shown and a number the browser would round is refused.
  Evidence (`frontend/`): `src/features/schedules` 4 files, 84 tests passed (`model.test.ts` 26: the request, the draft,
  the words; `SchedulesPage` 9; `ScheduleFormPage` 15; `SchedulePage` 34), the whole Studio suite 95 files and 1,317
  tests passed, `npm run lint` clean, `tsc --noEmit` clean for the Studio, `npm run build` built, `npm run check:api`
  matches the contract. Mutation checks, each reverted: 11 of the model (a dropped `dst`, `bufferLimit` or offset, an end
  equal to the start, exhausted not first, the units tried smallest first, a future end, an array input, an eleventh cron
  expression, editable exclusions, a name counted in UTF-16) and 18 of the pages (a dropped permission, a delete for a
  trigger-owned schedule, a delete without its revision, an untrimmed note, a pause of a paused schedule, a key never
  kept and a key kept for a changed request, no `expectedRevision`, unpublished workflows offered, a form not a fresh
  copy, the problem ignored, a future end, a cancel for an ended backfill, the outcome filter not sent, recent actions
  asked for, the digest in full, a next time while paused, creating without `run:start`): all caught (the one that
  survived, the fresh copy, got its test). Not done: the backfill preview (G1966); WCAG AA is by the platform's tokens,
  role and label queries and the `jsx-a11y` lint rules, not by a contrast measurement (there is no axe harness in the
  Studio and no new colour was added); `npm run typecheck --workspace @sagat/e2e` needs the TypeScript SDK built first
  (`sdks/typescript/conformance/prepare.sh`) and was not run here, the one edit in `smoke.spec.ts` is a menu list.
- [ ] T018-033 [P] [US2] Schedule input disclosure (016 audiences, `no-store`, audit) —
  `tests/integration/schedules-bulk-operations/disclosure.rs` — per-level cases — FR-018-042.
  *Not started (2026-11-04).* It needs the route `discloseScheduleInput`
  (`contracts/rest.openapi.yaml`, `/schedules/{scheduleId}/input-disclosures`), so it changes the canonical OpenAPI, the
  generated reference and the Studio types and wires the route; it is done with the other REST leftovers.

## Phase 5: US03 — Business identifiers and start delay (P1)

- [ ] T018-034 [US3] `BusinessIdStore` (insert-or-lock, holder status, claim in creation transaction,
  lookup, `runs.business_id`, purge) — `crates/infra-postgres/src/{business_ids.rs,execution.rs}` —
  store tests — FR-018-018, FR-018-021, FR-018-022.
  *Part done (2026-11-05), the task stays open:* migration `20270103000000_business_ids` (the binding table, `runs.business_id`,
  grants for `sagat_app`, a down that refuses while a binding exists, storage classification, the `sagat_quota_row_delta`
  trigger); `crates/infra-postgres/src/business_ids.rs` claims inside `PgRunStore::create` in the Aggregate slot
  (`INSERT ... ON CONFLICT DO NOTHING`, else `SELECT ... FOR UPDATE`, holder status through the chain, takeover of a closed
  holder, a holder that is gone holds nothing); the run and every segment of a chain carry the copy (D1961, D1963). Real
  PostgreSQL, `integration_schedules_bulk_operations` 249 passed in both passes (the owner role and `sagat_app`), 15 of
  them in `business_ids.rs`; mutation checks killed the binding row not locked, a closed holder not replaced, the claim
  ignoring an open holder, the copy on the run not written, the claim skipped, a gone holder counted as open, the options
  left out of the fingerprint, a chain judged by its first run, and a handoff that drops the identifier; the one survivor is the plain `continue_as_new` copy (G1970).
  Not done: the lookup `GET .../business-ids/{id}` and the call of `business_ids_purge` (G1968, G1969).
- [ ] T018-035 [US3] Start options on `POST /runs` (`200` for `USE_EXISTING`), binding lookup, list
  filters, rerun `KEEP`/`DROP` — `crates/application/src/runs/business_id.rs`,
  `crates/api-http/src/routes/execution.rs`, `crates/api-http/src/dto.rs` — contract tests —
  FR-018-019, FR-018-020, FR-018-022, FR-018-040.
  *Part done (2026-11-05), the task stays open:* `POST /runs` takes `businessId`, `businessIdConflictPolicy` (`FAIL`,
  `USE_EXISTING`) and `businessIdReusePolicy` (`ALLOW_DUPLICATE`, `ALLOW_DUPLICATE_FAILED_ONLY`, `REJECT_DUPLICATE`); a held
  identifier is `409` with `BUSINESS_ID_IN_USE`, a remembered one `409` with `BUSINESS_ID_REUSE_REJECTED` and the instant it
  ends, `USE_EXISTING` answers `200` with the holder and creates nothing; an invalid identifier or option is `422` in
  words and nothing is bound; the holder is named only to a caller with `run:read` (D1964); the options join the
  idempotency fingerprint only when present (D1962); `GET /runs` filters by `businessId` and `businessIdPrefix` (a prefix has
  no wildcards). Code is in `crates/application/src/runs.rs` (`StartOptions`, `start_with_options`) and
  `crates/api-http/src/routes/execution.rs`, not in the `runs/business_id.rs` and `dto.rs` this line names. OpenAPI,
  generated Studio types and `docs/reference/api.md` follow; `contract_schedules_bulk_operations` 3 passed (one is the
  business-id contract on every answer), `integration` `business_ids_rest.rs` 6 passed.
  *Update 2026-11-05 (rerun):* the rerun's `KEEP`/`DROP` (FR-018-040, D1966) is built: `RunService::rerun_with`, the
  `businessId` of the operation request for `RERUN_AS_NEW` (422 for any other operation), the `dropBusinessId` of a batch
  `RERUN` job, a held identifier failing the item as `CONFLICT`. `business_id_rerun.rs` 8 passed (seen failing first: 5 of
  them, with the API stub that changed nothing), `business_ids_rest.rs` 9 passed (3 new over HTTP), 5 of 6 mutants killed and
  the survivor, a fingerprint marker for `DROP`, removed because it distinguished nothing. Not done: the lookup of a
  binding `GET .../business-ids/{id}` (G1969).
- [ ] T018-036 [US3] 32-way race, policy table in/after window, terminal-commit race, chain continuity,
  key replay/reuse, harness vs signal paths — `tests/{integration,fault}/schedules-bulk-operations/business_ids.rs`
  — SC-018-006 — FR-018-018..021.
  *Part done (2026-11-05), the task stays open:* in `business_ids.rs` 32 concurrent starts of one identifier give one
  open run, 32 over the binding of a closed holder give one new run, 32 with `USE_EXISTING` all get the one run; the policy
  table inside the window and the free identifier after it; a retry under the same key gets the same run and another
  identifier under that key is a different request; namespaces and tenants are independent; the tenant is charged once for
  the binding and once for the run; a chain holds the identifier until it ends and every segment carries it; a refused
  start, and a start the admission refuses after the claim, leave no binding. `fault_schedules_bulk_operations` (31 passed)
  proves that the transitions of a run that holds an identifier never touch a schedule, batch or binding table
  (`run_driven_locks.rs`), and `batch_locks.rs` accepts a plan with the binding row in the Aggregate slot, after the schedule
  row and before the runs it acts on (the order of D1961; it checks a declared plan, not the claim itself).
  Not done: the terminal-commit race with a start that is claiming, the harness-versus-signal paths, and the crash points of
  the claim in a fault matrix; T018-012's slot cases are still the ones that block the lock-order half of this task.
- [ ] T018-037 [US3] Start delay (events, fenced release pass with `apply_if`, cancel/deadline, signal
  refused, Update admitted, quota; with 022 run-level concurrency the release acquires the permit or
  enqueues the run, FR-022-018) — `crates/scheduler/src/passes.rs`, `crates/infra-postgres/src/execution.rs`,
  `tests/{integration,fault}/schedules-bulk-operations/delay.rs` — SC-018-007 — FR-018-023, FR-018-050.
- [ ] T018-038 [P] [US3] Studio business id, start fields, delayed status —
  `frontend/apps/studio/src/features/runs/` — component tests — FR-018-047.
  *Part done (2026-11-05), the task stays open:* the start dialog has the optional identifier (checked in words before
  anything is sent: 1 to 200 visible ASCII characters), the two policies once there is an identifier, only the policies that
  are not the defaults are sent, the key changes with the identifier, and a `200` is announced as "already has an open run,
  nothing was started" (the client now tells a caller the status of a success); the run list filters by the start of an
  identifier (debounced, in the address, followed when the address changes by itself, cleared with the other filters) and
  shows it on the row; the run header shows it as a fact; the rerun dialog of a run that has an identifier offers to keep it
  (the default) or to start without it, and a held identifier is told in words with nothing to refresh. `npm test` 1346 passed (96 files), `npm run lint`, `typecheck`,
  `build` and `check:api` clean; 14 mutants of this code, 13 killed and the survivor (a trim that the dialog already did)
  removed. Not done: the delayed status, which needs the start delay of T018-037. `npm run typecheck --workspace @sagat/e2e`
  needs the TypeScript SDK built first (`sdks/typescript/conformance/prepare.sh`) and was not run here.
- [ ] T018-039 [US3] Business-id canaries absent from logs, spans, metrics, audit reasons, outbox —
  `tests/integration/schedules-bulk-operations/privacy.rs` — SC-018-014 — FR-018-042.
  *Part done (2026-11-05), the task stays open:* `privacy.rs` takes a canary identifier through a start, a refusal, a returned
  holder, a cancellation and the refusal of a reuse, and finds it, as text and as the hex of its bytes, in no table of the
  schema except the binding and the run (audit trail, outbox, journal, dedupe records and payloads included), in no log line
  at TRACE level, in no metric label and in the text of no error. The probe line proves the capture was listening, and the
  tables are asserted after the run so a hit names the table. Not done: the spans exported over OTLP are not searched
  (G1971).

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

- [x] T018-045 [US5] Batch store, service and routes (create, preview, read, list, items, cancel) —
  `crates/infra-postgres/src/batch.rs`, `crates/application/src/batch.rs`,
  `crates/api-http/src/routes/batch.rs`, `crates/api-http/src/dto_batch.rs` — contract/store tests —
  FR-018-027, FR-018-028, FR-018-032.
  *Done 2026-11-04 (the executor that selects and applies is T018-046/047).* The store and the service are built
  (decision D1957): create (judged against the active jobs of the namespace and of the
  tenant under a fence, idempotent by key, the payload of a signal or an update kept as content and never returned, the
  audit record naming the action and never the justification), preview (counted up to `maxItems + 1` with a sample of 20
  in a read that stops after 5 seconds), read, list newest first with a cursor, items, and cancel (once, audited once, a
  finished job left alone). `batch::` 16 passed in `integration_schedules_bulk_operations`; the test of the fence fails
  (naming its stage) when the lock is removed; a concurrent race for the last place passes even without the lock, so it
  is kept as a check of the outcome and the fence has its own deterministic test. Gates on the final tree (private
  `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`): `integration_schedules_bulk_operations` 185 passed and
  `fault_schedules_bulk_operations` 16 passed, each also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
  `integration_platform_foundation` 101 (the runtime-privilege audit found the bare word of a statement stored as a
  value, now a flag); `integration_security_multitenancy` 126 (the quota views list the two new resources);
  `contract_schedules_bulk_operations` 1, `contract_rest_platform_foundation` 9, `contract_rest_security_multitenancy`
  17, `contract_developer_experience` 8; `integration_database_connections` 7; the unit suites of domain, application,
  infra-postgres, scheduler, api-http and execution-engine 1,202 passed, 0 failed (the inline test that fixes the number
  of quota resources found the new ones); Studio `npm run check:api` and `npm run typecheck` clean; `cargo fmt --check`
  and `cargo clippy --workspace --all-targets --locked -- -D warnings` clean. The gates found two failures on the first
  run and one more on the second, all of them tests that list every resource or every literal; none was a defect of the
  behaviour. `MARK_MANUAL_REVIEW` is the single-run `Hold`, which needs `run:operate` only.
  *The routes and the DTOs (D1959):* `POST /namespaces/{id}/batch-operations/preview`, `GET` and `POST
  /namespaces/{id}/batch-operations`, `GET /namespaces/{id}/batch-operations/{batchId}`, `POST .../{batchId}/cancel` and
  `GET .../{batchId}/items`, in `api-http/src/routes/batch.rs` and `dto_batch.rs`, over the `BatchUseCase` trait object
  (`application/src/batch_api.rs`), merged into the platform contract with `x-implemented: '018'` (six operations, the
  `Batch operations` tag, `BatchId` and eight schemas) and served only behind `SAGAT_BATCHES_ENABLED`. Evidence:
  `contract_schedules_bulk_operations` `batch::every_batch_operation_conforms_to_the_contract_on_its_normal_and_error_paths`
  (every one of the six operations, each status it documents and each body checked against its schema: previews with a
  filter, ids, a bad id, a bad status, a query refused, an unknown field; a job created and refused for a viewer, an
  anonymous caller, another tenant, a short reason, `UPDATE`, a missing step, a rate over 200, an unknown action and a
  missing key; reading, listing with a state and a bad cursor, items with a state, the job through another namespace of
  the tenant as 404; cancelling twice and then a job that ended as 409 `BATCH_NOT_CANCELLABLE`); removing the same-namespace
  check of the route makes it fail (the job is returned through the wrong namespace). The cross-tenant battery
  (`contract_rest_security_multitenancy` `battery::`, 2 passed) now creates a job of the victim and snapshots it, and every
  batch operation resists an administrator of another tenant; `integration_schedules_bulk_operations` `batch::` holds the
  new refusal of a finished job; `apps/control-plane/tests/config.rs` the strict flag; `contract_developer_experience` was
  blessed after reading the diff (six operations in a new group, 176 in all) and rerun without the switch; Studio
  `npm run api`, `check:api` and `typecheck` are clean. Gap G1960.
- [x] T018-046 [US5] Executor pass (horizon wait on `pg_snapshot_xmin`, keyset selection rounds, item order
  run → fence → job → item, rate slot, deferral, drain) with kill/restart, duplicate, lost-hint and
  late-commit faults — `crates/scheduler/src/batches.rs`, `tests/fault/schedules-bulk-operations/batch.rs`
  — SC-018-009 — FR-018-029..031, FR-018-033.
  *Done 2026-11-04 (SC-018-009 is proven in part: see the review below).* The executor itself is built (decision D1958): `BatchExecutor::step` selects, applies, drains, fails and ends a job
  one step at a time, over `BatchStore` (`due_jobs`, `select_round`, `next_item`, `record_item`, `complete_job`,
  `drain_cancel`, `fail_job`). A design defect of D1033 was found by a held transaction: `pg_snapshot_xmax` is the last
  completed identity plus one, so an open transaction with that identity let the selection start; the horizon is now the
  creator's own identity. Evidence: `batch_executor::` 12 passed in `integration_schedules_bulk_operations` (pages by id
  and `maxItems`; an empty job; the wait for a transaction open at creation; one item per slot with its effect, outcome
  and counters, the last item ending the job; a finished run and an unknown id skipped with their reasons; a crash between
  the effect and its record; the loss of the creator's permission; a cancel drained in pages; a refusal retried after a
  backoff and one that is not; two tests of a lease that was lost). Mutations that were caught: the original horizon
  (`xmax` with `>=`) fails the held-transaction test as `Acted` instead of `Idle`, and recording an item without the fence
  fails the stale-lease test. Gaps G1956..G1959.
  *Update 2026-11-04 (the pass):* `BatchPass` (`crates/scheduler/src/batches.rs`) is the pass over the jobs of the shards a
  replica owns, like `SchedulePass`: it asks the store for the due jobs (`state` selecting, running or cancelling, slot
  reached, earliest first), takes the lease of the shard of each job from the keeper's placement, and takes at most
  `steps_per_job` (20) steps for a job and `round_actions` (100, hard 1,000) steps over the round; a lost lease is a
  refused step (`scheduler_fenced_total{kind="batch"}`), a failing one is logged and counted. The engine runs it behind
  `SAGAT_BATCHES_ENABLED` (off by default; `SAGAT_BATCHES_BATCH` 1-5000, `SAGAT_BATCHES_ROUND_ACTIONS` 1-1,000,
  `SAGAT_BATCHES_POLL_MS`), pacing the items by the database's clock like the schedule pass. Evidence: 5 in
  `batch_pass::` (one replica selects and cancels the three runs of a job in 4 steps and each run is cancelled once; a
  replica with no shard does nothing; two replicas split twelve jobs by shard, take 2 steps for each job of their own
  shards and cancel every run once; a replica whose shard was taken is refused where it records and the job goes on when
  the shard is back; a round of one step takes one step and the next carries on), 2 unit tests of the ceilings, 2 in
  `apps/execution-engine/tests/config.rs`. Mutations that were caught: the pass without the fence fails the lease test
  (2 steps, 0 refused instead of 0 and 1), and a round without its ceiling fails the ceiling test.
  *The faults* (`tests/fault/schedules-bulk-operations/batch.rs`, 7 passed): a process killed after each write of the five
  transactions of the executor (a page of the selection 3 writes, an item 3, the last item 4, the drain of a cancelled
  job 6, the failure of a job whose creator lost a permission 4; the connection dies inside PostgreSQL at the Nth write)
  commits nothing (every table digest equal), leaves the job where it was, and a new executor started from the database
  alone ends it exactly once (one item for each run, counters adding up to the items, one `batch.finished` record, the
  key of each run unchanged); two steps at the same item, both waiting for the row of the job, count it once (one
  `Acted`, one `Refused`); a cancel that commits first while an item that was applied is being recorded loses neither
  (the item is counted `SUCCEEDED`, the other two `CANCELLED`, one end record). Mutations that were caught: dropping the
  "item already ended" guard (the item is counted twice), refusing to record into a cancelling job (`Refused` instead of
  `Acted`), and the audit of a drained job in a transaction of its own (the probe sees two transactions). No lost hint
  exists to test: a job is found by the pass's own poll, as a schedule is. The late commit of a run against the
  selection is the held transaction of `batch_executor::` (the selection waits for it); its residual window is G1957.
  Not part of this task and not done: fairness between tenants, the gauges (`scheduler_lag_seconds{pass="batches"}`,
  `batch_items_total`) and the monitor alert, which the schedule pass lacks too (T018-017).
  *After an independent read (D1960).* The kill matrix uses an action with no effect of its own, so it proves that the
  record of an item is atomic and not that the effect and the record are (G1961, O1951); the held-transaction test proves
  that the selection waits, not that a run the held transaction commits late is then selected; no lost hint exists to
  test. SC-018-009 is covered by `a_job_over_a_thousand_runs_applies_each_once_across_a_restart_and_a_cancel` (1,000 runs
  in two pages of selection, 400 items, a restart, 300 items, a cancel: 700 succeeded and 300 cancelled, every key applied
  once, the counters equal the item rows) and by the lock monitor of T018-049; the run whose creating transaction was in
  progress is the part not proven by a test (G1957, G1962).
- [ ] T018-047 [US5] Item adapters for the eight actions through single-run use cases, skip reasons (incl.
  `TEST_RUN`), failed compensation of cancelled runs — `tests/integration/schedules-bulk-operations/batch_actions.rs`
  — one case per action and reason — FR-018-030, FR-018-031, FR-018-034, FR-018-050.
  *Part done 2026-11-04 (task stays open for `UPDATE`, which a batch cannot apply yet (G1959), and `TEST_RUN`, which
  needs feature 022, not approved).* `ItemActions` is implemented for the run service in
  `application/src/runs/batch_items.rs` for cancel, pause, resume (`operate`), signal, retry-step and
  mark-manual-review (`review` with `Retry` and `Hold`) and rerun, each under the key of its item. Cases against real runs
  and workers, `batch_actions::` (4 passed) and the earlier `batch_executor::`: `CANCEL` of a running run and of one that
  ended (`ALREADY_TERMINAL`); `PAUSE` and `RESUME` of a run they fit and of one that ended or is not paused
  (`NOT_APPLICABLE`), with the `paused` flag checked; `SIGNAL` to two runs that wait for it (both woken) and to a run that
  waits for nothing and to a run already woken (`NOT_WAITING`, never buffered), the job showing that it has a payload (`hasPayload`), never its digest (D1960);
  `RETRY_STEP` of a compensation that ran out of attempts and waits for an operator (`reserve~c`: the run goes back to
  `COMPENSATING` with another attempt), of a run that has no such step (`NOT_WAITING`) and of a step whose outcome is
  unknown (the item `FAILED` with `CONFLICT`, nothing repeated, the run stays in review); `MARK_MANUAL_REVIEW` of a step
  that has not started (the run goes to `MANUAL_REVIEW`) and of a run without that step (`NOT_WAITING`); `RERUN` and a
  run that is not there (`RUN_CONTINUED`, `HIDDEN_FROM_CREATOR`). Real defects found by these cases and fixed: a run without
  the named step was reported `HIDDEN_FROM_CREATOR` (the adapter now reads the run first, so a `NotFound` after that means
  "no such step"), and a batch could not address a compensation (`reserve~c`) because the adapter read the step id with
  the strict parser; a step id that is no node is now `BATCH_ACTION_PARAMETERS` at `/stepId` when the job is created.
  Mutations that were caught: the strict parser, the missing `NotFound` mapping, every non-cancel skip as
  `ALREADY_TERMINAL`, and a missing signal wait as `NOT_APPLICABLE`.
- [x] T018-048 [US5] Batch authorization and ceilings (no `run:batch`, missing action permission,
  mid-job revocation, foreign ids, job/item ceilings, zero quota) —
  `tests/integration/schedules-bulk-operations/batch_security.rs` — SC-018-010 — FR-018-033, FR-018-034,
  FR-018-041, FR-018-045.
  *Done 2026-11-04.* The cases are spread over four files of `integration_schedules_bulk_operations` and each is a named
  test: `batch::creating_a_job_needs_run_batch_and_every_permission_of_the_action` (the role matrix over every action: a
  viewer or a developer is refused, an operator and an administrator are not, and `MARK_MANUAL_REVIEW` needs only
  `run:operate`); `batch_executor::a_creator_who_loses_a_permission_fails_the_job_and_nothing_more_is_applied`
  (mid-job revocation: the job fails `AUTHORITY_REVOKED` and the items that had not run are cancelled);
  `batch::active_jobs_are_limited_per_namespace_and_a_limit_of_zero_refuses_before_any_work`,
  `batch::the_active_jobs_of_a_tenant_are_limited_too` and
  `batch::a_creation_waits_for_the_fence_of_the_active_jobs_before_it_counts_them` (the ceilings of active jobs, judged
  under a fence);
  `batch::parameters_the_action_does_not_take_and_limits_out_of_range_create_nothing` (`maxItems` over 100,000, a rate over
  200, `REPIN` and `UPDATE` unavailable); `batch::jobs_are_read_listed_newest_first_and_never_across_tenants` and the
  cross-tenant battery. New in `batch_security::` (2 passed): a job names the ids of a run of another namespace and of
  another tenant besides its own, the preview counts one, the job selects one, the other two are untouched and never
  become items, not even skipped ones that would say they exist; a limit of zero, for the namespace and then for the
  tenant, admits no job and leaves nothing stored. Mutations that were caught: the selection without its tenant and
  namespace conditions (the preview counts three runs instead of one), and a limit of zero read as one. Not covered: the
  `deployment:manage` permission of `REPIN`, which does not exist before feature 019.
- [x] T018-049 [US5] Lock monitor (`pg_locks` sampling: never two run locks) and harness interleaving with
  API cancel/signal — `tests/fault/schedules-bulk-operations/batch_locks.rs` — zero deadlocks —
  FR-018-030; NFR-018-001.
  *Done 2026-11-04.* `batch_locks::a_batch_job_and_the_api_on_the_same_runs_never_hold_two_run_locks_or_deadlock`: 24 open
  runs, two tasks that pause, resume and signal them through the API for as long as the test lasts, three rounds of a
  `PAUSE` job and a `RESUME` job over the same runs at the top rate, then the API cancelling the runs from the last to the
  first while a `CANCEL` job works through them; the monitor looked 1,157 times (one run of the test), saw run locks held and
  never more than one by one transaction, the API made 1,008 calls and met nothing but a state that was not the one
  asked for, the seven jobs ended `COMPLETED` with counters equal to their items and no failure (a skipped item is
  `ALREADY_TERMINAL` and nothing else), every run is `CANCELLED`, and `pg_stat_database.deadlocks` did not move. The test
  is not vacuous: its monitor and its deadlock are shown by the self-tests of T018-012 (it sees two run locks held at
  once, and the harness reproduces a deadlock); no mutation of the platform could be made to hold two run locks,
  because no path of the platform does. *After an independent read:* the batch executor takes no run lock of its own (the
  run service does, one run at a time), so the monitor observes the run service under batch load; the order of the batch
  side (job, then item, in `record_item`, `drain_cancel`, `fail_job` and `select_round`, and the job alone in
  `request_cancel`) is by construction and is exercised by the duplicate-step and late-cancel tests of T018-046, not by
  this monitor.
- [x] T018-050 [P] [US5] Studio batch (from run-list filter, preview, confirm with reason, progress,
  items, cancel) — `frontend/apps/studio/src/features/batch/`, `frontend/apps/studio/src/features/runs/runList.ts`
  — component tests — FR-018-047.
  *Done 2026-11-04.* `features/batch/`: `BatchFromRuns` (a "Batch operation…" button in the toolbar of the run list, shown
  only with `run:batch`, which turns the status, workflow and origin filters into the selector of a job: choose an action,
  count the runs, give a reason, confirm; nothing is asked of the platform before a person acts, the job starts only after
  the runs were counted for the same limit and a reason of 8 to 2,000 characters, an unchanged request keeps its
  idempotency key, a signal needs a JSON object within 256 KiB, a step-based action names its step), `BatchesPage`
  (`/batches`, newest first, a state filter, load more, followed every 5 s while one job is at work) and `BatchPage`
  (`/batches/:batchId`: the state in words and a shape, a progress bar with "25 of 40 runs have an outcome (62%)", the
  counters, the outcome of each run with the reason in words or the error code, a filter by outcome, load more, followed
  every 2 s while the job works, the payload never shown, nor its digest, only that the job has one; "Cancel the job" with `run:batch`, a reason
  and the plain statement that what was applied stays applied). A deployment with batch operations off (`ROUTE_NOT_FOUND`)
  is told so. Routes `batches` and `batches/:batchId`, a "Batches" entry in the menu, six endpoints in
  `api/endpoints.ts`. The pure rules are in `model.ts` and tested alone. Evidence, from `frontend/` in CI's order: `npm run
  lint` clean, `npm run typecheck` clean, `npm test` 91 files and 1,233 tests passed (39 new in `features/batch/`: 14 of the
  model, 10 of the dialog, 9 of the job page, 6 of the list), `npm run build`, `npm run check:api`. Six mutations were caught,
  each by its own test: the dialog without its "count first" guard, the count kept after the limit changed, the dialog
  offered without `run:batch`, the cancel button without `run:batch`, the cancel form without its reason check, and the
  outcome filter not sent. Not run here: `npm run typecheck --workspace @sagat/e2e` needs the TypeScript SDK built
  (`bash sdks/typescript/conformance/prepare.sh`) and fails without it for reasons unrelated to this change, and the
  browser journeys (Rust-driven, ignored without Node and Chromium) have no journey for batches. The filter of the run
  list (`runList.ts`) is not changed: the dialog reads the filters the page already holds.
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
  *Part done 2026-11-05 (task stays open: the OTel export canary, the audit rollback, alert kinds and the quota exhaustion
  per resource).* Label closure: `counted::` reads the Prometheus page of the schedule and batch series and fails on any
  label that is not `kind`, `outcome`, `action` or the histogram's `le`. Alerts: the lag of the schedule and batch passes
  rides on `SagatSchedulerLate` (`max(scheduler_lag_seconds) > 60`, no `pass` matcher), whose summary now names them, and a new
  `SagatBatchJobsStalled` (`batch_jobs_active > 0` with no item outcome for fifteen minutes: jobs accepted where no engine
  runs them, G1965, or a selection waiting for an old transaction) points to `batch-operations.md#5-when-a-job-does-not-move`.
  The new rule is tested with `promtool test rules` (`infra/observability/tests/batch_stalled_test.yaml`, run by CI after
  `check rules`): it fires after fifteen minutes with jobs active and a flat counter, stays silent while items get outcomes
  and while no job is active, and fires when the counter has never existed. Also: `promtool check rules` accepts the 25
  rules (the CI image, run here), `contract_observability_operations` 9 passed (every metric a rule evaluates is one the
  code emits, every rule has a runbook section that exists). The separate `SagatSchedulesLate` of the plan is not added:
  the generic rule already covers the pass.
- [ ] T018-072 Benchmark on a declared environment (firing lag at 1k/10k schedules, batch rate per job
  and 16 jobs, 32-way business-id claim, reset at 1k/20k events); adjust ceilings only from data —
  `tests/performance/schedules-bulk-operations/`, `tests/performance/reports/schedules-bulk-operations.md`
  — report with failures — NFR-018-005.
- [ ] T018-073 Runbooks/docs (`docs/runbooks/{schedules,batch-operations,run-reset}.md`, cron section of
  `docs/runbooks/events.md`, entry-wait pattern, business-id privacy warning) — review + link check —
  FR-018-049.
  *Part done 2026-11-05 (task stays open: `run-reset.md`, the cron section of `events.md`, the entry-wait pattern and the
  business-id warning belong to US03, US06 and US07).* `docs/runbooks/schedules.md` (what a schedule is and the nine
  decisions a time can become, the two flags that must agree and their settings, the order to look in when a schedule did not
  start a run, the signals, the input that is never returned, what is not built) and `docs/runbooks/batch-operations.md`
  (what a job promises and what it does not, the limits, the two flags, how to read a job and its items, what to look at when
  a job does not move, the signals, what is not built). Every setting, default, reason code, audit action and metric they name
  was read from the code (engine and control-plane settings, `Limits`, the quota defaults, the audit table, the roles); a link
  check of the relative links of both found none missing, and `check-design-docs.py` and `check-agent-config.py` pass. Not
  done: a review by an operator.
- [ ] T018-074 Real-browser e2e of schedule, batch and reset flows, keyboard only, WCAG AA, failure states
  — `tests/e2e/schedules-bulk-operations/` — SC-018-015 — FR-018-047.
- [ ] T018-076 Barrier test: every run-driven transition (completion, failure, timer, wait timeout,
  deadline, terminal, handoff) against each aggregate path; `pg_locks` never shows a run-driven
  transaction on schedule, business-id, job, item, obligation or pending-reset rows —
  `tests/fault/schedules-bulk-operations/run_driven_locks.rs` — SC-018-016 — FR-018-007, FR-018-021,
  FR-018-030.
  *Part done 2026-11-05 (task stays open: business-id and pending-reset rows, the wait timeout and the handoff).* The test
  holds `ACCESS EXCLUSIVE` on every table of the two aggregates that exist (`schedules`, `schedule_revisions`,
  `schedule_actions`, `schedule_backfills`, `schedule_obligations`, `batch_jobs`, `batch_items`, found by name so that one
  added later is locked too; each is shown to refuse a statement with `lock_not_available` after 300 ms), which stops any
  statement that reads or writes them, and drives the transitions a run goes through: a worker completes an attempt and the
  step it unlocks (the run succeeds), a worker fails an attempt for good (the run ends), a person cancels, and the
  scheduler's passes fire a timer (the step behind it is ready), take back a lease that ran out and enforce a run
  deadline. Every one finishes while the locks are held, so none of them reads or writes a row of an aggregate (a run
  learns nothing from them and they learn of a run by looking, ADR-0050 decision 7). The control: the firing of a due
  schedule does wait for the same locks (seen in `pg_stat_activity`, not by a delay) and acts once they are released.
  Evidence: `fault_schedules_bulk_operations` `run_driven_locks::` 1 passed, the suite 31 passed, also with
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`; a mutation that makes the transaction a run-driven change begins read the schedule that
  tracks the run fails it with "stage `a worker completes` did not finish within 60s", naming the transition that waited.
  Not covered: the wait timeout and the handoff transitions, and the rows of business ids and pending resets, which
  exist when US03 and US06 do.
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
- [ ] T018-081 [P] Rust SDK (D1040) — `sdks/rust/src/`, `sdks/rust/conformance/` — unit tests + scenarios —
  FR-018-048.
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
  follow T018-062 and the merged contract; T018-068 needs all six (T018-081 follows T018-062 like the other SDK tasks). T018-076 follows US05/US06;
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

Task count: 81 (80 published + T018-081 added by D1040; Setup 2, Foundational 10, US01 10, US02 11, US03 6, US04 5, US05 7, US06 8, US07 2,
Polish and cross-feature 19).
