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
- [x] T018-006 [P] Domain reset and events `run.start_delayed`, `run.start_released`, `run.reset`,
  `wait.satisfied.via`; eligibility over `RunState::replay` + suffix facts; `ResetPlan` —
  `crates/domain/src/execution/{reset.rs,event.rs,mod.rs}` — per-cause tests — FR-018-023, FR-018-035..039.
  *Deferred 2026-10-29 (reordered, not dropped).* It is not on the US01 path (it serves US03 start delay, US04 `via`
  and US06 reset) and it adds variants to `RunEvent`, which is matched exhaustively in about nine crates (event codec,
  projection, DTOs, telemetry) and in `execution/{mod,state}.rs`, files another session was changing for 014 (PR #59 has
  since merged). It follows US01 and US02, after a fresh merge of `develop`.
  *Part done 2026-11-05, the task stays open:* the two events of the start delay, `run.start_delayed` and
  `run.start_released` (D1968), are built with their reducer, command, planner hold and event codec: `start_delay.rs` in
  `crates/domain/tests` 8 passed, and a replay of the journal equals the stored projection (`verify_run`) in every delay test.
  *Done 2026-11-05 (D1975, D1976):* `wait.satisfied.via` (T018-043: `RunEvent::WaitSatisfied { node, via }`, written by
  the signal, the chain signal, the event dispatch and Signal-with-Start), `run.reset` (`RunEvent::RunReset`, its reducer
  arm, `ResetReapply`, the event codec with a closed detail and a round-trip test) and, in `reset.rs`, the eligibility of
  a reset over the addressed journal (`reset_points`) and `ResetPlan` (`reset_plan`). Evidence: `cargo test -p
  sagat-domain --test reset` 15 passed and the module unit tests (2), `event_codec` unit tests 26 passed, fifteen mutants
  of the module and the reducer killed, clippy over the workspace.
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
  *More done 2026-11-09 (task stays open: the cases of 019 (not approved) and 022 (its slot does not exist), of the batch item, of the reset
  `REQUIRE_TERMINAL` and of with-start and update-with-start, G2029).* `against_at(start)` (`support/lock_order.rs`) runs a real path against a reference
  transaction that holds the lock `start` of a list taken in the order of the slots, waits until the path is blocked by it (`pg_blocking_pids`, not a time), takes the
  locks after it and commits; a path that held, while it waited, a lock the reference takes later would hold what the reference asks for and PostgreSQL would break the
  cycle with `40P01`, read on both sides. `slot_order::` runs each case once for **every start** (state of its own each time), so every pair of locks of a list is ordered
  and not only the first against the rest (D2089; the first version ordered only the first: moving the authority check after the quota locks stayed green).
  Self-tests: a path in the order completes, one against it is a deadlock with exactly one victim, the start 0 does not see a swapped pair the start 1 does, a path that never
  meets the reference fails by name. Cases (`fault_schedules_bulk_operations` `slot_order::`, 12 passed): the scheduled action, the release of a buffered time and a
  backfill step (the schedule row, [the backfill row,] the authority `FOR UPDATE` on the memberships, which conflicts with the admission's `FOR SHARE`, the two active-run quota
  scopes, the usage of the tenant), trigger-now, the payment of a `CANCEL_OTHER` debt (run, debts, usage), the release of a delayed start (run, usage), the claim of a business
  identifier (binding, quotas, usage: a person's start has no authority lock), and the request (source, pending-resets quota, usage) and the completion (entry, source, authority,
  quotas, usage) of a pending reset. What it does not prove: a lock no reference names, a path it does not run, an interleaving of more than two transactions; and it is a
  **release gate** (a pull request runs the fault suite only when its files change). The lease row of a shard is in no reference: it is a **leaf lock** (D2083), guarded by
  `scripts/check-architecture.py` (the rule, its unit tests, and two hand mutants on the tree, `holds_shard` `FOR UPDATE` and an acquire without `SKIP LOCKED`, each red); the
  deadlock the first case met was the harness's own (its reference took the lease `FOR UPDATE` and then a run row), and `Slot::RunPassFence` is gone. The per-start run found a real
  order that differed from plan row 8: the step of a backfill took the backfill row after the admission; it takes it in slot 3 now (D2088, red first). Two hand mutants in
  `RunStore::create`, the quota before the authority and the memberships before the schedule row: 4 of the 9 cases red each, restored.

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
  *Rest done 2026-11-08 (D2040):* the trigger-owned schedule's catch-up, `catchup::trigger_owned` 2 passed (both roles): an outage of 40 days of a daily trigger is
  one run at the earliest missed time under the key `cron-<epoch of that time>` and one `COALESCED` summary of the rest, and the legacy pass makes the same run
  with the same key of the same outage; an outage of 400 days records `MISSED_CATCHUP_WINDOW` (34 times) for what is older than 366 days, fires the earliest time inside the
  window and summarizes the rest. The task stays unticked: the reviews of T018-060 and T018-061 are pending.
  *More done 2026-11-09:* the edge of the window and the legacy pass over an outage longer than it, `catchup::trigger_owned` 5 passed. An outage that began
  365 days ago is one run at the earliest time and one summary (`STARTED`, `COALESCED` of at least 364), no time missed; one that began 366 days ago
  (the midnight of that day is older than the window by the hours of today that have passed) records one `MISSED_CATCHUP_WINDOW` of count 1 and acts on
  the next time; and the legacy pass over an outage of 400 days makes one run at the oldest tick, a year late, with the key `cron-<that tick>` (D2040),
  as the schedule pass makes one run, so the handover never makes a catch-up bigger than the legacy one. The spec's 366 days is the code's
  (`MAX_CATCHUP_WINDOW_MS` = 31,622,400,000 ms, `trigger_policies`): no finding. A trigger-owned schedule has the policy `ONE` only (its policies cannot be
  edited, `SCHEDULE_TRIGGER_OWNED`), so `ALL` has no trigger-owned case. The tests read the clock once (`midnight_from(now, days)`, whole seconds; the key `cron-<epoch>` is compared as a number, never the text of an
  instant that went through the database with one that did not: microseconds on PostgreSQL and macOS, nanoseconds on Linux), and the 400-day case asserts the time acted on (the midnight of 365 days ago), not only that
  there is one run. One hand mutant, the window of `trigger_policies` one day shorter: 4 of the 5 red, restored. The task stays unticked for the reason above (the reviews of T018-060 and T018-061 are pending).
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
- [x] T018-033 [P] [US2] Schedule input disclosure (016 audiences, `no-store`, audit) —
  `tests/integration/schedules-bulk-operations/disclosure.rs` — per-level cases — FR-018-042.
  *Done 2026-11-05 (D1973):* `POST /namespaces/{namespaceId}/schedules/{scheduleId}/input-disclosures`. Service tests
  `disclosure::` (6) and REST tests `disclosure_rest::` (3) pass against a real PostgreSQL: each level to the audience that
  may see it (a viewer the public values, a developer also the internal ones, nobody a secret, and no canary in a debug
  print of the answer), personal data only with the explicit reveal and the grant (`PII_REVEAL_NOT_GRANTED`, the refusal
  audited, each granted reveal its own record), the audit `schedule.input_disclosed` for an internal read and none for a
  public one, the classification of the version the schedule pins against the one it follows, no schema to classify by
  (structure for an inspector, nothing for a reader), another tenant, another namespace, a deleted and an unknown schedule
  all `404`, `private, no-store`, the 1 KiB limit, an ordinary read of the schedule with no value in it, one token of the
  namespace per request. Contract: path and schemas in the platform OpenAPI, reference blessed, Studio types regenerated
  (`npm run check:api` passes), battery, runbook, and eleven answers of the live conformance test
  `contract_schedules_bulk_operations rest::` (200 for an administrator and a viewer, 403 for a reveal nobody is granted,
  422/400 for a request the contract does not take, 413 over 1 KiB, 401, 404 for another tenant and an unknown schedule),
  which now counts thirteen implemented schedule operations. Gates: `integration_schedules_bulk_operations` 337 and
  `fault_schedules_bulk_operations` 37 in both modes, `integration_payload_classification` 41 (the evaluator
  refactor) and `integration_platform_foundation` 101 and `integration_security_multitenancy` 126 in both modes,
  contract targets, clippy over the workspace. Mutants (9, over the query filters, the pinned version, the reveal grant, the audit action of a
  reveal, the reveal parsing, admission, the 1 KiB limit and the permission of the policy): 8 killed (the deleted
  schedule, the namespace filter, the pinned version, the grant taken as given, the collapsing reveal, the reveal never
  asked for, the missing admission and the missing body limit); 1 equivalent today: the policy reading with `run:read` in
  place of `schedule:read`, because every built-in role has both or neither (`Role::permissions`), so no role can tell
  them apart; the day a role splits them that line needs a test.

## Phase 5: US03 — Business identifiers and start delay (P1)

- [x] T018-034 [US3] `BusinessIdStore` (insert-or-lock, holder status, claim in creation transaction,
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
  *Update 2026-11-05 (lookup):* the lookup is built (`GET /namespaces/{ns}/business-ids/{id}`, D1967): the store reads the
  holder, the chain's current run, how it ended and `reusableAfter` without a lock, and answers nothing in the three cases in
  which a start finds the identifier free. `business_id_lookup.rs` 6 passed (open holder, closed holder with its end and
  window, past the window, unbound / purged holder / another namespace, a viewer, another tenant's principal with the
  victim's namespace id), and the chain test checks the lookup on every segment. 7 of 9 mutants were killed on the first
  pass; the 2 survivors each got a test and were then killed: the store's tenant filter
  (`the_binding_of_a_tenant_is_not_read_by_another_even_with_its_namespace_id`) and the route's admission
  (`a_binding_lookup_consumes_one_token_whether_it_finds_the_identifier_or_not`: the lookup debits the namespace's bucket
  once, found or not).
  *Update 2026-11-05 (purge):* housekeeping calls `business_ids_purge` (G1968, closed): `business_ids_purge.rs` 4 passed in
  both modes (only a binding that holds nothing is purged: open, closed inside its window, closed past it, a holder that
  is gone; a purge is bounded and goes on where it stopped; housekeeping makes it; the runtime role may run the function and
  has no `DELETE` of its own), seen failing first with a stub that purged nothing.
- [x] T018-035 [US3] Start options on `POST /runs` (`200` for `USE_EXISTING`), binding lookup, list
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
  the survivor, a fingerprint marker for `DROP`, removed because it distinguished nothing.
  *Done 2026-11-05 (ticked):* the last two parts landed. The binding lookup `GET /namespaces/{ns}/business-ids/{id}`
  (`getBusinessIdBinding`, D1967; over HTTP in `business_ids_rest.rs`: open and closed, an identifier with `/ % # ? + &` in it,
  unbound, a namespace that is not one, another tenant's probe finds nothing and is recorded; the cross-tenant battery gives
  the victim a real identifier and attacks the operation; the contract test checks its 200 and 404 against the schema) and
  the `scheduleId` filter of `GET /runs` (`rest::the_runs_a_schedule_started_are_listed_by_it`, seen failing first: the
  filter was ignored and 3 runs came back). The code is in `RunService::start_with_options`, `rerun_with` and
  `business_binding` and in `crates/api-http/src/routes/execution.rs`, not in the `runs/business_id.rs` and `dto.rs` this
  line names (D1961). Ticked from: `contract_schedules_bulk_operations` 3, `contract_rest_security_multitenancy` 18,
  `business_ids_rest.rs` 13, `business_id_rerun.rs` 8, `business_id_lookup.rs` 6.
- [x] T018-036 [US3] 32-way race, policy table in/after window, terminal-commit race, chain continuity,
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
  *Done 2026-11-05 (ticked):* the rest of the line landed.
  - The terminal-commit race (`business_ids.rs`): a start beside the end of its holder, written and not committed, does not
    wait for it and finds it open (`BUSINESS_ID_IN_USE`), and takes the identifier over once the end commits; a claim in
    flight, holding the row of the binding, blocks no transition of the run that holds it. A mutant that makes the claim lock
    the holder it reads is killed (the start waits, and the stage limit names it).
  - The signal and the other run-driven paths (`run_driven_locks.rs`, SC-018-016): with every table of the aggregates,
    `business_ids` included, held in `ACCESS EXCLUSIVE`, a signal wakes a waiting run, a wait runs out and takes its
    other way, and a chain hands off to its next segment, which carries the identifier, all without waiting. Mutants that make
    a signal or the production hand-off read the binding table are killed; the one on the plain `continue_as_new` copy
    survives, which is G1970.
  - The crash matrix (`fault_schedules_bulk_operations` `business_ids.rs`): a start under an identifier is one transaction of
    30 writing statements on 13 tables; the connection is killed after each in turn, and each time no table has changed,
    there is neither a binding nor a run, and the same start sent again makes exactly one run that holds the identifier and a
    third is a replay of it. A first start is made before the probe: its first counters are writes that later starts do not make.
  `T018-012`'s slot cases for the admission paths of 019, 021, 022, 024, 025, 026 and 030 stay T018-012's, as those paths exist.
- [ ] T018-037 [US3] Start delay (events, fenced release pass with `apply_if`, cancel/deadline, signal
  refused, Update admitted, quota; with 022 run-level concurrency the release acquires the permit or
  enqueues the run, FR-022-018) — `crates/scheduler/src/passes.rs`, `crates/infra-postgres/src/execution.rs`,
  `tests/{integration,fault}/schedules-bulk-operations/delay.rs` — SC-018-007 — FR-018-023, FR-018-050.
  *Part done 2026-11-05, the task stays open:* a start with `startDelayMs` (1 ms to 366 days) creates the run `WAITING`
  with its entry held and `startDelayedUntil` on the run, a fenced pass of the run shards releases the entry once at the
  instant (D1968; migration `20270104000000_start_delay`, `RunStore`/`MaintenanceStore::due_start_releases`, the pass
  `start_releases` with the condition read again under the run's lock), and a cancellation or a deadline ends it with no step
  having run. `delay.rs` (integration) 9 passed in both modes: held entry and no task offered, released once and not
  before, cancelled, past its deadline, a signal refused and then delivered, the active-run quota and the identifier,
  the range in words, the key and its replay, and the race with a run that stopped qualifying between the candidate query and
  the lock; `delay.rs` (fault) kills the release after each of its writing statements and the next pass releases it once;
  the REST start and its refusals in `business_ids_rest.rs`. All 13 mutants of the domain, the store, the pass and the
  application were killed (one needed the race test). Not done: a declared Update on a held run (G1972) and the release that
  acquires or enqueues 022's run-level permit (blocked on feature 022, which is not approved).
- [x] T018-038 [P] [US3] Studio business id, start fields, delayed status —
  `frontend/apps/studio/src/features/runs/` — component tests — FR-018-047.
  *Part done (2026-11-05), the task stays open:* the start dialog has the optional identifier (checked in words before
  anything is sent: 1 to 200 visible ASCII characters), the two policies once there is an identifier, only the policies that
  are not the defaults are sent, the key changes with the identifier, and a `200` is announced as "already has an open run,
  nothing was started" (the client now tells a caller the status of a success); the run list filters by the start of an
  identifier (debounced, in the address, followed when the address changes by itself, cleared with the other filters) and
  shows it on the row; the run header shows it as a fact; the rerun dialog of a run that has an identifier offers to keep it
  (the default) or to start without it, and a held identifier is told in words with nothing to refresh. `npm test` 1346 passed (96 files), `npm run lint`, `typecheck`,
  `build` and `check:api` clean; 14 mutants of this code, 13 killed and the survivor (a trim that the dialog already did)
  removed.
  *Done 2026-11-05 (ticked):* the delayed status landed with the start delay. The start dialog has an optional "Start after"
  (a whole number of minutes, hours or days, at most 366 days, checked in words before anything is sent, sent as
  `startDelayMs`, part of what a key stands for, and announced as "waits 1 hour before it starts"); a waiting run that has
  `startDelayedUntil` says "Starts <time>" under its id in the list, and its header says so as a fact with the words "held
  by its start delay" instead of the generic wait. `npm test` 1358 passed (97 files), `lint`, `typecheck`, `build` and
  `check:api` clean. `npm run typecheck --workspace @sagat/e2e`
  needs the TypeScript SDK built first (`sdks/typescript/conformance/prepare.sh`) and was not run here.
- [x] T018-039 [US3] Business-id canaries absent from logs, spans, metrics, audit reasons, outbox —
  `tests/integration/schedules-bulk-operations/privacy.rs` — SC-018-014 — FR-018-042.
  *Part done (2026-11-05), the task stays open:* `privacy.rs` takes a canary identifier through a start, a refusal, a returned
  holder, a cancellation and the refusal of a reuse, and finds it, as text and as the hex of its bytes, in no table of the
  schema except the binding and the run (audit trail, outbox, journal, dedupe records and payloads included), in no log line
  at TRACE level, in no metric label and in the text of no error. The probe line proves the capture was listening, and the
  tables are asserted after the run so a hit names the table.
  *Done 2026-11-05 (ticked):* the spans are searched too. The capture (`capture_with_spans` in `tests/support/logs.rs`) writes
  every span with its fields when it is created and when it closes, whether or not an event happens inside it, which is the
  attribute set a trace exporter sends; the test proves it was listening with a span of its own, and the flow now includes the
  lookup. A mutant that creates a span carrying the identifier is killed. What is left of G1971 is the wire conversion of an
  OTLP exporter, which carries the same fields and belongs to T018-071's export canary.

## Phase 6: US04 — Signal-with-Start and Update-with-Start (P2)

- [ ] T018-040 [US4] Signal-with-Start (holder signal, creation delivery, full rollback incl. a new run 022
  would queue, delay refused, timeout race, no stored signal without a waiting step) —
  `crates/application/src/runs/with_start.rs`, `crates/api-http/src/routes/execution.rs`,
  `tests/integration/schedules-bulk-operations/with_start.rs` — SC-018-008 — FR-018-024, FR-018-050.
  *Done 2026-11-05 (not ticked: the 022 case is blocked, G1973):* `POST /runs/signal-with-start` (D1969). Service tests
  `with_start::` (8) and REST tests `with_start_rest::` (5) pass against a real PostgreSQL: creation delivery and the
  journal's `wait.satisfied`, a holder signalled with no second run, `NOT_WAITING` leaving the journal untouched, an entry
  that waits for nothing rolling back the run and its binding (the identifier is then free), the reuse policy on a closed
  holder, replay and `IDEMPOTENCY_KEY_REUSED` (also across a plain start and a with-start under one key), `run:start` and
  `run:signal` both required, the delay refused (422), typed validation. Contract: path and schemas in the platform OpenAPI,
  reference blessed, Studio types regenerated, the cross-tenant battery sends it, and a rate-admission test shows one token
  per request. 11 mutants, one survived (the signal outside the start fingerprint) and a test now kills it. Not done: a
  new run that 022's permit would queue (022 is not approved) and the timeout race at the creation instant (G1973). The
  task names `crates/application/src/runs/with_start.rs`, which was not created: the code is in `runs.rs` next to `signal`,
  whose core (`deliver_signal`) it shares.
- [x] T018-041 [US4] Update-with-Start (`USE_EXISTING`/`FAIL`, ADR-0031 receipts, `REJECTED` keeps the
  run, nothing on pre-admission failure, `no-store`) — same files — US04 cases + replay — FR-018-025.
  *Done 2026-11-05 (D1970; the code is in `runs.rs`, `declared_operations.rs` of the application and of
  infra-postgres, and `execution.rs` of the routes, not in the `runs/with_start.rs` the task names):*
  `POST /runs/update-with-start`. Service tests `update_with_start::` (11) and REST tests
  `update_with_start_rest::` (8) pass against a real PostgreSQL, both modes: the run and the Update in one
  transaction (journal `run.started` then `state.updated`, audit, outbox, receipt, `verify_run`), an open holder
  updated with no second run, `FAIL` as `BUSINESS_ID_IN_USE`, a validator that says no and a stale revision as a
  `REJECTED` receipt that keeps the run, an undeclared operation and arguments the declaration refuses creating
  nothing (the identifier stays free), replay with the same key and `IDEMPOTENCY_KEY_REUSED` for another
  argument, update key, expected revision or a plain start under the key (and the other way round), a held run
  admitting the Update (closes G1972), `run:start` and `run:update`, typed validation, `no-store`. Crash matrix
  `fault_schedules_bulk_operations update_with_start`: the backend killed after each of the 43 writes of an
  applied request and the 36 of a rejected one leaves every table as it was, and sending it again makes one
  run and one receipt. Contract: path and schemas in the platform OpenAPI, reference blessed, Studio types
  regenerated, cross-tenant battery, one token per request. 13 mutants: 12 killed (among them the Update in a
  second transaction, killed by the crash matrix, and a commit after a refusal) and one equivalent (the handler's
  own `no-store`, which the API sets on every response; removed). Open: a new run that 022's permit would queue
  (blocked), and the two races of G1974.
- [x] T018-042 [US4] With-start idempotency and authorization (fingerprint, replay after current auth,
  cross-tenant `NOT_FOUND`) — `tests/integration/schedules-bulk-operations/with_start.rs` — FR-018-026, FR-018-041.
  *Done 2026-11-05:* the fingerprint of both requests covers every field (the signal; the Update's operation,
  key, arguments and expected revision) and neither is a retry of a plain start under the same key
  (`with_start::a_key_used_by_a_plain_start...`, `update_with_start::the_same_key_replays...` and
  `a_key_used_by_a_plain_start...`, which kill the mutants that take the Update or the signal out of
  the fingerprint); a retry
  after the caller lost the right is refused `403` and writes nothing
  (`with_start_rest::` and `update_with_start_rest::a_retry_after_the_caller_lost_the_right...`); another
  tenant's workflow is `404` for both routes in the cross-tenant battery (`body_of`).
- [x] T018-043 [US4] Write `via` on `wait.satisfied`; retain Update arguments on `state.updated` only for
  Updates declared `reapplyOnReset` — `crates/application/src/{runs.rs,chain_progress.rs,declared_operations.rs}`,
  `crates/scheduler/src/events.rs`, `crates/infra-postgres/src/event_codec.rs` — old/new codec round-trip,
  no payload written for undeclared Updates, content charge — FR-018-039.
  *Done 2026-11-05 (D1975, G1978); the task text is amended in one place: the arguments are referenced from the
  receipt of the Update, not from the `payload_digest` of the `state.updated` row, which already holds the new state.*
  **`via`:** `RunEvent::WaitSatisfied { node, via: Option<WaitVia> }` and `satisfy_wait(.., via)`; `signal` for
  `signalRun` and a chain signal, `event` for the dispatch of a published event (the scheduler and the scoped event
  commands), `signal_with_start` for Signal-with-Start whether it created the run or found an open holder. Tests:
  `wait_via::` (3, real PostgreSQL: signal, published event, Signal-with-Start in both branches), the chain signal test
  of `integration_workflow_primitives` reads the row, the scoped event commands of `sagat-domain`
  (`scoped_event_wait_lifecycle`), and the codec round trip (`event_codec::tests`: each name, an older row without
  `via` decodes and is written back as it was, an unknown name decodes as none). **Arguments:** migration
  `20270105000000_retained_update_arguments` (`operation_update_receipts.arguments_digest`, a foreign key to `payloads`,
  `APPLIED` only; `down` refuses while any is set; classified; the three pinned upgrade oracles move), the evaluator
  returns the canonical arguments only when the declaration opts in (`DeclaredUpdateDecision::Applied { .., arguments }`),
  the store writes the `payloads` row, the receipt column and `detail.retained` in the transaction of the Update.
  Tests `retained_arguments::` (3, real PostgreSQL): an Update that opts in keeps them (receipt, content equal to the
  canonical arguments, journal `retained: true`) and one that does not writes no payload (`retained: false`); the
  content charge differs by exactly the size of the arguments between an opting-in Update and a twin that does not; the
  same arguments applied to two runs are one row, and a rejected Update keeps nothing and has no `state.updated`.
  Seventeen mutants killed: nine for `via` (the plain signal, the with-start that creates the run and the one served by
  a holder, the pass-through to the wait, the chain signal, the dispatch of an event, the scoped event command, and the
  codec writing and reading it) and eight for the arguments (no Update and every Update keeping them, no content write,
  no receipt reference, the journal row saying always `true` and saying nothing, the marker never written and never
  read).
  Gates, both modes where marked: `sagat-domain`, `-application`, `-infra-ir`, `-scheduler`, `-api-http`, `-cli`,
  `-infra-postgres` unit suites; the 20 contract targets that read the journal, the API or the IR; integration
  `schedules_bulk_operations` 343 (both roles), `platform_foundation` 101 (both; migrations and privilege audit),
  `durable_execution` 179 (both), `workflow_primitives` 755 (both), `security_multitenancy` 126 (both),
  `payload_classification` 41 (both), `events_webhooks_messaging` 111, `observability_operations` 20,
  `sagas_compensations` 82, `gitops_versioning` 35; fault `schedules_bulk_operations` 37 (both) and
  `workflow_primitives` 92; clippy over the workspace. The capability that lets a workflow opt in is still off in every
  deployment (G1977), so nothing is retained yet outside these tests; the reapply that reads the arguments is the
  reset (T018-053, T018-056).
- [x] T018-044 [US4] Error reasons and remediation rows — `crates/api-http/src/error.rs`,
  `docs/quickstarts/errors.md` — `errors-018` contract test — FR-018-043.
  *Done 2026-11-05 (D1971, G1975), for the reasons the built operations raise:* 29 reasons have a row of their own
  in `ErrorCode::remediation`, each with the sentence of the contract's table, and are named in the OpenAPI;
  `contract_schedules_bulk_operations errors::` (3 tests) fails when a sentence, a row, the OpenAPI mention or the
  classification of a table row drifts (6 mutants killed), and `crates/api-http/tests/quota_reasons.rs` plus the
  tightened quota assertions of `limits.rs`, `backfill.rs`, `batch.rs` and `rest.rs` tie the three ceilings'
  `details.reason` to the labels the stores really pass. `docs/quickstarts/errors.md` is for definition
  problems and is not touched (D1971). The reasons of reset, 020, 022, 029 and 031 are added with the tasks that raise them.

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
  *Part done 2026-11-04 (task stays open for `TEST_RUN`, which needs feature 022, not approved; `UPDATE` was added on
  2026-11-05, see the end of this note).* `ItemActions` is implemented for the run service in
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
  *`UPDATE` added 2026-11-05 (D1972, G1959 closed):* `batch_actions::an_update_is_applied_to_the_runs_that_declare_it_and_skipped_for_the_others`
  (two open runs `SUCCEEDED` with the declared state at revision 1; an ended run `SKIPPED ALREADY_TERMINAL` and untouched;
  a run whose version declares no such Update `SKIPPED OPERATION_NOT_DECLARED`; a rule that says no `FAILED
  VALIDATION_FAILED` with the state unchanged; another job applies again, once), the creation tests (an `UPDATE` without
  its `update`, or with a name no operation can have, creates nothing), a unit test of the `Error.code` of a refused
  Update, and 6 mutants killed. The task stays open for `TEST_RUN` only (feature 022).
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
  *Part done 2026-11-05 (D1976, G1979), the task stays open:* the registry (66 event types, `rule_of` exhaustive, `RULES`
  by name, a test that reads the names out of `RunEvent::name()`, a test typed from the contract) and the eligibility and
  the plan over a corpus of: a sequential run, a parallel fork, a timer and a wait, a saga that aborts, a failure that is
  resolved later, an unknown outcome inside and after its window, a paused run, a child, a collection, and the plan with
  its effect markers. Not done: an approval run and a loop run in the corpus (their completions are not points under the
  literal D1022 list, G1979, which asks the owner), and an end-to-end journal for the clause "compensated later", which
  the reducer does not let exist (the predicate is unit-tested). The error `RESET_RULE_MISSING` is T018-054's.
- [ ] T018-053 [US6] Reset store (prefix read/digest outside, verify and re-emit inside with `restored`,
  `reset_of`, `run.reset`, `rerun_of` + `reset_sequence`, guard after business-id row, `purged_at`/`test_tree`
  read, 022 permit acquire-or-refuse, 020 attribute fold) — `crates/infra-postgres/src/{resets.rs,execution.rs}`
  — content test — FR-018-037, FR-018-050, FR-018-051.
  *Part done 2026-11-05 (D1977, G1980), the task stays open:* migration `20270106000000` (`runs.reset_sequence`,
  immutable, `down` refuses while any run has it; the three pinned upgrade oracles moved), `restore_prefix` in
  `crates/infra-postgres/src/execution/resets.rs` and the restored-row path of `persist`: the rows `1..=N` of the source are
  copied verbatim (actor, time, trace, payload), the first one marked `restored` with `reset_of`, with none of the side
  effects of the first time; the outputs of the prefix and the declared state are rebuilt; `run.reset` is recorded and the
  run is settled. Proved by `reset::{a_run_is_reset_to_a_point_and_goes_on_from_there_with_the_history_it_had,
  a_reset_to_the_start_is_a_run_of_its_own_and_the_refusals_are_typed,
  the_declared_state_and_the_output_of_a_signal_in_the_prefix_are_restored,
  the_same_key_is_the_same_reset_and_another_point_under_it_is_another_request}` (real PostgreSQL, owner and `sagat_app`)
  and by a battery of 14 mutants of the store and the service, all killed. Not done: the 022 permit and the 020 attribute
  fold (features not approved), scoped prefixes and the configurable ceiling (G1980). The `run_resets` register is
  built (D1978, below).
- [ ] T018-054 [US6] Reset service and routes (points, preview, reset, list/get) —
  `crates/application/src/runs/reset.rs`, `crates/api-http/src/routes/resets.rs` — contract tests —
  FR-018-035..037.
  *Part done 2026-11-05 (D1977), the task stays open:* the service `reset_points`, `reset_preview` and `reset` in
  `crates/application/src/runs/reset.rs` (permissions `run:operate` and `run:start`, justification of 8 to 2000
  characters, the typed reasons `RESET_NOT_SUPPORTED_FOR_SOURCE`, `RESET_SOURCE_NOT_TERMINAL`, `RESET_POINT_NOT_ELIGIBLE`,
  `RESET_PREFIX_TOO_LONG`, `RESET_RULE_MISSING`, `RESET_REEXECUTION_NOT_ACKNOWLEDGED` as `AppError::ResetRefused`, mapped in
  `crates/api-http/src/error.rs` to `422 VALIDATION_FAILED`, and `409 CONFLICT` for `RESET_SOURCE_NOT_TERMINAL`, as
  `contracts/errors.md` says; the routes of 2026-11-06 below drive it) and the idempotency of the request. The four tests above prove the points, the
  preview, the permission refusals, `RESET_SOURCE_NOT_TERMINAL`, `RESET_POINT_NOT_ELIGIBLE`,
  `RESET_REEXECUTION_NOT_ACKNOWLEDGED` and the key; three more tests prove `RESET_PREFIX_TOO_LONG` (past the ceiling and
  at it), `RESET_NOT_SUPPORTED_FOR_SOURCE` for a child run, and `RESET_RULE_MISSING` with the type named (a row of a
  newer kind after the run's last event); a battery of 6 mutants of those three reasons was killed. A member of a
  continuation chain (a row of either chain event in its journal) is also proved not to be a source, 2026-11-06.
  The branch of `RESET_NOT_SUPPORTED_FOR_SOURCE` for a run of a database execution **has no test** (G1980 (5)).
  *Also done 2026-11-05 (D1978):* the register `run_resets` (migration `20270107000000`: the table, a guard trigger, the
  quota triggers for the justification, grants, the storage classification, the three upgrade oracles moved), written
  in the transaction that creates the run (`RunTx::register_reset`), `reset()` answering `ResetMade { reset, run }`,
  and `resets` / `reset_record` for the list and the get. Proved by `reset::{a_reset_is_recorded_with_what_was_asked_and_who_asked_and_a_refused_one_is_not,
  the_register_lists_the_resets_of_a_run_newest_first_and_a_retry_adds_none,
  the_register_is_private_to_the_tenant_and_to_the_run_that_made_the_reset,
  what_a_reset_is_never_changes_and_a_completed_one_stays_as_it_completed}` and a battery of 17 mutants (13 of the
  store and 4 of the guard trigger), all killed, then 3 more of the business policy after the runtime-privilege audit
  (which reads a bare `"DROP"` in Rust as a statement) made the statement write the name: all killed.
  *Also done 2026-11-06 (D1979):* the five routes (`listResetPoints`, `previewReset`, `resetRun`, `listRunResets`,
  `getRunReset`) in `crates/api-http/src/routes/resets.rs` and `dto_resets.rs`, the OpenAPI operations
  (`x-implemented: '018'`), the generated reference and the Studio types, the cross-tenant battery (a victim reset and
  the two bodies), the six `RESET_*` reasons raised with the sentences of `contracts/errors.md`. Proved by
  `reset_rest::*` (5 tests over a real API, a real worker and real tokens: the points by kind, node, time and cursor;
  the preview; the reset, its retry and its refusals; the business identifier; the register and who may read it),
  `contract_schedules_bulk_operations resets::every_reset_operation_conforms_to_the_contract_on_its_normal_and_error_paths`
  (every answer has a documented status and schema), `errors::*`, the battery of `contract_rest_security_multitenancy`,
  and the unit tests of the DTO and of the kinds of a point; a battery of 19 mutants of the routes, the DTO, the plan,
  the admission and the chain refusal: 18 killed (one of them after the test of a page that ends at the limit was
  added), 1 equivalent (the admission of the points asks `run:operate` and
  the service asks it again). It also found and fixed a defect of the plan (D1979 (2)). Not done: the Studio dialog (T018-059), `CANCEL_SOURCE` and the reapply (T018-057, T018-056: both are
  `422` in words); the branches of `RESET_NOT_SUPPORTED_FOR_SOURCE` for a run of a database execution (a trigger ties
  `origin` to an internal workflow, so a test cannot make one with SQL) has no test, G1980.
- [ ] T018-055 [US6] Replay = projection at every eligible point (corpus incl. a 022 run-permit holder and
  020 search-attribute upserts); outputs and attribute rows equal the source at N; holder count = open
  non-restored acquisitions; ceiling; crash during reset — `tests/integration/schedules-bulk-operations/reset.rs`
  — SC-018-011 — FR-018-037, FR-018-050.
  *Part done 2026-11-06, the task stays open:* `reset::at_every_point_the_run_that_is_made_replays_to_its_projection_keeps_the_history_and_goes_on_to_the_end`
  takes all four points of a sequential run (the start and the end of each of three steps): the projection of the new
  run equals a replay of its journal before it goes on and after it ends, rows `1..N` are the source's as it stored
  them (type, who, payload, when), the steps of the prefix hold the output the source gave them and no other step
  does, the run finishes what is left (at the last point it ends with what it was given), and the source is
  untouched. `tests/fault/schedules-bulk-operations/reset.rs` kills the connection after each of the 62 writing
  statements of a reset (one transaction, 13 tables) in both roles: nothing changes in any table, and the request sent
  again makes one run and one entry with the source's history, and sent a third time answers the first. A mutant
  that makes a retry another request is killed by it. Not done: the other shapes of the corpus at every point
  (parallel, saga, collection, loop, child, a wait), approvals, the 022 run-permit holder and the 020 attribute rows
  (not approved), and a prefix at the ceiling (the ceiling is tested at its boundary by sequence, not with a journal
  of 5,000 events).
- [ ] T018-056 [US6] Reapply order, first-undeliverable stop, `LEGACY_EVENT`, ceiling, key replay, lost
  hint — `tests/integration/schedules-bulk-operations/reapply.rs` — report equals expectation — FR-018-039.
  *Built 2026-11-06 (D1982..D1995, G1982..G1988); left unticked for the maintainer to tick after reading it. Reworked the same
  day after three independent reviews (correctness, security, architecture).* A reset with `reapply` `SIGNALS`, `UPDATES` or
  `ALL` delivers again, in the transaction that creates the run and in the order of the source journal, the signals recorded
  `via: signal` and the Updates that kept their arguments (`reapplyOnReset`), under the key `reset:{reset id}:{sequence}` (the
  prefix is reserved, with `batch:` and `schedule:`, in one list that every validator of a caller's key uses, D1995); each is
  reported `REAPPLIED`, `REJECTED` or `NOT_REAPPLIED` with its reason, the report is stored with the entry and is the same on a
  replay; the preview lists the same candidates `WILL_TRY` or `NOT_REAPPLIED`; more than 256 is `RESET_REAPPLY_TOO_MANY`; the
  setting `SAGAT_RESET_REAPPLY_ENABLED` (the control plane only; the engine has none, D1990) turns it on, closing G1977 (off:
  `RESET_REAPPLY_NOT_AVAILABLE` for a request that was never made, asked where a run would be created, so a retry of one that
  succeeded is still answered, D1991); the Studio dialog and the CLI (`--reapply`) show it. The first test of it found a defect
  of Update-with-Start's store (D1984): an Update left the in-memory state of a transaction behind its journal.
  What the reviews changed, each with the test that was seen failing first (or, for a pure guard, the mutant that survived
  until the test existed): (A, D1989) two equal signals a reset delivered in one transaction are two signals in a reset of that
  reset: `a_reset_of_a_reset_keeps_two_equal_signals_that_the_first_reset_delivered_in_one_transaction` and three unit tests of
  `candidates`; (B, D1990) the engine setting is gone (`config` tests of both binaries, the harness of the SDK conformance
  passes nothing); (C, D1991) `a_retry_is_answered_from_the_register_where_the_reapply_is_off_now_and_a_new_request_is_refused`
  and `a_deployment_that_did_not_turn_the_reapply_on_says_so_before_it_looks_at_the_source`; (D, D1992) a report that a newer
  version wrote is read as written, never an error (`reset_rest::a_report_that_a_newer_version_wrote_...`, unit
  `what_another_version_wrote_is_read_as_written_and_never_fails`, the OpenAPI names are open strings, Studio shows an
  unknown name as written); (E, D1993) one bounded chain walk and batched reads
  (`a_chain_of_resets_of_resets_is_walked_once_and_the_walk_stops_at_its_bound`: 6 connections for a preview of 200 candidates
  and 16 for the reset, bounded at 12 and 32, 64 hops found and 65 `NOT_RETAINED`; three unit tests of the owner of a row);
  (F, D1994) `the_service_the_control_plane_builds_refuses_the_reapply_unless_the_operator_set_it` (unset refuses, set serves;
  the test support turns it on and says so); (G, D1994) an Update to a run that is paused or being cancelled is a clean
  refusal and one addressed to another run is refused before it writes (`an_update_to_a_run_that_is_paused_or_being_cancelled_...`,
  `an_update_addressed_to_another_run_than_the_transaction_s_is_refused_before_it_writes`); (H, D1995, G1986) the counter
  `workflow_reset_reapply_total{kind,outcome,reason}` is counted once the reset is committed
  (`what_a_reset_delivers_again_is_counted_once_it_is_committed_with_closed_labels`, and the totals of the crash matrix);
  (I, D1995, G1985) the audit trail and the trace id join; (J, G1982) the domain test
  `a_role_that_may_reset_a_run_may_deliver_again_what_the_reset_reapplies`; (K, D1993 (4))
  `nothing_can_delete_the_content_a_reset_reads_before_the_transaction_that_delivers_it`; (L, D1995) the hint no longer names the
  variable and `Candidate`, `Material`, `Delivery` and `Retained` print no content; (M, D1995) the reserved prefixes are one
  list with a test that fails for a validator that does not use it and a behaviour test in each of `chains.rs` and `versioning.rs`.
  Evidence: `reapply::*` 24 tests in the integration target (real PostgreSQL) and 9 in `reset_rest::`;
  `fault_schedules_bulk_operations reset::` cuts the connection after each of the 91 writing statements of a reset that delivers
  again and counts the metric; unit tests: `sagat-application` 110 (27 of the reapply), `sagat-api-http` 37, `sagat-infra-postgres`
  29, the engine 13, `sagat-cli` all; the config of both binaries; Studio 1,386 tests. Gates on the final tree:
  `integration_schedules_bulk_operations` 390 passed in both roles, `integration_platform_foundation` 101 in both
  (runtime-privilege audit included), `fault_schedules_bulk_operations` 39 in both, `contract_schedules_bulk_operations` 7,
  `contract_developer_experience` 8 (`docs/reference/cli.md` regenerated, rerun without bless), `contract_rest_security_multitenancy`
  21, `integration_workflow_primitives operations` 27, `sagat-domain` all, Studio lint, typecheck, build and `check:api`, the `reset`
  scenario of the SDK conformance suite equal in all six languages; fmt, clippy over the workspace, the architecture,
  agent-config and design-docs checks. A battery of 144 mutants (the candidates and their dedupe, the stop, the answers, the
  keys, the planner, the service, the gate, the store and its chain, the API, the CLI, the Studio, the roles, the metric, the
  wiring): 141 killed, 3 survived, all equivalent: the permission checks of `run:signal` and `run:update`, which no role of the
  matrix lacks while holding `run:operate` (G1982, and the domain test now pins the matrix), and the preview asking the store for
  the bodies it then ignores (`read: false` is a cost, not an output; the store returns no content for it, tested, and the planner
  never carries it into a preview, tested). Not done: `CANCEL_SOURCE` (T018-057); the 022 permit and the 020 attribute fold; a wait
  below the root (G1983); the span and the latency histogram of an Update delivered by a reset (G1986); the unbounded 256 x 256 KiB
  of a reset's reads (the run output ceiling covers it).
- [ ] T018-057 [US6] `CANCEL_SOURCE` pending reset (fenced completion, eligibility re-check, races,
  crash, failed source compensation, pending quota) — `crates/scheduler/src/passes.rs`,
  `tests/fault/schedules-bulk-operations/reset.rs` — SC-018-012 — FR-018-038, FR-018-040.
  *Amendment 2026-11-06 (as D300 did): the text above names `crates/scheduler/src/passes.rs` and `tests/fault/schedules-bulk-operations/reset.rs`, which are not where the work is. The pass, its settings and its backoff are `crates/scheduler/src/resets.rs`; the completer is `crates/application/src/reset_completion.rs` (the use case is `crates/application/src/runs/pending_reset.rs`); the crash, race and fence tests are `tests/fault/schedules-bulk-operations/cancel_source.rs`.*
  *Built 2026-11-06 (D1996..D2005, G1989..G1992), amended the same day after three independent reviews (D2006..D2012, G1993..G1995); left unticked for the maintainer to tick after reading it.* `sourcePolicy:
  CANCEL_SOURCE` resets a run that has not ended. **The request** (one transaction on the row of the source) judges what a reset of
  an ended run judges, on the journal as it is now, then the namespace's quota (`pending_resets`, 16 by default, hard 64:
  `429 PENDING_RESET_LIMIT`), writes the entry `PENDING_SOURCE` (actor, justification, acknowledgement, digest of the prefix), the
  receipt of the key, two audit records and the ordinary cancellation of the source (never a terminate; a source already stopping is
  not cancelled twice), and answers `202` with no run. **The pass of the engine** (`ResetPass`, fenced by the lease of the shard like the
  batch pass) takes the entries whose source has ended and, as the member who asked with the role they hold then, judges everything
  again on the final journal (digest of the prefix, point, acknowledgement, reapply, business identifier, quotas), creates the run
  exactly as `reset()` does and completes the entry in one transaction that holds the fence and the entry `FOR UPDATE` and locks the
  membership of the requester; or it fails the entry with a closed reason (`BOUNDARY_NO_LONGER_ELIGIBLE`, `ADMISSION_REFUSED`,
  `BUSINESS_ID_IN_USE`, and three added by D1998: `AUTHORITY_REVOKED`, `REEXECUTION_NOT_ACKNOWLEDGED`, `REAPPLY_TOO_MANY`). Setting
  `SAGAT_RESET_CANCEL_SOURCE_ENABLED` (control plane and engine, off by default, strict; `422 RESET_CANCEL_SOURCE_NOT_AVAILABLE` for an
  open source where it is off; a retry of an accepted request is answered from the register whatever it says now). No expiry and no
  withdrawal (D2000, G1991). Migration `20270108000000` (the column, the reasons, the quota resource; the three upgrade oracles moved).
  Telemetry: `workflow_reset_completions_total{outcome,reason}`, `workflow_resets_pending`, `scheduler_lag_seconds{pass="resets"}` (the
  existing alert `SagatSchedulerLate` covers it). OpenAPI, the generated Studio types, `docs/reference/cli.md`, the runbook (section
  4.2), the quickstart and the scenario `reset` are updated; the Studio dialog ("Cancel and reset to a point…", always asking for the
  acknowledgement, a list that shows waiting and failed entries and refetches on focus, never on a timer) and the CLI
  (`--cancel-source`, `workflow resets`) show it.
  Evidence on the final tree (each command run, all exit 0): `integration_schedules_bulk_operations` 429 passed in both roles (the
  owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`), 39 of them `cancel_source::*` and `cancel_source_rest::*` (the request and its retry,
  the completion, the race with the end of the source, the quota, the setting, the requester revoked or demoted, the business
  identifier, the point and the digest, the pass and its lease, the metric with closed labels, the lag, the reapply by an engine that
  has no setting for it, and a real engine started with the flag that makes the run an API accepted); `fault_schedules_bulk_operations`
  48 in both roles, 9 of them `cancel_source::*`: the connection killed after each of the 19 writing statements of a request and each
  of the 47 of a completion (nothing changes in any table; the same request or step sent again makes one cancellation, one entry,
  one run, and a third answers the first), the source ending before and after the request commits (both orders, held row lock and the
  queue of the lock, no sleeps), two equal requests meeting at the row, two engines completing and two failing the same entry (what
  they wait on is asserted), and the revocation that returns while the run is admitted for a viewer and for a developer;
  `integration_platform_foundation` 101 in both roles (the audit of the privileges of the runtime role included: it refused a bare
  `"DROP"` twice and the code now writes the choice of policy as a boolean); `contract_schedules_bulk_operations` 7;
  `contract_developer_experience` 8 (`docs/reference/cli.md` regenerated, read, rerun without bless); `contract_rest_security_multitenancy`
  21; `fault_platform_foundation` 38; `e2e_platform_foundation` 42; unit tests of `sagat-cli`, `sagat-control-plane`,
  `sagat-execution-engine`, `sagat-scheduler`, `sagat-application` (119), `sagat-infra-postgres`, `sagat-api-http`, `sagat-domain`; the
  registry tests of `sdk_conformance` (3, one ignored); the scenario `reset` equal in all six languages without bless on the final tree (blessed from
  TypeScript after reading the diff; a first run of the other five found the attempt left in flight at the end of the scenario, D2004); Studio lint, typecheck, 98 files and 1,433 tests, `npm run api`
  and `check:api`; `cargo fmt --check`, clippy over the workspace with `-D warnings`, the architecture, agent-config and design-docs
  checks.
  How the server tests were made: the code was written before most of its tests, so those were not seen red first; what shows that
  each one bites is a battery of 73 mutants of the new logic (the request, the receipt, the completer and its mapping of errors to
  reasons, the store, the fence and the lock, the quota, the pass, the settings, the CLI): 69 killed. 17 survived the first run,
  each was a missing test or a mutant run against the wrong suite, and was killed after a test was added (stale entry read before
  another engine completed or failed it, the shard filter, the count by namespace, a shard stolen between the scan and the step with
  the metric, the lock the engines wait on, the permissions of the reapply, a role that starts runs and does not operate them); the
  four that remain are equivalent: the point in the fingerprint of the receipt (the digest of the prefix already holds it), the
  permission loop at the head of the completion (`reset_candidate` and `start_with` ask the same two of the three themselves, the
  other two are held by every role that holds `run:operate`, G1982, and `recheck_requester` asks all of them again under the lock),
  the digest in the `WHERE` of the entry's `UPDATE` (the service compared it before, in the same transaction that holds the row), and
  `finished_at IS NOT NULL` in the lag (the minimum of NULLs is NULL). The tests that did fail first, for the right reason, against
  the new behaviour: `reset_rest::a_reset_makes_a_run_and_an_entry_a_retry_is_the_same_entry_and_the_register_lists_it` (it expected
  the old `422`), the lists of `contract_schedules_bulk_operations errors`, `quota::tests::durable_resources_have_explicit_scope_...`
  (36 resources, now 37), `contract_rest_security_multitenancy` (the enum of `/quotas`), the audit of the privileges. The Studio
  part was written test first by its own agent (21 tests red before the code, then 73 mutants, 71 killed, 2 equivalent).
  *Review of 2026-11-06, done the same day (correctness, security and architecture found no blocker; the items below are what they asked for, A to N).*
  A: the scan takes at most `SAGAT_RESET_CANCEL_SOURCE_PER_TENANT` entries of a tenant per round, the pass leaves an entry that failed out until its time (2 s doubling to 5 minutes, 1,024 remembered), only the two known conflicts are races, every line names `tenant_id` and `reset_id` (D2006, G1990). B: a failure of the store that is not an answer of the admission leaves the entry pending, and a source that cannot be read is an anomaly and not `AUTHORITY_REVOKED` (D2007). C: the race for the last place is a test that holds the advisory key and sees one `202` and one `429`, the second source not cancelled. D: the acknowledgement is asked before the source is cancelled, also for a step in flight; the preview, the contract sentence, the CLI, the runbook and D1999 say it, the Studio already asked (D2008, G1994). E: ADR-0081 (proposed) records that the per-process flags stand in for the registry, the engine first and no default-on; `optional_passes` is a pure function with a test; no shipped manifest turns the flag on (D2009). F: the order of locks of the request is a recorded deviation in ADR-0081 and one line of ADR-0050, with the argument that there is no cycle (D2001, G1993; T018-012 stays open). G: two tests of the `down` migration (refusals, clean schema and the `up` again) and 13 mutants of the `down`, all killed. H: the justification is in the fingerprint of both paths (D2010). I: the register keeps the policy and the acknowledgement that were asked for an ended source (D2011). J: the index of the count. K: the `Debug` of the request types prints none of the caller's content. L: the audit names the reset it completes, and the creation is one helper for both callers (D2012). M: the documents (D2002, G1989..G1992, the runbook, the rebase hazard in the migration, the two suspicions checked in G1995). N: the sequence past the journal and past the ceiling have their own reasons, the prefix is compared row by row, the failure of an entry has its crash matrix (3 writes) and runs under a valid fence; the Java javadoc of `reapply` says the reapply policies (`SagatClient.java` and the same line of `ResetRequest.java`).
  Evidence of the review on the final tree (each command run, exit 0): `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings` (it found, and the work fixed, an eight-argument function, a collapsible `if`, a complex type and a fixture of `dto_resets` missing the new field); `check-architecture.py`, `check-agent-config.py`, `check-design-docs.py`; `integration_schedules_bulk_operations` 446 passed in both roles (the owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`); `integration_platform_foundation` 101 in both roles; `fault_schedules_bulk_operations` 49 in both roles; `contract_schedules_bulk_operations` 7; `contract_developer_experience` 8 (`docs/reference/cli.md` regenerated, the diff read, rerun without bless); `contract_rest_security_multitenancy` 21; `fault_platform_foundation` 38; `e2e_platform_foundation` 42; the unit tests of the eight crates 1,636; `sdk_conformance` registry 3 (1 ignored) and the scenario `reset` equal in the six languages without bless; Studio lint, typecheck, 98 files and 1,433 tests, `npm run api` and `check:api`; Java `./mvnw -q verify` 258 tests.
  Tests that were seen failing for the right reason before the change that makes them pass: the two starvation tests of A (a failing head of the scan, in another tenant and in its own), the contract sentence of `RESET_REEXECUTION_NOT_ACKNOWLEDGED` (the table changed first), the CLI preview of a step in flight, and `docs/reference/cli.md`; for the others a red run was not recorded, and the proof that each bites is the mutants (the rule of the first delivery): 38 mutants of the changed logic and 13 of the `down`; one survived (the pass handed the batch as the share of a tenant), a test was added (`a_round_takes_no_more_of_a_tenant_than_its_share_though_the_batch_would_hold_more`) and the mutant was killed. The mutants cover the share and the skip list of the scan, the backoff (doubling, ceiling, bound, forgetting), the known races, the classification of the store's failures, the anomaly, the step in flight (both statuses, the request and the shared creation), the lock of the pending resets, the fingerprint (with and without trimming), the register's policy and acknowledgement, the `Debug` of the request types, the audit tokens, the restored mark, the audit of a failure, `optional_passes`, the manifest check, the index and the CLI preview. The first battery of 73 mutants was run again on the final tree: 64 apply as written and 59 were killed; 9 no longer apply because the code moved, and each of them was restated on the current text and killed (they are in the 38); 5 survived. One of the five was a gap that D opened, and was killed after a test was added: the acknowledgement of the request is stored in the entry, but the tests that plant entries by hand do not go through the request, so `nothing_in_flight_and_nothing_completed_after_the_point_asks_for_no_acknowledgement` and `a_step_in_flight_when_the_request_is_made_needs_the_acknowledgement_and_nothing_is_cancelled_without_it` now read the entry back. The other four are the equivalents of the first delivery, unchanged: the point in the fingerprint of the receipt (the digest of the prefix already holds it), the permission loop at the head of the completion (`reset_candidate` and `start_with` ask the same, and `recheck_requester` asks them all again under the lock), the digest in the `WHERE` of the entry's `UPDATE` (the service compared it before, in the transaction that holds the row), and `finished_at IS NOT NULL` in the lag (the minimum of NULLs is NULL).
  Not done: the 022 permit and the checks of the features that are not approved (`CONCURRENCY_PERMIT_UNAVAILABLE`, `RUN_PURGED` and the
  `REMATERIALIZE` reasons have no producer); a withdrawal or an expiry of an entry (G1991, for the owner); a durable bound on the retries of an
  entry that fails for a reason that is not an outcome (G1990: the backoff is in memory and an entry is never given up); the observation that an engine without the flag makes nothing (G1992 (2): its
  decision is a tested pure function, the observation itself cannot be); a point that becomes ineligible end to end (G1979 (2), G1992 (1)); no browser journey covers the Studio dialog.
  *Note 2026-11-09, not ticked, review pending: the expiry of an entry that waits (owner decision OD-2026-11-08-1, D2059 to D2068, G2015 to G2018; G2014 was withdrawn, see the note of the review below).*
  **Done (built, tests below):** an entry still `PENDING_SOURCE` a whole window after its request (24 h; the engine's
  `SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS`, 3,600 to 604,800 inclusive, strict, engine only) with its source open ends `FAILED`
  `SOURCE_DID_NOT_END`, no run, its place freed (the count is derived), the source untouched, audited by `system:engine`, counted
  `workflow_reset_completions_total{outcome="expired"}`; one transaction that writes what a failure writes (`end_locked_entry`), under the
  fence and the lock of the entry, the source read `FOR SHARE ... SKIP LOCKED` and never waited for, an ended source always the
  completion's; bounded per round and per tenant, with the completion's backoff; migration `20270115000000_reset_expiry` (the CHECK of the
  reasons; the maintainer renumbers); REST/OpenAPI text, `errors.md`, CLI words, Studio words and dialog text; runbook 4.2/6/7, ADR-0081
  amendment, `owner-decisions.md` (status "Built 2026-11-09, review pending", not "Built, reviewed"), plan row 18 and the limits table.
  **Not done:** a withdrawal call (not asked: G2018); an alert and its `promtool` test (`promtool` is not installed: G2016); a signal before an
  entry expires (G2016); the plan of the scan
  measured at the size of a deployment (G2015); the SDK scenario `reset` does not exercise the expiry and was not changed (it cannot wait a
  day); the three upgrade oracles now expect `20270115000000` and were **not run**; **no mutation battery** (OD-2026-11-06-2); the review by the
  correctness, security and architecture reviewers has not been made, so the flag stays off.
  **Evidence on this tree (each command run here, exit 0):** `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings` (its
  first run refused an unread field of a test helper, removed); `scripts/check-architecture.py`, `check-agent-config.py`, `check-design-docs.py`;
  `integration_schedules_bulk_operations` 610 passed, 1 ignored (as before) as the owner and again with `SAGAT_TEST_AS_RUNTIME_ROLE=1`, 21 of
  them new (`cancel_source_expiry` 18, `reset_expiry_migration` 2, `cancel_source_rest::an_entry_that_expired_...` 1); `fault_schedules_bulk_operations`
  74 passed in both roles, 6 of them new (`cancel_source_expiry`); `integration_platform_foundation` 101 passed in both roles (the audit of the
  privileges of the runtime role, which reads the Rust string literals as SQL, with the new statements in the tree); `contract_schedules_bulk_operations` 7; `contract_developer_experience` 8 (without
  the bless switch, `docs/reference` unchanged: no help of the `sagat` CLI changed); `contract_rest_security_multitenancy` 21; unit and doc tests of
  `sagat-cli`, `-control-plane`, `-execution-engine`, `-scheduler`, `-application`, `-infra-postgres`, `-api-http`, `-domain`: 1,719 passed, 0 failed
  (new: the window and `did_work` of the scheduler, the two configuration tests of the engine, the one of the control plane, the CLI and the Studio
  words); Studio lint, typecheck, 98 files and 1,438 tests, `npm run api` and `check:api`; the registry tests of `sdk_conformance` (3 passed, 1
  ignored) and `SAGAT_SDK_SCENARIO=reset ... -- --ignored` in the six languages, equal and without the bless switch (it needs
  `SAGAT_RUST_SDK_TARGET_DIR`, an owned directory outside the repository; a first run without it failed in `prepare.sh` for Rust and proved nothing).
  **Seen failing for the right reason first:** the integration tests of the expiry against a store that returned nothing (16 of 18), the
  configuration test of the control plane (its `--help` did not say the window is the engine's), the two of the engine with the window read as
  the default, the CLI words (the raw name was printed) and the Studio words. **Not seen failing before the code:** `reset_expiry_migration` (written
  after the migration), the REST test, and the six fault tests (written after the store; the two guards they stand for, the look at the source
  under `FOR SHARE ... SKIP LOCKED` and the check that it has ended, were taken out by hand for one run and `an_entry_is_never_both_expired_and_completed_whichever_comes_first`
  and `an_expiry_and_a_completion_that_meet_at_the_entry_make_one_outcome_and_the_ended_source_wins` (renamed `..._make_one_outcome` on 2026-11-09: it is the race test, not the guard test, D2060) failed, then both were put back).
  *Note 2026-11-09 (second), not ticked: the findings of the three independent reviews of the expiry (correctness, security, architecture: no blocker, no tenant leak, "an ended
  source wins" and "never both" held at every interleaving they traced), fixed in the same tree. D2079 to D2082, G2014 withdrawn, G2024 to G2026.*
  **A. The clock.** G2014 was wrong and is closed: `created_at` is `clock_timestamp()` of the transaction of the source (`register_pending` refuses any other), the control plane's clock stamps
  only the audit records (D2079). Guards, red under a hand mutant first and then put back: `a_control_plane_whose_clock_is_three_hours_off_neither_cuts_the_window_short_nor_stretches_it` (run
  service on a clock +3 h and -3 h; the mutant that binds the audit record's time into `created_at` failed it at the bracket of two readings of the database's clock, `skew 3h: asked at 08:29, the
  database read 05:29`); `an_engine_whose_clock_is_three_hours_off_reads_the_window_on_the_database_clock` (through `loops::reset_round`; the mutant that removes its `sync()` failed it, `expired: 1`
  at 22 hours). The text of D2060 and the doc of `ResetExpiry::requested_by` that said a caller that computed the cutoff wrongly could not end an entry inside its window were false and are corrected.
  **B. The flag off in an engine.** Runbook (sections 4.2 and 6), the log line of an engine with the pass off, `optional_passes`' doc, D2059/D2060/D2080 and G2024 say that nothing expires an entry then;
  `with_the_flag_off_the_engine_runs_no_pass_that_expires_a_reset_either` pins it (it pins what was already so: it was not red first, and no mutant is named for it).
  **C. ADR-0081.** "Enforced by" lists the checks of the expiry; decision 5 no longer says that the completion takes "never a source row" and says what the completion and the expiry take; the alert with
  its `promtool` test is recorded as a precondition for lifting decision 4 (G2026), and no untested alert was written.
  **D. Tests that passed under a mutant.** (1) The store test and part (2) of the fault test now hold the source with an uncommitted `UPDATE runs SET updated_at = updated_at` (`FOR NO KEY UPDATE`); with
  `FOR KEY SHARE ... SKIP LOCKED` in the store the store test, the fault test and the new busy test all failed (`Expired` where `SourceBusy` was expected), then `FOR SHARE` was put back. (2) The fault call
  is bounded with `within`: with `SKIP LOCKED` removed it failed after 60 s (`did not finish within 60s`) instead of hanging, then it was put back. (3) The fault test is renamed
  `an_expiry_and_a_completion_that_meet_at_the_entry_make_one_outcome` (the race test) and says which tests cover the guard. (4) The two assertions decided by other CHECKs are gone from
  `the_list_of_reasons_admits_the_expiry_and_stays_closed`. (5) `reset_expiry_migration` reads the migration before `THIS` from the migrator, with a test that pins `THIS`.
  **E. Pacing, a busy source, mixed windows.** (1) `SourceBusy` is `Completed::Busy`: left out for a second (`Backoff::BUSY`), counted (`ResetRound.busy`), logged at debug with the ids; the integration test
  and the unit test were red against the old mapping (`busy: 0`, the entry forgotten, the newer one never looked at). (2) `ResetRound::pause_after`: a round that expired entries rests for what they cost at a
  hundred a second (at least 200 ms); the unit test and the loop test were red against a pause of zero (`the next round followed at once: 20µs`). (3) The runbook and D2062/D2082 say that each engine
  applies its window to the shards it holds and shards move (set the same value everywhere), and each engine logs its window once when the pass starts (that line is not tested: G2025).
  **F. Accepted, not done:** the re-validation of the CHECK under `ACCESS EXCLUSIVE` (D2063), the cutoff in SQL, the duplicated literal and the `#[doc(hidden)]` export (D2082).
  **Evidence on this tree (each command run here, exit 0):** `cargo fmt --check`; `cargo clippy --workspace --all-targets --locked -- -D warnings`; `scripts/check-architecture.py`, `check-agent-config.py`,
  `check-design-docs.py`; `integration_schedules_bulk_operations` 614 passed, 1 ignored, as the owner and with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (4 more than before: the two clock tests, the busy test and the pin of
  `THIS`); `fault_schedules_bulk_operations` 74 in both roles; `integration_platform_foundation` 101 in both roles; `contract_schedules_bulk_operations` 7; `contract_developer_experience` 8; `contract_rest_security_multitenancy` 21;
  unit and doc tests of the eight crates 1,723 passed, 0 failed (4 more: the busy backoff and the pause of the scheduler, the loop test and the engine's configuration test of the flag); `sdk_conformance` registry 3
  passed, 1 ignored, and the scenario `reset` equal in the six languages without bless; Studio lint, typecheck, 98 files and 1,438 tests, `check:api`. **Not run:** the three upgrade oracles; no mutation battery
  (OD-2026-11-06-2: the hand-made mutants above are one-line edits to see a named test fail, each put back, not a battery); no `promtool`.
- [x] T018-058 [US6] Telemetry projector skips the restored prefix and links the source span —
  `crates/application/src/telemetry.rs`, `crates/scheduler/src/telemetry.rs` — no duplicate attempt
  spans — FR-018-037, FR-018-044.
  *Done 2026-11-06.* `RunFacts` carries `reset: Option<ResetOrigin { source, source_trace, restored_through }>`,
  which the store reads from `runs.reset_sequence`, `rerun_of` and the trace of the source
  (`crates/infra-postgres/src/telemetry.rs`; the projector of `crates/scheduler/src/telemetry.rs` needed no change).
  `derive` takes the entries `1..=restored_through` as context whatever the progress of the projector says, so a
  projector that starts again from the beginning does not tell them either; the run span tells only what happened
  after them and links the span of the source when its trace is known. Proved by
  `telemetry_derive::the_restored_prefix_of_a_reset_run_is_not_told_again_and_the_run_span_links_its_source` (spans,
  measurements, span events, progress 0, 3 and 6, a source without a trace) and by
  `reset_telemetry::the_attempts_of_the_restored_prefix_are_told_once_by_the_source_and_the_new_run_links_it` (the
  real projector and an OTLP collector: the source tells three attempts, the new run one, its span links the
  source's); 5 mutants killed, two of the store and three of `derive`. The progress of the note is not started at the
  restored sequence: the projector reads the prefix once and derives nothing from it.
- [x] T018-059 [P] [US6] Studio reset dialog and provenance links — `frontend/apps/studio/src/features/runs/`
  — component tests — FR-018-047.
  *Done 2026-11-06.* `ResetRun.tsx`: "Reset to a point…" on a run that ended, for someone who may operate and start
  runs; the points in words (the start, after a step, a child, a collection, a loop) with their time; what a reset to
  the one chosen keeps and runs again, each step with what it declares in words; the acknowledgement that steps run
  again, asked only when the plan says so and forgotten when another point is chosen; the choice about the business
  identifier for a run that has one; a reason; the key of the request kept for an identical retry and changed with
  the request; the platform's refusals in words; the person is taken to the new run. Nothing is sent before the plan
  has been read. `ResetsMade` lists the resets made from the run with a link to each run they made (nothing for a
  platform that does not serve the register), and the header of a run that was started again, or reset, links the run
  it came from (the contract's `rerunOf`; it did not do so for a rerun either). Proved by `ResetRun.test.tsx` (17
  tests) and two of `RunDetailPage.test.tsx`; the Studio gate: lint, typecheck, 1,377 tests in 98 files, build,
  `check:api`; 21 mutants of the component, 20 killed and 1 not (the guard against a second submit while sending,
  which the disabled button and the idempotency key make unreachable). The e2e workspace typecheck needs the TypeScript
  SDK built and was not run; no browser journey covers the dialog.

## Phase 9: US07 — Existing cron triggers as schedules (P3)

- [ ] T018-060 [US7] Publication maintains trigger-owned schedules in the publishing transaction; a
  reconciliation pass keeps them equal to `workflow_triggers` (old control planes); the pass fires them
  with the 004 identity; spec edits refused — `crates/infra-postgres/src/{definitions.rs,dispatch.rs}`,
  `crates/application/src/schedules.rs`, `crates/scheduler/src/schedules.rs` — publish/republish/remove
  and reconciliation tests — FR-018-017.
  *Built 2026-11-08, not ticked, review pending (D2033 to D2045, G2004 to G2008 of `research.md`).* **Done:** publication keeps the schedules of the cron triggers in the
  publishing transaction (`crates/infra-postgres/src/trigger_schedules.rs`, called by `register_triggers`: new, kept, dropped, returned as the same schedule, a changed
  expression or zone as another trigger; a taken name never blocks it; exempt from the `schedules` quota); the reconciliation pass (`crates/scheduler/src/trigger_schedules.rs`,
  `crates/application/src/schedule_triggers.rs`, the loop `trigger schedules` of the engine: one query per round, shard of the trigger's id, 20 entries a tenant and 200 a round, per-entry
  backoff, a lock wait of one second, tenant and trigger in every log line, metrics with closed labels); the firing with the 004 identity (`crates/application/src/{schedule_firing,runs}.rs`: scope
  `trigger:{id}`, key `cron-{epoch}`, the `cron.fired` document, `system:engine`, the dispatcher's journal actor, the version the trigger names; `ALREADY_ACTED` with the run found; a
  backfill keeps the key; the engine's authority is not looked up as a membership); the legacy pass and the schedule pass on one tick; the Studio marks the schedules and says the
  refusal, `sagat schedule get` says whose it is. **Evidence (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`):** `integration_schedules_bulk_operations` 548 passed in both
  roles (42 new: `trigger_schedules` 10, `trigger_firing` 11, `trigger_reconcile` 8, `trigger_handover` 11, `catchup::trigger_owned` 2), `fault_schedules_bulk_operations` 66 in both roles (8 new:
  `upgrade`), the unit suites of eight crates 1,705, the Studio 1,435 (lint, typecheck, `check:api` clean). The tests were written after the code, not before; the proof that they fail
  against the old behaviour is the mutation battery below, and the one red they gave on the way: the backfill of a time the legacy pass had started was refused for ever
  (fixed, D2039). **Not done:** the Studio's disabled buttons before the handover (G2004); the crash matrix kills the first transaction of each pass, not the second statement of the
  legacy pass that moves the trigger on (it is one statement: the same state as a kill before it); the mutation survivors listed in T018-061's note. Mutation battery
  (hand-made, in the scratchpad, run against the suites that hold the tests): 38 mutants of the new logic (identity of the tick, the handover state and its cache, the policies, the gates, the firing, the publication, the reconciliation query, cap and backoff, the lock wait, the legacy-pass gate, the command's checks, the migration's guards): 33 killed at the first run (one of them, a lock wait with no bound, by a hang that was stopped by hand after 17 minutes), 4 survived and each was a missing test (firedAt of the document; the quota room left by the API after the triggers; a legacy pass that stops when the state cannot be read; the bound of the failures remembered), the tests were added and the four are killed, and 1 did not compile and was replaced by a mutant that does and is killed. Not mutated and so unproven: the override of the standing of a trigger-owned tick (near-equivalent: those runs are never tracked, so the open count is zero), the backoff of a refused entry, the schedule's removal by the step that finds its trigger gone is covered by a test but was not mutated.
  *Round 2, 2026-11-08, not ticked (the correctness, security and architecture reviews of the first build: items A to T, D2046 to D2058, G2009 to G2013, ADR-0083 proposed).* **Done:** the reconciliation reads through partial indexes (`workflow_triggers (run_shard(id)) WHERE kind = 'cron'`, `schedules (run_shard(trigger_id)) WHERE trigger_id IS NOT NULL`, the last-fire one), its drift query has a `statement_timeout` of 4 s, productive rounds are paced at 100 writes a second (`pause_after`), and an entry carries the stored cron text and zone unparsed so a row this build cannot parse fails its own entry and is counted (`trigger_schedule_rows_undecodable_total`); the lock order is the trigger's row and then the schedule's in Respec, SetNext, `remove_gone` and publication (slots in `tests/support/lock_order.rs`); the engine's authority needs no membership in any path (a buffered release included); `cron_pass` uses the shared `tick_identity`, `tick_reason` and `cron_document` (a golden test pins the bytes); three gauges (`trigger_schedule_backing_off`, `trigger_schedule_last_round_timestamp_seconds`, `trigger_schedule_unconverged`), three alerts in `infra/observability/alerts.yaml` and `infra/observability/tests/trigger_schedules_test.yaml`; a legacy fire after `changed_at` is warned by trigger id and counted (`cron_legacy_fires_after_handover_total{source="legacy_pass"}`); API schedule names `trigger-<32 hex>[-n]` are refused (`SCHEDULE_NAME_RESERVED`, 422); `controllable` on the schedule resources (additive, optional, only for origin `TRIGGER`; OpenAPI, `schema.d.ts`, the Studio disables the buttons with the reason, the CLI hint says whether the handover happened). **Evidence (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`, the final tree):** `integration_schedules_bulk_operations` 570 passed, 1 ignored (the measurement) in the owner role and 570 in `sagat_app`; `fault_schedules_bulk_operations` 68 in both roles (2 new: the legacy pass killed after its run committed, then each order of the two passes); unit suites of eight crates 1,708; `integration_platform_foundation` 101 in both roles; `integration_events_webhooks_messaging` 111; `fault_events_webhooks_messaging` 8; `contract_schedules_bulk_operations` 7; `contract_developer_experience` 8 (no bless); `contract_rest_security_multitenancy` 21; `contract_observability_operations` 9; `fault_platform_foundation` 38; `e2e_platform_foundation` 42; `integration_scalability_ha` 44; `sdk_conformance` 3 and 1 ignored; Studio lint, typecheck, 98 files and 1,437 tests, `npm run api` then `check:api` clean; `promtool check rules` 30 rules and `promtool test rules` for the three files (run with the CI recipe's mounts) pass; `cargo fmt --check`, clippy `-D warnings`, `check-architecture`, `check-agent-config`, `check-upgrade-boundary`, `check-design-docs` clean. **Mutation battery 2** (27 mutants of the changed logic, hand-made, in the scratchpad): 20 killed at the first run; 7 survived: 3 by construction (the activation's recount of the convergence behind the census that already refuses; the census's own check behind the step's check; `h.state = 'HANDED_OVER'` in the legacy-fire query, equivalent on the data because `changed_at` is NULL while the state is `LEGACY`) and 4 for want of a test (the census under the table lock with a publication in flight; the trigger row read under `FOR SHARE` by the removal; the unmade counter; the `controllable` flag over HTTP). The four tests were added (`a_trigger_published_while_the_handover_counts_is_waited_for_and_named_in_the_refusal`, `the_removal_of_a_schedule_waits_for_a_trigger_that_is_being_deleted_and_then_removes_it`, `a_trigger_whose_every_schedule_name_is_taken_is_counted_and_left_for_the_next_round`, `the_schedules_over_http_say_whether_the_control_of_a_trigger_owned_one_is_open_and_say_nothing_of_the_others`) and the four mutants are killed on a rerun. A red run on the old code is not recorded here for every test of this round (the four above were written after the mutants showed the gap); what is recorded for each is the mutant it kills. Measured, not extrapolated: the drift query takes 14 ms with 10,000 triggers and 122 ms with 100,000, the convergence count 5 ms and 34 ms (one developer machine, PostgreSQL 16 in Docker, other sessions running); 500 schedules were made in 1.79 s unpaced (279 writes a second), so the 100 a second pacing means about 3 minutes for 10,000 triggers of one tenant; the write measurement at 100,000 did not run (a second replica held no shard) and the 30 minutes in D2047 are an extrapolation. **Not done:** the typed `controllable` field of the six SDKs (G2009: `sdks/` was not touched); the CI step does not list `trigger_schedules_test.yaml` (nor the registry's file) and `.github/workflows` was not touched (G2012: add them to the `promtool test rules` step at the next workflow change); the lock-order test is the plan-based slot check plus a deterministic deadlock test on the real code, with no `pg_stat_database` deadlock counter; the schedule pass has no post-commit kill case (it is one transaction: a kill before it commits is the case already covered); the fire of a legacy pass at the same instant as the commit of the handover is closed by the control window, not by a test of the instant itself (G2013).
  *Round 3, 2026-11-09, not ticked (the third correctness, security and architecture reviews: items 1 to 24; D2069 to D2078, G2019 to G2023, ADR-0083 amended).* **Done:** one definition of "converged" (the view `schedule_trigger_drift`, read by the reconciliation and by the handover: a cron that never fires is marked exhausted and converges, a pause in `LEGACY` is repaired by a resume); a round reads a window of 2,000 triggers after a cursor and sweeps (a cost of the window and of the writes, not of the number of triggers), `MAX_BATCH` 500 and a pause that is not shortened, a refusal is a one-second constant pause and not a failure; the legacy pass reads the handover before each trigger; the tripwire has a baseline and does not replay after a restart; a replica that holds no shard stamps its loop as alive; a backfill or a run-now of a trigger that is gone is refused with `SCHEDULE_TRIGGER_GONE`; the reserved-name check is on a change of the name only; the dead port method and the `PgConnection` re-export are gone; three alerts and their rule tests (`SagatTriggersUnconvergedAfterHandover`, `SagatCronLegacyFiredAfterHandover`, `SagatCronHandoverUnreadable`). **Evidence (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`, the final tree):** `integration_schedules_bulk_operations` 589 passed, 1 ignored, in the owner role and 589 in `sagat_app` (+19 over round 2: `trigger_round3` 18 and `trigger_scale` 2 new, the hand-written slot test of `trigger_followup` removed); `fault_schedules_bulk_operations` 68 in both roles; the unit suites of eight crates 1,714; `integration_platform_foundation` 101 in both roles; `integration_events_webhooks_messaging` 111; `fault_events_webhooks_messaging` 8; `contract_schedules_bulk_operations` 7; `contract_developer_experience` 8; `contract_rest_security_multitenancy` 21; `contract_observability_operations` 9; `fault_platform_foundation` 38; `e2e_platform_foundation` 42; `integration_scalability_ha` 44; `sdk_conformance` 3 and 1 ignored; Studio lint, typecheck, 98 files and 1,437 tests, `check:api` clean after `npm run api`; `promtool check rules` 33 rules and `promtool test rules` of the three files pass (the CI step's mounts); `cargo fmt --check`, clippy `-D warnings`, `check-architecture`, `check-agent-config`, `check-upgrade-boundary`, `check-design-docs` clean. **Red first** (seen failing on the code of round 2, for the right reason): the definition of converged over seven states, the cron that never fires, the pause in `LEGACY`, the order of the locks (derived from `pg_locks`), the bound of the table lock with a long publication, the state the function does not know, the name written to the database, the legacy pass held across the handover, the unreadable state (two tests), the trigger that is gone, the update of a name that looks reserved. **Written after the code, proved by the mutant they kill, not red first:** the window and the sweep (new API), the restart of the tripwire, the stamp of a replica without shards, the booking of a refusal, the apply path of the command, the printing filter, the constants of the control window. **Mutation battery 3** (26 mutants of the changed logic): 25 killed at the first run; 1 survived (`refused-doubles`: the round booked a refusal as a failure, and no test looked at the booking), the booking was extracted (`book`) with a unit test and the mutant is killed; one kill is by a NULL decode and not by behaviour (`census-counts-under-lock`). **Not done:** 100,000 triggers measured with the window (20,000 were: ADR-0083; the rest is an estimate, G2023); the CI step of the alert rule tests (G2022: the line to add is in ADR-0083; `.github/workflows` was not touched); the typed `controllable` of the SDKs (G2009); a census of the control planes (G2020); a lookup of old legacy ticks in the runs (G2021, pinned by a test as the documented limit); the real fire time for the tripwire (G2019).
- [ ] T018-061 [US7] Handover gate: `schedule-trigger-handover` operator command, an activation of the
  instance-capability registry (T018-080) requiring `schedules-v1` from every live engine, legacy cron pass runs only in `LEGACY`, pause/resume/trigger refused before
  it; rolling upgrade with old and new engines on the same ticks; `cron_*` metrics kept —
  `crates/scheduler/src/events.rs`, `apps/control-plane/`, `apps/execution-engine/src/lib.rs`,
  `tests/fault/schedules-bulk-operations/upgrade.rs` — SC-018-005 — FR-018-017, FR-018-046; NFR-018-004.
  *Built 2026-11-08, not ticked, review pending (D2036 to D2043, G2005, G2008).* **Done:** migration `20270114000000_trigger_handover` (the guard, the function, `changed_db_role`, the census;
  a `down` that refuses once handed over; the privilege audit lists the two functions in `ALLOWED`; the three released-upgrade oracles and the storage classification moved);
  `control-plane schedule-trigger-handover --operator --expect-live --dry-run` (`apps/control-plane/src/{handover,cli,main,lib}.rs`, the usage text names the owner-role commands),
  an activation of the registry that requires `schedules-v1` of every live engine and writes the handover in the same transaction, idempotent, with the permission-denied path in words;
  the legacy cron pass runs only while the state is not `HANDED_OVER` (read at most every two seconds; `SAGAT_CRON_PASS` never existed); trigger-owned schedules refuse pause, resume, trigger-now
  and backfill with `SCHEDULE_TRIGGER_HANDOVER_PENDING` from the real state, and accept them after; `cron_*` metrics unchanged; the runbooks (`schedules.md` section 6 with the exact
  procedure, refusals and the rollback hazard G1011/G2005, `events.md`, `instance-registry.md`, `control-plane.md`, `observability.md`). **Evidence:** as T018-060, plus
  `trigger_handover` 11 (refuses with an engine lacking the capability and names it, passes with all, `--expect-live` fewer/more/none, dry run changes nothing, second run exits 0, the runtime role
  is told to use the owner role and cannot write the row or call the functions, nothing moves a handed-over row back, the migration's `down` battery), `trigger_firing`
  (both orders of the two passes, `ALREADY_ACTED`, the key, the document, the actor, the version, the legacy pass stopped after the handover), `fault upgrade` (each pass killed after every one
  of its writes then both passes: one run per tick, in four sequences; a race on one tick; the reconciliation and the handover cut off at each write; an old binary after the handover),
  the CLI 13 (`schedule_and_batch`), `integration_platform_foundation` 101 in both roles (the privilege audit), `integration_events_webhooks_messaging` 111, `fault_events_webhooks_messaging` 8,
  `contract_schedules_bulk_operations` 7, `contract_developer_experience` 8 (no regeneration was needed), `contract_rest_security_multitenancy` 21, `fault_platform_foundation` 38,
  `e2e_platform_foundation` 42, `integration_scalability_ha` 44; `cargo fmt --check`, clippy `-D warnings`, `check-architecture`, `check-agent-config`, `check-upgrade-boundary`, `check-design-docs` clean.
  **Not done:** a command that completes a handover whose requirement was recorded by hand (G2008); a tenant audit record of the handover (D2042); the Studio's disabled buttons (G2004); the
  released-upgrade profile of CI with an old control plane publishing during a window (the three release_upgrade oracles only moved to the new boundary).
  *Round 2, 2026-11-08, not ticked (items A, B, I, J, K, M, N, O, P, T of the reviews; D2046, D2048, D2051 to D2053, ADR-0083).* **Done:** the handover refuses while a cron trigger has no schedule or one with no next time, over all shards, inside the activation's own transaction under a table lock on the triggers, naming up to sixteen trigger ids and the count (`--dry-run` says the same; the drift gauge is not the precheck: the runbook step is "convergence is 0"); `--apply` is required to hand over (without it the command is a dry run and says so), an option-like value for `--operator` is refused, the operator name must be printable (no control, bidirectional or zero-width characters) and the log carries the OS user and host; a `schedules-v1` requirement recorded by hand while `LEGACY` is completed by the command after its census and convergence checks (the `AuditAction::ScheduleTriggerHandover` is removed); the requirement of `schedules-v1` cannot be removed while the state is `HANDED_OVER` (a `BEFORE DELETE` guard); the control of a trigger-owned schedule opens `changed_at` plus 5 s (`SCHEDULE_TRIGGER_HANDOVER_PENDING` before; the engines believe `LEGACY` for at most 2 s); an engine that registers or goes silent between the first check and the lock stops the handover with nothing recorded; the crash matrix covers a kill after the legacy pass's run committed, in both orders of the two passes, and the legacy pass reads the state of the handover at every round; the runbook says to attest from the orchestrator before step 1 and says plainly that the evidence is self-asserted and a shared owner login gives no individual accountability (one login per operator is recommended, D2042 and T). **Evidence:** as T018-060 round 2, and in `trigger_followup` (A: the refusal and the dry run; the census under the lock with a publication in flight; I: the hand-recorded requirement; M: the removal guard; N: the control window; O: the engine that goes silent; P and the upgrade matrix in `fault upgrade`; K: the tripwire; J: the flag over HTTP and the use case), `trigger_handover` (B: the command line, the operator name, `--apply`). **Not done:** a tenant audit record of the handover (D2042); the released-upgrade profile of CI with an old control plane publishing during a window; ADR-0083 is Proposed and awaits the owner.
  *Round 3, 2026-11-09, not ticked (items 1 to 9, 12, 13, 17, 20 to 24 of the third reviews; D2069 to D2078).* **Done:** `schedule_trigger_handover_complete` takes the table of the cron triggers before the handover row (as the census does), through `schedule_trigger_handover_lock()` with a two-second `lock_timeout` ("run the command again", nothing recorded; the most a writer stalls is two seconds), asks only whether anything is unconverged under the lock and names it outside, and moves only the state `LEGACY`; `changed_by` has a check and is printed through a filter, as are the OS user and host; `run_handover_command` is what `main` prints (the "dry run" sentence only for a verification), with the live count of the locked census; the step is `handover_step` of the infrastructure crate; the control window is tied to the engines' bound by a test with the real constants; a handover that cannot be read is counted and alerted on and the legacy pass goes on (the trade-off is D2075); ADR-0083 narrows ADR-0082 decision 2 and says what is measured and what is estimated; ADR-0082 and ADR-0083 and `research.md` carry the gap of the CI step. **Evidence:** as T018-060 round 3; the DST comparison of the two calendars covers Sydney, Auckland, Lord Howe (a half-hour shift), Kolkata and Kathmandu. `hand_over_now` of the test support says that it skips the command's checks; each path has a test through the real command (`trigger_handover`, `trigger_round3::the_apply_path_of_the_command_hands_over_and_says_what_the_binary_prints`). **Not done:** a tenant audit record of the handover (D2042); a reverse command (none by design); the released-upgrade profile of CI with an old control plane publishing during a window.

## Phase 10: Polish, SDKs and gates

- [x] T018-062 [P] Scenario files and expectations of `contracts/sdk-conformance.md` —
  `tests/contract/sdk-conformance/{scenarios,expected}/` — reviewed fixtures — FR-018-048.
  *Done 2026-11-06 (D1981).* Nine scenarios (`schedules-crud`, `schedules-operate`, `schedules-overlap`, `business-ids`, `start-delay`, `signal-with-start`, `update-with-start`, `batch-operations`, `reset`) with their workflow and result goldens, blessed from the TypeScript SDK after every observation was read against the REST contract and then reproduced, equal, by the other five languages without bless. The draft's `errors-018.json` is not written (each reason is asserted where it is raised) and the concurrency-permit reset is not a scenario (T018-053's permit is not built). The harness gained `server.schedules` / `server.batches` for the engine passes. Found by them: the actions of a backfill never carried `backfillId` (fixed in `5cba87e0`, with tests) and G1981 (several refusals are a plain 422 without a reason; left open and pinned as they are).
- [x] T018-063 [P] TypeScript SDK — `sdks/typescript/src/` — unit tests + scenarios — FR-018-048.
  *Done 2026-11-06.* TypeScript: `src/operations.ts`, `src/wire.ts`, `src/client.ts`; 16 new unit tests (150 in all), 29 mutants of the client killed after four tests were strengthened (the first run let four survive). Nine scenarios equal to the goldens, twice in a row, by the agent that wrote it and again in the full run of T018-068.
- [x] T018-064 [P] Go SDK — `sdks/go/client/` — unit tests + scenarios — FR-018-048.
  *Done 2026-11-06.* Go: `client/{schedules,batch,businessids,withstart,reset,wire}.go`, runner `conformance/operations.go`; 17 new tests, `go test -race ./...`, `go vet`, `gofmt` clean; 13 mutants killed. Nine scenarios equal to the goldens, twice in a row, by the agent that wrote it and again in the full run of T018-068.
- [x] T018-065 [P] Python SDK — `sdks/python/sagat_octopus_orchestrator/{client.py,sync.py}` — unit tests +
  scenarios — FR-018-048.
  *Done 2026-11-06.* Python: `operations.py`, `_wire.py`, `client.py`, `sync.py`; 26 new tests (297 in all), mypy strict, `ruff check` and `ruff format --check` clean; 13 mutants killed. Nine scenarios equal to the goldens, twice in a row, and again in the full run of T018-068.
- [x] T018-066 [P] C# SDK — `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/` — unit tests + scenarios — FR-018-048.
  *Done 2026-11-06.* C#: `Schedules.cs`, `Batch.cs`, `BusinessIds.cs`, `WithStart.cs`, `Resets.cs`, `SagatClient.Operations.cs`; 20 new test cases (472 in all), `dotnet format --verify-no-changes` clean; 13 mutants killed. `StartAsync` now returns `StartedRun` (a `Run` plus `Started`) and `ISagatClient` gained members: a custom implementer of the interface must add them. Nine scenarios equal to the goldens, twice in a row, by the agent that wrote it and again in the full run of T018-068.
- [x] T018-067 [P] Java SDK — `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/` — unit
  tests + scenarios — FR-018-048.
  *Done 2026-11-06.* Java: `Schedules`/`BatchOperations`/`BusinessIds` and the typed answers, blocking and `...Async` forms; 19 new tests (258 in all), `./mvnw -q verify` exits 0; 9 mutants killed. `start` still returns the `Run` (a final record: widening it would break `Run r = client.start(...)`, which the first CI run caught in `examples/hello-world/java`), and `startResult` returns `StartedRun(run, started)`. Nine scenarios equal to the goldens, twice in a row, and again in the full run of T018-068.
- [x] T018-068 All five SDKs against one server build, skips stated — `sdk_conformance` output
  (SC-018-013) — FR-018-048.
  *Done 2026-11-06 (six languages, not five: D1040).* `cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` over the whole suite, on one server build (the tree of this commit), without bless and without reuse: 75 scenarios, each in 6 languages, `test result: ok. 1 passed; 0 failed`, 1853 s; none skipped. The registry tests of the same target pass (3 passed). The unit suites and linters of the six SDKs were rerun by me after the agents reported: TypeScript 150, Go (`-race`, vet, gofmt), Python 297 (mypy, ruff), C# 472 (`dotnet format`), Java `./mvnw -q verify` exit 0, Rust 127 (fmt, clippy). That the goldens bite was shown by the Go agent, who forced `started` to true and saw `business-ids` fail at `/observations/4/result/started`.
- [x] T018-069 [P] CLI `sagat schedule …`, `sagat batch …`, `sagat workflow {reset,signal-with-start,update-with-start}`,
  `--business-id`/`--if-exists`/`--start-delay` on `sagat workflow run` — `apps/cli/src/{args.rs,workflow.rs}`
  — CLI tests — FR-018-047.
  *Done 2026-11-06.* `workflow run` takes `--business-id`, `--if-exists fail|use-existing`, `--reuse …` and
  `--start-delay <SECONDS>` (sent in milliseconds, once) and says when an open run already held the identifier;
  `workflow signal-with-start` and `workflow update-with-start` (`apps/cli/src/with_start.rs`); the reset is three
  commands, `workflow reset-points`, `reset-preview` and `reset` (`--acknowledge`, `--drop-business-id`;
  `apps/cli/src/reset.rs`); `sagat schedule list|get|create|update|delete|pause|resume|trigger|backfill|actions`
  (`schedule.rs`, the schedule is a JSON file given by `--definition`) and `sagat batch
  preview|create|get|list|items|cancel` (`batch.rs`). Each command is one request of the REST API and its answer in
  words; the tool decides nothing the platform decides, a refusal (403, 404, 409, 422, 429) is said and exits 1, a
  business identifier is never echoed in an error, and every request that changes something carries a key (the one
  given by `--key`, or one made for the call). Proved by `args.rs` (17 tests), `reset_and_with_start.rs` (11) and
  `schedule_and_batch.rs` (12) over a mock platform; 40 mutants of the parser and the commands, all killed after the
  tests that let 4 survive were strengthened (a space in an identifier, a request that must not be sent, a 403, the
  filters of `batch items`). The generated `docs/reference/cli.md` follows the usage text. The auditing of the runtime
  privileges reads every string literal of `apps/` too, so `businessId: DROP` is written by a serde derive.
- [x] T018-070 Cross-tenant battery over every new route and pass — `tests/contract/security-multitenancy/rest/battery.rs`
  — battery output, audited probes — FR-018-041; NFR-018-002.
  *Done 2026-11-05:* routes — `battery::every_implemented_operation_resists_an_administrator_of_another_tenant` walks every
  `x-implemented` operation of the OpenAPI, so the 21 operations of this feature are in it as they land, each with a body
  of its own in `body_of` (`signalWithStart` and `updateWithStart` among them; 20 tests of
  `contract_rest_security_multitenancy` pass, both modes of the platform targets in the other suites) and the probe of
  another tenant's binding is on record (`business_ids_rest::another_tenant_probing_a_binding_finds_nothing_and_the_probe_is_on_record`);
  passes — the schedule pass serves another tenant's due schedule while one tenant has a backlog
  (`pass::a_tenant_with_a_backlog_does_not_hold_back_another_tenants_due_schedule`), a batch job reaches only the runs of its own
  namespace and tenant whatever ids it names (`batch_security::a_job_reaches_only_the_runs_of_its_own_namespace_whatever_ids_it_names`),
  jobs, schedule decisions and bindings are never read across tenants (`batch::jobs_are_read_listed_newest_first_and_never_across_tenants`,
  `actions::the_decisions_of_a_schedule_are_not_seen_by_another_tenant…`, `business_id_lookup::the_binding_of_a_tenant_is_not_read_by_another…`),
  and the with-start routes name no holder to a caller who may not read runs.
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
  *Rest done 2026-11-09 (decisions D2084, D2085, D2087; gaps G2027, G2028, G2031, G2032).* (a) The export canary, `observability::customer_text_…` in
  `integration_schedules_bulk_operations`: the real exporter sends to a collector that keeps the bodies it receives (opt-in, `Collector::keep_bodies`), and the whole OTLP/HTTP
  JSON (the live spans of `sagat_*` targets and the spans the telemetry projector makes of the runs), every log line at every level and the Prometheus page are searched for eight
  canaries (schedule name, note, input, pause note, business id, batch reason, batch selector, reset reason) put in a schedule, an identifier, a batch job and a reset: none is found.
  **The search is shown to be able to find:** a control puts a canary into a span, a log line and a metric label the way the pipeline would export it and the search finds it on the
  wire, in the log and on the page, and `assert_absent` refuses it; the flow runs inside a root span with a trace id (the exporter takes a span only with a trace of its own or its
  parent's), spans of the scheduler, the application and the PostgreSQL adapter (not only probes) are required to have reached the collector, each class of canary is shown to have
  been carried (stored where the platform keeps it; the batch selector is refused, G2027) and the run each concerns (the schedule's, the identifier's, the batch target, the reset's
  source and its new run) is shown to have been told. One hand mutant, a span a service creates (`target: "sagat_application"`) carrying the schedule's name: red naming the class,
  restored. The REST test installs the exporter too: `http.request` spans of every call (the lookup `GET …/business-ids/{id}` among them, the identifier being in the URL) were
  exported and none carries the text (a mutant that puts the path on the span: red, restored). The canaries are `[a-z0-9-]`, so JSON writes them as they are (G2028). It found **D2084**
  (a run the schedule pass starts had no trace id and was never told to the collector; red first) and **D2087** (the same for the run a pending reset's completion makes and for a batch
  rerun's: `traces::`, red first, then `RequestContext::traced()` in `start_run`, `ResetCompleter` and the batch executor), and **G2027**: a batch selector by `businessIdPrefix` is still
  refused with a stale remediation. A trace the context has is kept (trigger-now), a generated id is 32 lowercase hex and not zero (`request_context.rs`, `traces::`; a mutant that
  replaces a kept trace: red). Only the runs the engine creates begin a trace; the operations it makes on a run that has one do not (G2032). (b) The audit rollback,
  `fault_schedules_bulk_operations` `audit_rollback::` 8 passed: the connection is killed after each writing statement (schedule created 12, updated 12, paused 7, resumed 7, deleted 5,
  batch created 8, batch cancelled 3), each time no table (`schedules` and `audit_records` included; `support/digests.rs`) changes and no audit record is left, sent again leaves exactly one,
  and a third send of the creation of a batch job answers the first and leaves no second record (a mutant that never replays a key: red); a stale revision (`RevisionConflict`), a name in use
  (`Conflict`), a missing schedule (`NotFound`) and a batch job over a limit of zero (`QuotaExceeded`) are refused by kind and leave no row anywhere. **Not covered** (G2031): the firing, the
  backfill create and cancel, trigger-now, `revoke_authority`, the item audit of the batch executor. A mutant that writes the audit of a pause outside its transaction: red. (c)
  `contract_observability_operations` 11 passed (2 new): the ten rules of the schedules, batch, handover and registry slices exist, name emitted metrics and point to the section their runbook
  has; and every label **name** of every rule is one the code puts on that metric, with its literal values held to "a word the code has somewhere" (a typo guard, not a tie to the emission), and the
  matchers that are patterns are listed and not looked at (`PATTERNS_NOT_LOOKED_AT`, one; a new one fails the test); two mutants (a wrong label name, a new pattern): red. The monitor's alert kinds are
  unchanged by this feature (five, all with a section). (d) `observability::every_ceiling_…`: for `schedules`, `schedule_backfills`, `batch_jobs`, `tenant_batch_jobs` and `pending_resets`, over
  HTTP, a limit of zero is a `429 QUOTA_EXCEEDED` with the reason and the limit of `contracts/errors.md`, counted once in `quota_exceeded_total{limit}` (one series per resource, no other label), with
  **no table changed** (the digest of every table before and after, the audit trail and the counters of the schedules included; the rate limiter's bucket is the one table a request changes) and no canary in
  the body, the audit trail, the log, the page or the wire. A refusal is not audited (D2085). A hand mutant, the wrong reason for the batch ceiling: red, restored. Every item of the task has evidence now;
  it is left unticked for the maintainer to tick if D2085 and the limits of G2028 and G2031 are accepted. Gates on the tree with these changes (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`): `integration_schedules_bulk_operations` 623 and `fault_schedules_bulk_operations` 99, `integration_platform_foundation` 104 and `integration_security_multitenancy` 126 (each also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`), `contract_observability_operations` 11, `fault_platform_foundation` 38, `e2e_platform_foundation` 42, the workspace without `sagat-tests` 2937; fmt, clippy `-D warnings`, `check-architecture.py` and its unit tests, the other checkers: clean.
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
  *More done 2026-11-06 (still open: the cron section of `events.md`, which belongs to US07, and a review by an operator).*
  `docs/runbooks/run-reset.md` (what a reset copies and promises, the points and what blocks one, the preview to read before
  acknowledging, the register, the refusals and what to do, what is not built) and `docs/runbooks/business-ids.md` (the one
  promise of an identifier, the reuse window, the start delay, **the privacy warning: an identifier is customer content, use an
  opaque reference**, the entry-wait pattern of signal-with-start, update-with-start, the refusals, what is not built). Every
  reason, status, permission, limit and command they name was read from the code, the OpenAPI and the CLI usage; the relative links
  of both resolve; `check-design-docs.py` and `check-agent-config.py` pass. The batch runbook's stale sentence about business
  ids is corrected.
- [ ] T018-074 Real-browser e2e of schedule, batch and reset flows, keyboard only, WCAG AA, failure states
  — ~~`tests/e2e/schedules-bulk-operations/`~~ `frontend/e2e/tests/` (amended, D2090: the first is the Rust e2e target of the feature, the second is the browser workspace CI runs) — SC-018-015 — FR-018-047.
  *Part done 2026-11-09 (decisions D2090 to D2105; gaps G2033 to G2042; owner question O1952). The task stays open and is **narrowed pending the owner** (O1952).* **Done:** fourteen journeys, in the browser workspace the CI `studio` job runs with the rest. `schedules.spec.ts` (4): create by an interval, find it,
  open it, pause, resume, run now, read the decisions and follow one to its run, delete; create by cron, then a name in use, a quota of zero and a stale revision in words, and a second Save after the conflict still refused; a schedule a cron trigger owns, its controls off with the reason; a **backfill** (the dialog, a range
  that is wrong in words, three whole hours acted on and listed, the same range again found acted on). `batch-operations.spec.ts` (4): the filters of the run list, count with a sample, a reason, start, follow the job to `Completed`, counters, items and the outcome filter, the list of jobs; a job at work cancelled, what was applied
  stays; items skipped with the reason, the count capped by `Most runs`, no run to take, a quota of zero; items that fail, with the code. `reset-run.spec.ts` (4): a run that ended, its points and what a reset keeps and runs again in words, the acknowledgement and the reason asked for, the new run linked back to its source and the
  source listing it; a refusal in words, the identifier held by another open run; an open run cancelled and reset when it ends, the entry waiting while the attempt in flight is held, then completed by the engine, then linked; a reset that could not be made (`ADMISSION_REFUSED`) in words, and the limit of waiting resets in the Studio's
  own sentence. `business-id.spec.ts` (2): a bad identifier, an open holder refused or opened instead, a finished run that remembers it, the run list filtered by it; the **delay of a start** (a delay that is not a number in words, one minute, the run held and saying until when, then released). Every step is a key (Tab one press at a time, Enter,
  Space, arrows, Escape); every dialog is inspected (the trap holds past the last control, every control reached has an accessible name) and takes focus and gives it back; axe (`wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa`, `wcag22aa`, no exclusion) finds nothing on the list, the form, the dialog open, the error state and the empty state of every journey.
  **Defects of the Studio the journeys and the reviews found, each fixed with a vitest test red first (D2095, D2096, D2100, D2101, D2103):** a finished batch job that said it took no run; runs and jobs called by eight characters that are the same for everything made in a minute (now `shortId` everywhere in the Studio); a Save after a stale revision that was accepted at the other
  person's revision (the fix of the page that sent the person back had refetched the form's own copy: found by review; the second save answered 200 in the real stack before the fix, 409 after), and the same in the delete dialog; focus lost after Pause, Resume and the cancellation of a job, and stolen by a flag; a dialog closed again by the `close` event of the closing before it
  (Escape and then Enter on the opener); decisions and counters not read again when a backfill goes on. **The `us04-act` failure** (D2097): the hypothesis of a lease that lapsed is disproved (a lapse sends the compensation to review, the handler ran once); the cause that reproduces is a counter shared by every run the worker serves, so a run left over in the
  namespace takes the failure of the run of the journey; fixed by keying the fixture on the run and the attempt (`failsFirstAttempt`); which run was left over in the two failures is not established. **Evidence** (this machine, macOS, headless Chromium 153, private `CARGO_TARGET_DIR`, shared and loaded: load average 5 to 11 during the runs): the fourteen journeys passed three runs
  in a row, 14 of 14 each (1 to 9 seconds a journey, the delay one 1.0 to 1.1 minutes); the whole suite as CI runs it, `cargo test -p sagat-tests --test e2e_workflow_studio --locked -- --ignored journeys`, with the sandbox: two whole runs, 52 passed and 2 skipped (the benchmark, `STUDIO_PERF`) each, 3.8 minutes, load average 5.9 to 9.4 at their start and end; five more runs of `US04` and `T037` alone (the four tests): 5 of 5 passed; `frontend/`: `npm run lint`, `npm run typecheck`, `npm run typecheck --workspace @sagat/e2e`, `npm test` (11 tooling tests; 99 files, 1,454 tests: 1,438 and the 16 of the two reviews' work and D2095 to D2103),
  `npm run build`, `npm run check:api`, `npm audit --audit-level=moderate`: clean; `rustfmt --check` of `studio_journeys.rs`: clean. **Not done:** (1) **the run on Linux** (G2037: the result of the `studio` job of the pull request, which is what re-ticks this task, with the `us04` failure explained); (2) the journeys of **the cancel of a backfill and the preview of a backfill** (G2038: the platform has no route
  for the preview, G1966); (3) the failure states of G2034 and G2035 (the preview refused for a selector too wide, `RESET_POINT_NOT_ELIGIBLE` and `RESET_REEXECUTION_NOT_ACKNOWLEDGED` from the platform, `SOURCE_DID_NOT_END`, `AUTHORITY_REVOKED`), **narrowed pending the owner: O1952**.
- [x] T018-076 Barrier test: every run-driven transition (completion, failure, timer, wait timeout,
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
  *Rest done 2026-11-09 (D2086).* **Method:** an `ACCESS EXCLUSIVE` lock on every table of every aggregate (found by name, so that one added later is locked too; each is shown to refuse a
  statement with `lock_not_available` after 300 ms), and every transition has to finish within 60 s (`within`, naming the stage): a transition that read or wrote one of those rows would wait and fail.
  It is not a `pg_locks` observation of a run-driven transaction; `pg_locks` is used by the controls, to see a path wait for its own table. The two transitions of the wait timeout and the handoff were
  in the suite already, with `business_ids` locked. The register of pending resets is now in the list: `a_source_that_a_pending_reset_waits_for_ends_without_touching_the_register` ends three sources, each with an
  entry in `PENDING_SOURCE` (a worker completes the attempt, a worker fails it, its lease runs out and the source goes to review); the completion of an entry and the request of another do wait for
  `run_resets`, and the claim of an identifier waits for `business_ids` (the controls). **The runs the aggregates name** (the first version drove runs that belong to no aggregate, so a read of
  an aggregate table conditioned on the run's kind would not have run): `the_runs_a_schedule_started_…`, `the_target_of_an_item_of_a_batch_job_…` (six items, one for each run, selected and not applied) and
  `the_run_a_cancel_other_schedule_owes_a_cancellation_…` (six debts) drive, under the locks, a completion, a failure, a lease that runs out, a deadline, a cancellation and a timer of runs of each kind.
  Hand mutants: a read of `run_resets` and a read of `business_ids` in the transaction that persists every run transition turned all three first tests red naming the stage; a **conditional** read,
  `schedules` when the run has a `schedule_id`, left the first test and the batch-item test green and turned the schedule and the debt tests red ("stage `Schedule: a worker completes` did not
  finish within 60s"), restored. A conditional read for a batch item cannot be made without reading the batch table first, which every run would wait for. Gates on the tree with these changes (private `CARGO_TARGET_DIR`, `RUST_TEST_THREADS=2`): `integration_schedules_bulk_operations` 623 and `fault_schedules_bulk_operations` 99, `integration_platform_foundation` 104 and `integration_security_multitenancy` 126 (each also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`), `contract_observability_operations` 11, `fault_platform_foundation` 38, `e2e_platform_foundation` 42, the workspace without `sagat-tests` 2937; fmt, clippy `-D warnings`, `check-architecture.py` and its unit tests, the other checkers: clean.
- [ ] T018-077 Feature 020/022 integration once they merge: every run creator through 022's T10 (schedule
  action, starts, with-start, batch rerun, rerun, reset), reset `CONCURRENCY_PERMIT_UNAVAILABLE`,
  test-tree refusals, restored rows skipped by 020 analytics, attribute fold on reset, `RUN_PURGED`
  before and during reset, `business_id` erasure — `tests/integration/schedules-bulk-operations/flow_visibility.rs`
  — SC-018-011, SC-018-012 — FR-018-050, FR-018-051, FR-018-021.
- [x] T018-078 IR field `reapplyOnReset` on Update declarations (capability-gated, ADR-0032) in the domain,
  IR codec and five SDK builders, merged from `contracts/workflow-ir.fragment.schema.json` —
  `crates/domain/src/workflow.rs`, `crates/infra-ir/src/`, `specs/001-platform-foundation/contracts/workflow.schema.json`,
  `sdks/*/` — schema/codec tests and builder fixtures — FR-018-039, FR-018-046.
  *Done 2026-11-05 (D1974, G1977):* the task text is amended: the Update declaration is in the IR codec
  (`DeclaredUpdateDefinition`), not the domain crate, which is untouched, and the SDKs are six (D1040), not five.
  Schema `$defs/update.reapplyOnReset` (boolean, default false) merged, the reference blessed after reading its one-line
  diff, `ajv` validates the example. Codec: `crates/infra-ir/tests/reapply_on_reset.rs` (5) and
  `reapply_on_reset_enabled.rs` (1): read by every parser, a non-boolean is a schema error, a new publication
  (`parse` and `declared_state_publication`) is refused with `UnsupportedDeclaration { field: "reapplyOnReset" }` while the
  capability is off, a stored version is still read, a document that does not write the field keeps the digest it had
  (pinned `e6de3b4d…7354`, taken from the code before the change) and `false` equals not writing it; the switch is
  `set_reset_reapply` of the process. Seven mutants of the codec killed (the two gates, the `== true` test, the refusal
  itself, the inverted switch, the field never read, the digest without normalisation); the eighth, the codec's own
  boolean check, is unreachable behind the schema's `type: boolean`. Six SDK builders write the field only when true, each
  with tests and a mutant killed: TypeScript 134, Python (pytest, mypy, ruff), Go (`-race`, vet, gofmt), C# (`dotnet test`
  and `format`), Java (`mvnw verify`), Rust (fmt, clippy, tests); the scenario `state-operation-builders` has the
  workflow `retained-update` and `sdk_conformance` passes for its six languages against a golden that holds only the two
  new entries. Server: infra-ir, application and cli unit suites, nine contract targets, `integration_workflow_primitives`
  755, `integration_platform_foundation` 101, `integration_schedules_bulk_operations` 337, clippy over the workspace.
  Nothing calls `set_reset_reapply` yet (G1977): a workflow cannot opt in on any deployment until T018-056.
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
  *Built 2026-11-08 and revised the same day after three independent reviews (correctness, security, architecture); not
  ticked, review pending (D2013 to D2032 and G1996 to G2003 of `research.md`; ADR-0082, proposed).* **Done:** migration
  `20270113000000_instance_registry` (it was `20270110000000` until `origin/develop` took that number for
  `step_credential_roles`; it was unpublished, so it was renamed): `serving_instances` with `token_hash` (the SHA-256 of a
  random per-start token that only the process holds; the runtime role cannot read the column), `platform_requirements`,
  `platform_requirement_epoch`, the history table with the database role that acted; six `SECURITY DEFINER` functions, three
  for a process (a process touches only the row it registered, by token) and three for the operator (not executable by
  `sagat_app`); `ENABLE ALWAYS` guards; grants; `storage-classification.json`; a `down` that refuses requirements, history and live
  processes; the three released-upgrade oracles moved to it. `crates/application/src/instances.rs` (kinds, capabilities, the rule,
  `InstanceToken`, `ServingGate` with the time of the registry's last answer, `Member`: register, beat, `fail_closed`,
  `observe`, `leave`); `crates/infra-postgres/src/instances.rs` (the process side; the operator's `activate`/`remove` under
  their own lock and statement timeouts, with the feature's step in the same transaction; the mapping of a failure to a lost
  connection). The control plane (`registry.rs`: a connection of its own, a heartbeat that asks again every second while the gate is
  closed, `closes_the_gate`, the heartbeat supervised by `serve_until_the_heartbeat_ends`, leave after the drain; the gate in
  `api-http`: middleware, `/v1/ready`, streams, uploads) and the engine (`capabilities(config)`: the capability follows the passes the
  configuration runs; every loop through `while_serving`; the keeper; the connector supervisor; the worker gRPC admission per call
  and per stream iteration; `/readyz` through `registry_state`; the heartbeat a critical task). A refused process stays up with a
  closed gate and asks again (ADR-0082). Metrics `serving_instances_live{kind}`, `instance_registry_refusals_total{reason}`,
  `instance_registry_gate_open` and `instance_registry_last_beat_age_seconds`; alerts `SagatInstanceGateClosed` and
  `SagatInstanceRegistrySilent` with a promtool test (the CI step runs it); `docs/runbooks/instance-registry.md` with links and
  metrics rows in the control plane, engine and observability runbooks; the ADR-0081 amendment; notes for 029 (G2001);
  `contracts/errors.md`, `openapi.yaml`, `schema.d.ts`, `error.rs`. **Evidence (2026-11-08, after the reviews):**
  `integration_schedules_bulk_operations` 506 passed in both roles (60 of them the registry's: `instances` 33, `instances_hostile`
  7, `instances_migration` 2, `instances_process` 11, `instances_rest` 7), `fault_schedules_bulk_operations` 58 in both roles (9 of the
  registry: the race in both orders held by a lock and watched in `pg_stat_activity`, the removal under the lock, the limit
  under 24 concurrent registrations, the fail modes, the lock-order slot), `integration_platform_foundation` 101 in both roles
  (the privilege audit lists the three operator functions), `contract_schedules_bulk_operations` 7,
  `contract_developer_experience` 8, `contract_rest_security_multitenancy` 21, `fault_platform_foundation` 38,
  `e2e_platform_foundation` 42, `integration_scalability_ha` 44, `sdk_conformance` 3 (1 ignored), the unit suites of
  eight crates 1,689, `cargo fmt --check`, clippy `-D warnings` over the workspace, `check-architecture`, `check-agent-config`,
  `check-design-docs`, `check-upgrade-boundary` and promtool (`check rules` 27 rules, `test rules`) clean. The flake
  `counted::the_items_a_cancelled_job_marks_are_counted_as_cancelled_and_the_one_that_ran_as_succeeded` seen once in the first round
  passed 40 of 40 runs single-threaded: not reproduced, not explained, not fixed. Mutants: 76 of the first round and 65 of the
  second (the token checks of the SQL, serving on a full table, `closes_the_gate`/`fail_closed`, `capabilities(config)`, the gate of the
  connector supervisor, of `while_serving`, of the readiness, of the worker admission and of the streams, the supervision of the
  heartbeat, the operator's bounds): 58 killed at the first run (one of them by a hang), 7 survived (the registration as an
  answer, the count of a lack that goes on, an epoch that went back, the pace of a closed loop, the end of the heartbeat with the
  process; and two that had been run under a suite that does not hold their test) and were killed after four tests were added.
  **Not done:** the command-line subcommand that activates and the handover (T018-061; it calls `PgInstanceStore::activate` with
  the handover write as its step, and G1997 is closed by D2023 but G2002 lists what the command must also do); moving the flags of
  batches, resets and their reapply to the registry (G1999); what a holder of the runtime role can still do (G2000: it can register
  rows of its own, hold the registration lock open, hold a victim's row lock; the database cannot close what the role legitimately
  calls); the released-upgrade profile of CI and the six-language SDK conformance with its toolchains (the
  six local `release_upgrade` tests are `#[ignore]` and need the verified v0.2.0 build). **For the owner:** D2014 (recording a
  requirement is the schema owner's, not the API's), D2016 (rows that are no longer live give way at the limit of 1,000), D2025 (a
  process that finds the table full serves, loudly, and is not counted) and ADR-0082 are the agent's decisions.
  *Left for the next touch of the workflow (2026-11-08):* `infra/observability/tests/instance_registry_test.yaml` (the promtool test of the two registry alerts) is not yet in the step of `.github/workflows/ci.yml` that runs `batch_stalled_test.yaml`: touching the workflow runs the whole stack on a pull request (Constitution 4.3.0), so the one-line change waits for the pull request that has to touch it anyway or for the release; until then the test has been run only by hand (`promtool test rules`, 27 rules).
- [x] T018-081 [P] Rust SDK (D1040) — `sdks/rust/src/`, `sdks/rust/conformance/` — unit tests + scenarios —
  FR-018-048.
  *Done 2026-11-06.* Rust: `src/client/{schedules,batch,business,resets,wire}.rs`, runner `conformance/src/runtime/operations.rs`; 16 new tests (127 in all), `cargo fmt --check`, `cargo clippy --all-targets --locked -- -D warnings` clean; 12 mutants killed. `start` returns `StartedRun` (it dereferences to `Run`); a 200 answer is accepted only when the conflict policy is `USE_EXISTING`. Nine scenarios equal to the goldens, twice in a row, and again in the full run of T018-068.
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
