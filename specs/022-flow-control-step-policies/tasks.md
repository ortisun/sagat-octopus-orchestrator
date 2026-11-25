# Tasks: Flow control and step policies

**Input**: `specs/022-flow-control-step-policies/` (spec, plan, research, data-model, contracts, quickstart)

Status: Proposed — not started. A task is checked only when its named evidence has been run and recorded
(constitution VIII).

**Format**: `- [ ] T022-NNN [P?] [USn?] Description — requirements. Evidence.` `[P]` = parallelizable.
Stories: US1 cache, US2 concurrency, US3 failure handlers, US4 early return, US5 standalone, US6 draft test.
US7 rate limits and the release 0.3.0 slice (OD-2026-11-13-11) are in [Release 0.3.0 slice](#release-030-slice-us02-and-us07).

## Phase 1: Setup

- [ ] T022-001 Owner review of spec, ADR-0056 and O1160–O1162; the coordinator integrates shared indexes. All FR. Evidence: recorded owner answers; ADR status set by a person.
- [x] T022-002 [P] IR fixtures in `tests/contract/flow-control-step-policies/fixtures/` (`order-intake.workflow.json` and one invalid document per `DefinitionProblem` rule of `contracts/errors.md`). FR-022-001,008,012,017,034. Evidence: used by T022-006 and T022-048..052.
  *Partly done 2026-11-15 (0.3.0 slice), not ticked.* `tests/contract/flow-control-step-policies/fixtures/`: two valid documents (`provider-sync`, `account-serial`), one warning (`warning-concurrency-self-child`) and seven invalid ones for the rules built now (`CONCURRENCY_POLICY_INVALID` ×2, `RATE_LIMIT_POLICY_INVALID` ×2, `FLOW_CONTROL_COMBINATION`, `UNSUPPORTED_NODE`/`SCOPED_CONCURRENCY`, `UNSUPPORTED_NODE`/`SCOPED_RATE_LIMIT`), used by `contract_flow_control_step_policies` (`ir::every_invalid_fixture_is_refused_with_its_code_before_a_version_exists`). Open: the documents of the rules not built yet, which `ir::every_rule_that_names_a_gate_is_covered_or_pending` lists with their owner (the two mismatch warnings G1187, `FLOW_CONTROL_KEY_UNBOUNDED` T022-103, `FLOW_CONTROL_KEY_SECRET`/`_PII` T022-104, `ENCODING_READ_BY_CONCURRENCY_KEY` 031); a new rule of `errors.md` fails that test until it is covered or listed.
  *Done 2026-11-16 (0.3.0 slice).* The two namespace warnings cannot be shown by one document: they are proved against PostgreSQL by `integration_flow_control_step_policies::publication::a_group_declared_elsewhere_with_another_limit_is_warned_and_nothing_else_is`, and the contract test lists them in `NAMESPACE_WARNINGS` and fails if that test or its code disappears. The fixtures of `FLOW_CONTROL_KEY_UNBOUNDED`, `FLOW_CONTROL_KEY_SECRET` and `FLOW_CONTROL_KEY_PII` move to the tasks that build those rules (follow-up notes under T022-103 and T022-104); `ENCODING_READ_BY_CONCURRENCY_KEY` is 031's (G1182), outside the slice. Evidence: `contract_flow_control_step_policies` 8 passed.
- [x] T022-003 [P] Test targets `integration_`, `fault_`, `contract_`, `e2e_`, `perf_flow_control_step_policies` in `tests/Cargo.toml` with `tests/{integration,fault,contract,e2e,performance}/flow-control-step-policies/main.rs`. Evidence: `cargo test -p sagat-tests --no-run --locked`.
  *Done 2026-11-15 (0.3.0 slice).* Evidence: the five targets in `tests/Cargo.toml` with their `main.rs`; `cargo test -p sagat-tests --locked --test integration_flow_control_step_policies --test contract_flow_control_step_policies --test fault_flow_control_step_policies --test e2e_flow_control_step_policies --test perf_flow_control_step_policies` builds and runs (11 and 8 tests in the first two; the other three hold their modules when their tasks land).

## Phase 2: Foundational (blocks every story)

- [x] T022-004 Domain policies and pure validation in new `crates/domain/src/workflow/flow_control.rs` and `crates/domain/src/workflow.rs` (cache, concurrency, handler, `EarlyReturn`, placement/region/effect/combination and `startAt` rules). FR-022-001,008,012,017,028. Evidence: unit and property tests; old definitions unchanged.
  *Done 2026-11-15 (0.3.0 slice: `ConcurrencyPolicy`, `RateLimitPolicy` and their combination rules).* `crates/domain/src/workflow/flow_control.rs` and `workflow.rs` (`Node::with_flow_control`, `WorkflowDefinition::{with_run_flow_control, check_flow_control, declares_gates}`, warning `ConcurrencySelfChild`). Evidence: `cargo test -p sagat-domain --locked --test flow_control --test flow_control_properties` — 30 + 3 passed (properties also at `PROPTEST_CASES=2000`); hand mutations caught (per-second ceiling 100→101, the attempt check); `an_ungated_definition_declares_no_gate_and_its_old_shape_is_unchanged`; whole domain suite in the workspace unit run.
- [x] T022-005 Capability gates `stepCache`, `earlyReturn`, `failureHandler`, `concurrencyKeys` in `crates/infra-ir/src/convert.rs`; configuration in `apps/control-plane` and `apps/execution-engine`. FR-022-031,036; SC-022-013. Evidence: previous corpus unchanged with gates on/off; disabled gate refuses before a version exists.
  *Done 2026-11-15 (0.3.0 slice: `concurrencyKeys`).* The gate is the definition service's (`DefinitionService::with_flow_control`, the 042 pattern) with `SAGAT_FLOW_CONTROL_CONCURRENCY_KEYS_ENABLED` read by `apps/control-plane` and `apps/execution-engine`; starts of a version whose capability is off are refused `CAPABILITY_DISABLED` at the one admission point. Evidence: `cargo test -p sagat-application --test definition_service` (53 passed; `a_gate_whose_capability_is_off_is_refused_before_a_version_exists`, `a_top_level_gate_whose_capability_is_off_is_refused_at_its_field`, `a_definition_without_a_gate_never_reads_the_registry`; mutation of the switch check caught by 2), the config tests of both apps, `contract_flow_control_step_policies::ir::the_corpus_that_declares_no_gate_reads_as_before`, and `integration_flow_control_step_policies::admission::publication_and_starts_wait_for_the_activation_and_this_build_never_admits_a_gated_run`.
- [x] T022-006 IR codec in new `crates/infra-ir/src/flow_control.rs` (closed shapes, problem codes, canonical key-expression text; with 031, its shared `EncodingCheck` for cache keys, concurrency keys and `earlyReturn.result` carriage, using 031's `ENCODING_*` codes). FR-022-001,002,008,012,017,034,036. Evidence: contract test over the T022-002 corpus; digest stability.
  *Done 2026-11-15 (0.3.0 slice: `concurrency` at the top level and on root nodes).* `crates/infra-ir/src/flow_control.rs`, wired in `lib.rs`, `convert.rs`, `continuation.rs`; canonical `workflow.schema.json` gains `concurrencyPolicy` and `rateLimitPolicy` (top level, `activity`, `script`; body variants refused before the schema as `SCOPED_*`). Evidence: `contract_flow_control_step_policies` 8 passed (owner and runtime role), red under hand mutation (surface check removed: 2 failed; publication check removed: 1; top-level key rule removed: 1); digest stability (`the_digest_is_the_canonical_one_whatever_the_order_of_the_keys`). Not in it: 031's `EncodingCheck` (031 is not on the tree; FR-022-036's `ENCODING_READ_BY_CONCURRENCY_KEY` is listed pending).
- [x] T022-007 Keyed digests in new `crates/application/src/keyed_digest.rs` (`ring::hmac`, namespace separation, labels, key version) and key configuration; activation of a new key version refused while semaphore rows exist under the old one. FR-022-002,020,022; NFR-022-006. Evidence: unit tests; redacted Debug; log canary; rotation-refusal test.
  *Done 2026-11-15 (0.3.0 slice).* `crates/application/src/keyed_digest.rs` (HMAC-SHA-256, separated by purpose, tenant and namespace, 256-byte bound, 12-hex label, `KeyRing` read from `version:hex`); the current version is the row of `flow_control_key_versions`, read shared through `flow_control_active_key_version()`, and a key row is filed only under it; `flow_control_key_activate` refuses a new version while a semaphore or window uses another, and a retired one. Evidence: `cargo test -p sagat-application --lib keyed_digest` 4 passed (redacted Debug and the log canary `nothing_printed_carries_the_key_or_the_value`; mutations of the purpose separation and of the Debug caught); `integration_flow_control_step_policies::migrations::{a_new_key_version_is_refused_while_a_row_uses_another, a_key_row_is_written_only_under_the_active_version_and_a_retired_one_stays_retired}` (mutation of the window count caught). Not wired into the apps' configuration yet: no code digests a key until the gate (T022-022).
- [x] T022-008 Journal events and reducer rules in `crates/domain/src/execution/{event.rs,state.rs,command.rs}` and new `execution/flow_control.rs`; mocked completions excluded in `execution/{sagas.rs,plan.rs}`; closed snake_case `detail` keys. FR-022-003,009,013,018,027,038. Evidence: replay test per event of `contracts/journal-events.md`; codec test that every detail key is snake_case.
  *Done 2026-11-15 (0.3.0 slice: the five `concurrency.*` events).* `RunEvent::Gate` with `GateChange`, the reducer's `apply_gate`, entry hold, `WaitingOn::Gate`, the planner's release and dequeue; reset rules `EXCLUDE` (D022-035); closed snake_case codec. Evidence: one replay test per event in `crates/domain/tests/flow_control.rs`; `event_codec::tests::every_flow_control_event_round_trips_with_closed_snake_case_detail` (red under mutation); `crates/domain/tests/reset.rs` 21 passed; the reducer findings of the correctness review fixed with tests seen failing (`reviews/2026-11-15/correctness.md`). Mocked completions are US6, not in the slice.
- [x] T022-009 Additive migration in `crates/infra-postgres/migrations/` after `20261220000000` (tables and `runs` columns of `data-model.md`, origin CHECK, triggers, `sagat_app` grants, refusing down). FR-022-005,007,016,020,026. Evidence: up/down test; `tests/integration/platform-foundation/runtime_privileges` sweep.
  *Done 2026-11-15 (0.3.0 slice: the concurrency tables, as revised; no `runs` column, no origin value).* Migration `20270130000000_flow_control_concurrency` (semaphores, holders, waiters, the key versions and its operator function; counters by `SECURITY DEFINER` triggers, the empty key deleted at commit, column-limited `INSERT`/`UPDATE` grants; a down that refuses while rows or `concurrency.*` events exist). Evidence: `integration_flow_control_step_policies::migrations` (rollback refused then allowed, forward again; counters and privileges; owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`), red first on a real trigger defect and on three review findings; `integration_platform_foundation -- runtime_privileges migrations`.
- [x] T022-010 [P] Entries in `crates/infra-postgres/storage-classification.json`. FR-022-007. Evidence: manifest test in `integration_security_multitenancy`.
  *Done 2026-11-15 (0.3.0 slice).* 30 entries in `storage-classification.json` (every variable column of the six tables and the key versions, all `protected_system`). Evidence: `integration_security_multitenancy -- manifest_covers_every_variable_size_column` passes (owner and runtime role).
- [ ] T022-011 [P] Permissions and grants in `crates/domain/src/identity.rs` and `crates/application/src/authz.rs`. FR-022-032. Evidence: catalog of 51 entries; machine roles hold none.
- [x] T022-012 [P] Error refinements in `crates/api-http/src/error.rs` and application errors per `contracts/errors.md`. FR-022-034. Evidence: one contract test per row; `sdks/error-catalog.json` codes unchanged.
  *Done 2026-11-15 (0.3.0 slice: the rows that name concurrency or a rate limit).* `FlowControlRefusal` (application) and its mapping in `api-http/src/error.rs`; `IrError::FlowControlActivationPending`. Evidence: `cargo test -p sagat-api-http --test flow_control_errors` 9 passed, one per row (`CONCURRENCY_QUEUE_FULL`, `RATE_LIMIT_QUEUE_FULL`, `FLOW_CONTROL_LIMIT`, `FLOW_CONTROL_ACTIVATION_PENDING` at start and publication, `BINARY_CAPABILITY_MISSING`, `CAPABILITY_DISABLED`, `UNSUPPORTED_NODE` forms) and `the_error_catalog_of_the_sdks_is_unchanged`; mutation 503→422 caught. The two reset rows are 018's contract (G1181).
- [x] T022-013 Quota resources and ceilings in `crates/domain/src/quota.rs`, `crates/application/src/quotas.rs`, `crates/infra-postgres/src/quotas.rs`. FR-022-034. Evidence: at-bound and over-bound test per ceiling; zero refuses (SC-022-011).
  *Not done 2026-11-15.* The ceilings need new `QuotaResource` values, which widen the `quota_limits` checks and the quota resource enum of the canonical OpenAPI, whose baseline is pinned: that is a contract change to read and approve (T022-060), and no code consumes the ceilings before the admission gate (T022-022, T022-072). Left for that change.
  *Done 2026-11-16 (0.3.0 slice: the concurrency rows of the ceilings table and `queued_runs`; D022-040).* Owner: OD-2026-11-16-2. Six `QuotaResource` values (`queued_runs`, `concurrency_live_keys`, `flow_control_key_waiters`, `concurrency_limit`, `flow_control_queue_timeout_ms`, `flow_control_node_declarations`), migration `20270130000300_flow_control_quota_resources` (the three resource lists restated; a down that refuses while a limit or ceiling of one exists), usage of the three counts in `crates/infra-postgres/src/quotas.rs`, `FlowControlCeilings` (effective = lowest of declaration and namespace) in `crates/domain/src/workflow/flow_control.rs`, the node-declarations ceiling enforced at publication; OpenAPI `QuotaResourceName` and `Quota.resource` enums and the Studio client regenerated (`npm run check:api`: matches). Baseline read: `SAGAT_CONTRACT_BASELINE_BLESS=1 cargo test -p sagat-tests --test contract_iceberg_writes -- existing_operations` gave 18 added operations (018's and 035's, which the baseline deliberately leaves out), 0 changed, 0 removed, because the digest does not follow `$ref`; the bless was therefore reverted and the baseline is unchanged; the test passes without the switch. Evidence: `cargo test -p sagat-domain --test flow_control_quota` 10 passed (at and over every bound, zero refuses, one key below `queued_runs`; hand mutations caught: waiter at the bound, limit and timeout ignoring the namespace, declarations off by one); `integration_flow_control_step_policies::publication::{each_quota_resource_of_the_gate_is_set_at_its_bound_refused_over_it_and_counted, the_namespace_ceiling_of_node_declarations_is_checked_at_publication}` (mutation removing the publication check caught) and `migrations::{the_counts_of_the_gate_read_the_waiter_and_key_rows, the_quota_and_declaration_migrations_roll_back_only_once_their_rows_are_drained}`, `publication::{the_rate_limit_declarations_and_the_default_ceilings_are_bounded_too, a_publication_retried_after_the_ceiling_was_lowered_is_replayed}` (reviews of 2026-11-16; the replay red without its routing) and `contract_flow_control_step_policies::quotas` (the OpenAPI enums and the newest `CHECK` lists equal `QuotaResource::ALL`; red with a name removed from the OpenAPI). `concurrency_live_keys` counts semaphores only (D022-043). The queue-full and live-key refusals that consume the counts land with the gate (T022-022, T022-072).
- [x] T022-014 [P] Metrics and spans in `crates/telemetry/src/{metrics.rs,span.rs}` with closed labels. FR-022-033. Evidence: telemetry tests reject unknown labels.
  *Done 2026-11-15 (0.3.0 slice: `concurrency_*`, `rate_limit_*` and the four `workflow.concurrency.*`/`workflow.rate_limit.*` spans).* `crates/telemetry/src/flow_control.rs`. Evidence: `cargo test -p sagat-telemetry --test flow_control_vocabulary` 2 passed (`every_recorded_series_uses_only_the_closed_vocabulary` rejects an unknown label; a `tenant` label added by hand failed it). Nothing records them until the gate exists (D022-037).

**Checkpoint**: stories may proceed in parallel; ADR-0056 recommends US2 first.

## Phase 3: US1 — Reuse a step's recent result (P1)

- [ ] T022-015 [US1] Pure cache decision (fingerprint, scope, outcome) in `crates/domain/src/execution/flow_control.rs`. FR-022-002..004. Evidence: property tests; different key expressions never share entries.
- [ ] T022-016 [US1] Hit in the readiness transaction (T1) under the run lock, taking no shard fence (worker completions hold none), with the hit provenance row: new `crates/application/src/step_cache.rs`, new `crates/infra-postgres/src/step_cache.rs`, `crates/infra-postgres/src/execution.rs`. FR-022-003,004,037. Evidence: no lease on hit when readiness comes from a worker completion and from a scheduler pass; barrier against invalidation both orders; `verify_run`.
- [ ] T022-017 [US1] Store obligation (T2) and leased writer (T3) in new `crates/scheduler/src/flow_control.rs`. FR-022-005. Evidence: stored, duplicate, quota-full, blob and backlog cases leave step and run unchanged.
- [ ] T022-018 [US1] Secret-usability recheck on hit via `crates/application/src/worker/secrets.rs`. FR-022-004. Evidence: revoked publisher turns a hit into a miss; no secret resolved.
- [ ] T022-019 [US1] Invalidation and listing routes in new `crates/api-http/src/routes/step_cache.rs`; purge in `crates/scheduler/src/passes.rs`. FR-022-006,007. Evidence: REST tests; audit without key values; byte release.
- [ ] T022-020 [P] [US1] Fault suite `tests/fault/flow-control-step-policies/cache.rs` (crash around T1, T2→T3, duplicate obligation, concurrent misses, lost hint). SC-022-001,002. Evidence: passes with `assert_consistent`.

## Phase 4: US2 — Limit concurrent work per business key (P1)

- [x] T022-021 [US2] Permit model in `crates/domain/src/execution/{flow_control.rs,state.rs}` (queued wait, acquire, release, timeout, dequeue). FR-022-018,019,021. Evidence: domain replay tests.
  *Done 2026-11-20 (0.3.0 slice).* The permit model of the foundation (`GateSlot`/`GatePhase` in `crates/domain/src/execution/flow_control.rs`, `RunState::apply_gate` in `state.rs`: queued, acquired or granted, released or returned, timed out, dequeued; run and step levels), with the re-decision of an attempt whose slot was given back unleased (T022-100). Evidence (2026-11-20): `cargo test -p sagat-domain --test flow_control` 30 passed (one replay test per event, run and step), `--test flow_control_properties` 3 passed, `--test readiness_gate` 20 passed; hand mutations caught: `dequeued` mapped to `Ended` (2 tests red), `wait_timed_out` mapped to `Dequeued` (it survived the suite until the timeout and cancellation tests gained phase assertions this day; red since).
- [x] T022-022 [US2] Acquire or enqueue at admission (T10) through one permit port used by every run creator of the plan's table (start delay acquires at release; a reset acquires a fresh permit or is refused, never queues; restored events ignored): ports in new `crates/application/src/concurrency.rs`, use in `crates/application/src/runs.rs`, semaphores at slot 7 of ADR-0050's merged admission order, SQL in `crates/infra-postgres/src/execution.rs`; key error ends the run (D1180). FR-022-018,021. Evidence: API, event, cron and child starts; queue-full codes.
  - *Amended 2026-11-13 after review* (D022-031, D022-033): the gate runs inside `PgRunStore::create`; invariants 1–2 of `data-model.md` land in this task's change, not in T022-027; the schedule-firing creator (`crates/application/src/schedule_firing.rs`) is one of its cases. Needs T022-091 first.
  - *Built 2026-11-20 (0.3.0 slice; D022-049, D022-050), not ticked: the event-trigger and cron cases of the evidence are T022-064's, still to be written.* `Engine::start` (every creator: `RunService::finish_start`, `scheduler/events.rs`, the child start in `scheduler/passes.rs`) evaluates the start key and asks `RunTx::decide_run_gates`; the adapter (`readiness_gate.rs::decide_start_gates`) files and locks the key after the run's row, counts it under its lock (holders plus waiters below the limit, D022-021), and writes a holder or a waiter row; refusals `CONCURRENCY_QUEUE_FULL` and `FLOW_CONTROL_LIMIT` (`queued_runs`, live keys, a zero effective limit or timeout), each counted `refused_full` at run scope; a key error ends the run `FAILED`/`EXPRESSION_ERROR` (D1180). The start's gates are projected on `runs.flow_control_gates` (migrations `20270203000000`, `20270203000100`) by a statement of its own, only in a commit with a run-scope gate event; the grant and timeout routines serve run waiters; a commit that ends a gated run locks its active-run counter row first (`run_active_counts_lock_slot`); `verify_run` checks invariants 1 and 2 for the start (rows, deadline, the key's counters). `start_run`, `start_run_delayed` and `record_reset` refuse a gated definition; `run_start_errors` of `scripts/check-architecture.py` refuses a domain start call outside `Engine::start`. A start delay, a reset, a start's `rateLimit` and a start gate with a `forEach`/`while`/`repeatUntil` are refused `BINARY_CAPABILITY_MISSING` (G022-009, G1197). Evidence (2026-11-20): `cargo test -p sagat-domain --test flow_control` 35 passed; `integration_flow_control_step_policies` 73 passed (`run_admission` 12: permit held and given back, queue and grant, reservation, timeout, cancellation, key error, the three refusals, a child start, collection and loop refused, counter-row lock order with a held lock and a `NOWAIT` probe, two seeded-inconsistency cases), owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`; seen failing first (all 7 first tests red with run-level gates refused at the door); hand mutations caught: the reservation rule (holders only), the start's release skipped, the grant pass and the timeout pass listing step keys only, the projection not written, the `queued_runs` ceiling off, the live-keys check skipped, invariants 1–2 off, the collection/loop refusal off, the counter-row pre-lock off, the domain's gated-start refusal off. Budget: `integration_claim_transition_efficiency` 22 passed, plain lines unchanged (lease 38/2/5, completion with a next step 50/1/11, completion that ends the run 47/1/9, admission 33/1/11); new lines: granted start (47, 1, 15), queued start (47, 1, 12), completion that ends a gated start (58, 1, 13). Also fixed while gating (2026-11-20): the grant and timeout passes failed every tick on a schema that predates the gate tables (`integration_schedules_bulk_operations::delay::a_schema_that_predates_the_delay_has_nothing_to_release_and_the_pass_does_not_fail`, red in the gate run, `failed: 2`); they now find nothing there (`undefined_table`). And the oracle of `pending_resets_migration::on_a_clean_schema_the_rollback_restores_the_checks_and_the_trigger_and_the_up_applies_again`, which feature 022's later quota and reset-reason migrations broke, reads this migration's own lists. Reviews in `reviews/2026-11-20/start-*.md`.
  *Done 2026-11-20 (0.3.0 slice, as amended; the creators that exist).* The event-trigger and cron cases are T022-064's
  (`run_creators::{an_event_trigger_admits_through_the_key_and_retries_an_event_the_key_refuses,
  a_cron_trigger_admits_through_the_key}`), the API and child cases and the queue-full codes `run_admission`'s; the start
  delay acquires at its release and a reset takes a fresh permit or is refused and never queues (D022-052, D022-053,
  G022-009 closed for both). Evidence: `integration_flow_control_step_policies` 94 passed, owner and
  `SAGAT_TEST_AS_RUNTIME_ROLE=1` (2026-11-20).
- [x] T022-023 [US2] Release, shard-fenced grant (T11) and queue timeout (T12) in `crates/scheduler/src/flow_control.rs` and new `crates/infra-postgres/src/concurrency.rs` (ADR-0056), with skip-ahead for waiters their own limit cannot admit and, with 019, (019 priority 1 first, enqueue time asc) order among them (D1602). FR-022-018,020. Evidence: admission-order grants; older `limit: 1` waiter does not block a younger `limit: 5` waiter; fence rejects a stale replica.
  - *Amended 2026-11-13 after review* (D022-021, D022-022, ADR-0089 proposed): the grant and timeout of this task are built once, for both gates, by T022-097 and T022-098; this task keeps release (T8, T14) and the next-eligible hint.
  - *Done 2026-11-20 (0.3.0 slice, as amended).* Release at the run's end (T8: the planner's `concurrency.released` before `run.finished`, the holder row deleted under the key's lock by `persist`, after the run's active-run counter row, D022-050 item 8) and at an attempt's end (T14: `give_back_ended_gates`), each setting the key's next-eligible hint when it has waiters (`key_is_eligible`, early, never late); the grant (T26) and the timeout (T27) of T022-097 and T022-098 now serve run waiters as well (T022-022). Evidence: `integration_flow_control_step_policies` 73 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1` (2026-11-20, head `64b6f2d9`), among them `run_admission::{a_start_with_room_holds_a_permit_runs_and_gives_it_back_with_its_last_event, a_start_without_room_waits_and_is_granted_when_the_holder_ends, a_cancellation_dequeues_a_queued_start_and_frees_a_held_permit, a_run_that_ends_locks_its_active_run_counter_before_its_permits_key}` and `queue::{a_released_permit_is_granted_to_the_oldest_waiter_by_the_next_pass (the hint set by the release, then rested), a_waiter_queued_behind_an_ineligible_one_is_granted_without_a_release, a_stale_replica_is_refused_by_the_key_fence}`; hand mutations caught: the start's release skipped, the grant pass listing step keys only (T022-022's evidence).
- [x] T022-024 [US2] Step permits at claim (T13) in `crates/infra-postgres/src/workers.rs` and release at attempt end (T14) in `crates/application/src/worker.rs`; step queue timeout. FR-022-019. Evidence: no attempt spent while waiting; timeout uses the failure route.
  - *Amended 2026-11-13 after review* (D022-019, ADR-0089 proposed): no permit at claim. A step with node-level `concurrency` waits outside `READY` (T28 of the plan) and is granted by T022-097; release at attempt end and at T29. Evidence: SC-022-024 (T022-099) and no attempt spent while waiting. If the owner picks O1169 (b), this task becomes the refusal of node-level `concurrency`.
  *Done 2026-11-20 (0.3.0 slice, as amended: no permit at claim, ADR-0089 and ADR-0094).* Evidence already built by
  T022-096..100 and rerun on this tree: a step of a node that declares `concurrency` waits outside `READY` and spends no
  attempt while it waits (`readiness::*`, whose helper asserts "waiting spends no attempt"; `queue::a_waiter_past_its_deadline_fails_through_its_route`,
  attempt 0, the failure route takes it); it is granted by the grant routine (`queue::a_released_permit_is_granted_to_the_oldest_waiter_by_the_next_pass`);
  its permit is given back at attempt end (`readiness::a_worker_completion_releases_a_gated_step_into_its_gates`) and on
  pause, review, cancellation and run end (T022-100's `returns::*`); SC-022-024 is T022-099's `gated_window`. Rerun 2026-11-20:
  `integration_flow_control_step_policies` 97 passed and `integration_claim_transition_efficiency` 22 passed, owner and
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
- [x] T022-025 [US2] Cancellation and run-deadline dequeue, pause keeps permit, paused waiters skipped, lowered limits, same-key child and limit-mismatch warnings, active-run accounting and the namespace `queued_runs` ceiling. FR-022-021. Evidence: one integration test per item.
  - *Note 2026-11-20 (D022-048, G022-007):* "pause keeps permit" holds for a leased step and for a run holder; a granted, unleased step gives its permit back on pause (FR-022-046, T022-100).
  *Done 2026-11-20 (0.3.0 slice; run permits of US02, step permits as T022-024 and T022-100 built them).* One integration
  test per item: cancellation dequeue `run_admission::a_cancellation_dequeues_a_queued_start_and_frees_a_held_permit`;
  run-deadline dequeue `lifecycle::a_run_deadline_dequeues_a_queued_start` (new); pause keeps a run holder's permit
  `lifecycle::a_paused_run_keeps_the_permit_of_its_start` (new; a granted, unleased step gives it back, G022-007,
  `returns::*`); paused waiters skipped `queue::paused_heads_do_not_hold_back_the_waiters_behind_them_and_a_resume_rearms_the_key`;
  lowered limits `lifecycle::a_lowered_limit_revokes_nothing_and_applies_to_the_next_start` (new); same-key child warning
  `cargo test -p sagat-domain --test flow_control a_child_of_the_same_workflow_under_a_run_permit_is_warned_about` and
  `contract_flow_control_step_policies::ir`; limit-mismatch warning
  `publication::a_group_declared_elsewhere_with_another_limit_is_warned_and_nothing_else_is`; active-run accounting
  `lifecycle::a_queued_run_counts_as_an_active_run` (new); `queued_runs` ceiling
  `run_admission::{a_start_beyond_a_queue_ceiling_is_refused_and_nothing_of_it_exists,
  two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other}`. `lifecycle` 4 passed, owner and
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Hand mutations caught: the namespace limit ignored (the lowered-limit test, red after its
  first version was found too weak: declared 2 with two holders could not tell), a stopped start never dequeued (the
  deadline test). The pause and active-run tests pin behaviour this change does not write (no code to mutate).
- [x] T022-026 [US2] `Run.concurrency` in new `crates/api-http/src/dto_flow_control.rs`; `GET .../concurrency-keys` in new `crates/api-http/src/routes/concurrency.rs`. FR-022-022. Evidence: REST tests; raw-key canary absent.
  *Done 2026-11-20 (0.3.0 slice; D022-055).* `Run.concurrency` (`HOLDING`/`WAITING`, pool, group, `keyLabel`, `since`,
  `queueDeadline`), read in `get` and `list` from the run's holder and waiter rows (`run_permit_lateral_sql!`), absent
  for a run that neither holds nor waits; `GET /namespaces/{namespaceId}/concurrency-keys` (`crates/api-http/src/routes/concurrency.rs`,
  `dto_flow_control.rs`; `OperationsService::concurrency_keys`, `ops:read`; port `FlowControlKeyReader` in
  `crates/application/src/flow_control_keys.rs`): oldest wait first, nobody-waiting keys last, exact keyset paging by an
  opaque versioned cursor, `group` filter, `queuedRuns` read by the one statement the ceiling reads (`QUEUED_RUNS_SQL`).
  Evidence: `integration_flow_control_step_policies::rest::{a_run_shows_the_permit_it_holds_or_waits_for_by_its_label_only,
  the_namespace_lists_its_keys_oldest_wait_first_and_pages_them_exactly, the_keys_of_a_namespace_are_its_tenants_and_need_ops_read}`
  3 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1` (the raw key value `acct-canary-5517` is in no body; the label is
  the first six bytes of the row's digest; the deadline equals the waiter row's to the microsecond; 404 for another
  tenant's namespace, 403 for a worker identity, 400 for a cursor not written by the server, 422 for an invalid group);
  `cargo test -p sagat-application --lib flow_control_keys` 2 passed. Hand mutations caught: a waiting permit shown
  `HOLDING`, the cursor comparison `>=` (a page repeats a key), the group filter removed. The test API gained
  `Options::flow_control`. Not in it: `keyLabel` `null` for a caller who may not disclose (T022-104); the route in the
  canonical OpenAPI (T022-060).
  *Revised 2026-11-20 (keys reviews; D022-058..060):* the cursor is the position sealed for its namespace under the active
  digest key version (400 for an altered, forged or foreign one); `keyLabel` only for a caller who may reveal personal
  data (`null` otherwise) and the key's instants cut to the minute for any other caller; a page walks an index in its
  order (`oldest_wait_at` kept by the count trigger) with a 2 s statement timeout; the route and `Run.concurrency` in the
  canonical OpenAPI with 400 and 422 (the battery attacks it). Evidence (2026-11-20, `40854f4d` and its successor):
  `integration_flow_control_step_policies` 124 passed owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1` (the four `run_creators`
  delayed-start tests after waiting on the database clock), among them `rest::{a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data,
  a_listing_shows_only_its_own_namespaces_keys_and_queued_runs, a_page_of_keys_reads_only_its_own_keys}`;
  `contract_rest_security_multitenancy` 21, `contract_developer_experience` 8, `contract_payload_classification` 18,
  `contract_rest_platform_foundation` 9 passed; `cargo test -p sagat-application --lib` (keyed_digest, flow_control_keys)
  passed. Hand mutations caught: labels always revealed, the listing's order off its index, the namespace filter
  removed, the cursor's associated data without the namespace.
  *Evidence added 2026-11-20 (reviews of 34b8be41 and 39157608; D022-061..063).* `rest::a_key_label_is_shown_only_to_a_caller_who_may_reveal_personal_data` compares each key instant with the holder's and the waiter's `since` (whole when revealed, its minute otherwise; red with the cut applied to revealers, by hand); `operations::tests::an_instant_is_cut_to_its_minute_toward_the_past`; `rest::a_cursor_is_sealed_under_the_active_version_and_one_of_a_forced_version_is_refused` (red with the version check removed, by hand). The listing for callers without labels is T022-111.
- [x] T022-027 [US2] Invariants 1–2 of `data-model.md` in `verify_run` (`crates/infra-postgres/src/execution.rs`) and `assert_consistent` (`tests/support/engine.rs`). FR-022-020. Evidence: fails on seeded inconsistency, passes after every fault case.
  - *Amended 2026-11-13 after review* (D022-033): reduced to the consistency assertion in `tests/support/engine.rs` and the seeded-failure tests; the `verify_run` checks land with T022-022.
  *Done 2026-11-20 (0.3.0 slice, as amended; D022-056).* `World::assert_consistent` (every test of the suites) also runs
  `support::flow_control::key_consistency` across the tenant in one `REPEATABLE READ` snapshot: counters equal rows;
  each holder row is a `HELD` permit of its run's or step's gate projection (attempt, pool, group, label) and each waiter
  row a `QUEUED` one (and its deadline to the microsecond); each `HELD`/`QUEUED` projection has its row; every run a row
  names is verified against its journal. Evidence: `integration_flow_control_step_policies::consistency` 5 passed (one
  seeded inconsistency per check, written with the tables' user triggers off: a counter, a holder row the journal does
  not hold, a held permit without its row, a waiter deadline off by one microsecond, a queued permit without its row);
  the whole suite (113) and `fault_flow_control_step_policies` (8) pass with the assertion, owner and
  `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Hand mutation caught: the deadline equality removed (the deadline test red).
- [x] T022-028 [P] [US2] Race and fault suite `tests/fault/flow-control-step-policies/concurrency.rs` (grant vs timeout ×20, crash at T10/T11/T12/T14, takeover duplicate grant, lost hint, restart). SC-022-003,004. Evidence: results recorded as ADR-0056 evidence.
  *Done 2026-11-20 (0.3.0 slice: a run's permit; the step permit's races are `queue::*`, T022-098).*
  `tests/fault/flow-control-step-policies/concurrency.rs`: a crash (the deciding backend terminated while it waits for a
  key the test holds) at T10 (no run, no count; the retried request queues once and replays), T11 (still queued; the next
  pass grants once), T12 (still queued; the next pass times out once) and T14 at the run's end (nothing ended or
  released; the retried report releases once and the waiter is granted); grant versus timeout of a queued start, 20
  rounds in three orders, exactly one outcome and nothing decided twice; a takeover of the key's shard waits for a grant
  in flight (the grant holds the shard row from its fence to its commit), so nothing is granted twice; a lost
  `task.ready` delivery delays nothing the next poll finds; a restarted process (fresh store and scheduler) grants the
  queue. Evidence: `fault_flow_control_step_policies` 8 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`, every run
  checked by `assert_consistent` (T022-027). Hand mutation caught: both deadline guards of the grant removed (the
  listing's filter and the check under the key's lock: the race red). Recorded as ADR-0056 evidence by this entry.
  *Revised 2026-11-20 (keys review, correctness M1 and M2):* the race has four orders, the fourth making the grant decide
  about a start that becomes due while the grant waits for the key (`queue.rs` has the step twin); the release crash runs
  20 rounds on one world. Evidence: `fault_flow_control_step_policies` 8 passed owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
  hand mutation caught: only the grant's check under the key removed (round 3 red in both suites).
- [x] T022-029 [P] [US2] Lock-order barriers `tests/integration/flow-control-step-policies/concurrency_locks.rs` for each per-path order of ADR-0056 decision 2 (admission, grant, timeout, terminal, claim against attempt end, cancellation, policy edits). FR-022-019,020. Evidence: no deadlock; reversed-order control waits as expected.
  - *Amended 2026-11-13 after review* (D022-026): also the slot-8b counter rows of `flow_control_counts` after the semaphores, and the merged routine of T022-097 against attempt end and cancellation.
  *Done 2026-11-20 (0.3.0 slice: a run's permit).* `tests/integration/flow-control-step-policies/concurrency_locks.rs`
  runs each real path against a reference in the slots' order (`support::lock_order::against_at`, `pg_blocking_pids`):
  the end of a gated run (run, then its active-run counter row, then the key; from both starts), a cancellation of a
  queued start (run, key), the timeout of a queued start (run, key), and the control: a plan in the reversed order is a
  deadlock. The grant takes a run with `SKIP LOCKED`, so it never waits for a run while it holds a key; admission
  (`run_admission::two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other`), readiness (`readiness::*`
  sorted lock order) and the pause's returns (`returns::*`) have their held-lock tests. Evidence: `concurrency_locks` 4
  passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Hand mutation caught: the counter-row pre-lock at a run's end off
  (the end-of-run test red). Not in it: `flow_control_counts` (T022-105) and policy edits (a lowered limit is read
  without a lock: `lifecycle::a_lowered_limit_revokes_nothing_and_applies_to_the_next_start`).
  *Revised 2026-11-20 (keys review, correctness M3 and m1):* `an_admission_takes_its_counter_row_before_its_key` (every
  counter slot of the namespace held, then the key); cancellation and timeout take the run, its counter row, then the key,
  from both starts. Hand mutation caught: the key locked before the runs insert (the admission test red, a deadlock).

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
  - *Amended 2026-11-13 after review* (D022-021): grant lag measured per key with three engines (as T022-089).
- [ ] T022-063 Erasure interplay with 020 (T18): delete entries by `source_run_id` and the run's obligations, keep hit provenance and pins discoverable by source, register `step_cache_entries`, `step_cache_hit_provenance` and pin provenance in 020's content-store registry `crates/infra-postgres/run-content-references.json` (hook: invalidate by `source_run_id`) and classify every column in `storage-classification.json`. FR-022-037; D1199. Evidence: SC-022-016 test against 020's manifest-driven reference check; next lookup misses after erasure.
- [x] T022-064 Run-creator conformance `tests/integration/flow-control-step-policies/run_creators.rs`: one case per creator of the plan's Run creators table (with 018 for schedule, batch, reset, Signal/Update-with-Start, start delay), failing when a creator bypasses the permit port; plus the 018 reset corpus for every rule of the reset-rule table (refused handler-phase points, fresh permit or `CONCURRENCY_PERMIT_UNAVAILABLE`, provenance and result re-materialized, no inherited `handler_for`). FR-022-018,020,038; D1198, D1600. Evidence: SC-022-015.
  *Done 2026-11-20 (0.3.0 slice: the creators that exist, 018's included; D022-052, D022-053).* One case per row of
  design 043's table of creators, the complete list of what creates a run on this tree, in
  `tests/integration/flow-control-step-policies/run_creators.rs` (each asserts `run.started` then `concurrency.acquired`
  with a holder row, or `concurrency.queued` with a waiter row, and `verify_run`): the start API with a business id
  (row 1), Signal-with-Start and Update-with-Start (row 3), `RERUN_AS_NEW` and a batch `RERUN` (row 4), a reset and a
  pending reset (row 5), a schedule's action, a backfill and trigger-now (rows 6 to 8), an event trigger (row 9), a cron
  trigger (row 10), an ad hoc workflow and its private rerun (row 13), a delayed start (feature 018); the child node (row
  11) is `run_admission::a_child_run_of_a_gated_definition_is_admitted_through_its_key`; a continuation successor (row
  12) cannot carry a gate (`a_definition_that_continues_as_new_cannot_gate_its_start`,
  `admission::a_gated_definition_that_reaches_a_continuation_successor_is_refused`). Built for it: the release of a
  start delay decides the gates through the planner (`Evaluation::StartGates`, D022-052; a ceiling's refusal is
  deferred to the next pass; the namespace's admission lock keeps `queued_runs` exact,
  `a_release_and_a_start_at_the_queued_runs_ceiling_are_decided_one_after_the_other`); a reset takes a fresh permit
  right after `run.reset` or is refused `CONCURRENCY_PERMIT_UNAVAILABLE` (409; a pending reset fails with that reason;
  never queues), the source's acquisitions restored as history (D022-053); the reset rules of the five `concurrency.*`
  events of `contracts/journal-events.md` (`cargo test -p sagat-domain --test reset` 21 passed,
  `the_rules_are_the_ones_of_the_contract` red before the registry changed). Also: a schedule's time the key refuses
  stays owed, an event the key refuses is retried, and a queued Signal-with-Start keeps 018's typed refusal with
  nothing created (G022-010, the hold-and-deliver is T022-109). Evidence: `integration_flow_control_step_policies` 94
  passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1` (`run_creators` 20); `cargo test -p sagat-domain` 998 passed
  (`flow_control::{a_delayed_gated_start_asks_for_its_gates_at_the_release,
  a_reset_clears_the_restored_permit_and_asks_for_a_fresh_one_that_never_queues}`). Hand mutations caught: the event
  dispatcher starting a run around the port (event and cron cases red), the planner never asking the start's gates
  (delay and reset red), the restored step permits not retired, the restored returns persisted, the release without
  the admission lock (the race red), a reset that queues (reset and pending reset red), a deferral counted as a
  failure. Not in the slice: the handler-phase points, `result.returned` and `handler_for` rules (US1, US6), the
  `rate_limit.*` rules (T022-086), draft-test trees (US6).
  Reviews of 2026-11-20 (`reviews/2026-11-20/creators-*.md`; security 2 Major, correctness 3 Major, architecture 1
  Blocker, 6 Major), resolved in D022-054: a refused release is held back with a backoff (migration `20270204000000`,
  `deferred_releases_back_off_and_do_not_hold_back_the_starts_behind_them`), counted once, and only `start_release`
  defers; the reducer fails closed (`flow_control::nothing_is_released_before_every_gate_of_the_start_holds`,
  `the_reducer_refuses_a_reset_that_queues`); one `StartCause` and one validator; the reset's projection rewritten and
  `refused_reset` counted; the restored marker only on `concurrency.*`; a pending reset retries
  `BINARY_CAPABILITY_MISSING`; the 409 and `CONCURRENCY_QUEUE_FULL` raised in 018's contract test with their sentences
  (`contract_schedules_bulk_operations` 11 passed; `cargo test -p sagat-api-http --test flow_control_errors` 10 passed,
  both red first); the reviewers' test gaps closed (`a_reset_whose_prefix_gave_a_step_permit_back_restores_it_as_history`,
  `a_reset_never_takes_a_free_permit_a_queued_run_waits_for`). Hand mutations of the fixes caught: the restored returns
  persisted (now red), the deferral filter off, a reset's reservation ignoring waiters, the reducer's fail-closed check
  off, the reducer letting a reset queue.
  - *Amended 2026-11-13 after review* (D022-031): schedule firing is on `develop` and is a case now, not pending; reset, batch rerun and with-start stay pending until 018's branches merge.
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
| T022-001 | Owner review | replaced for the slice by T022-066 (ADR-0056, ADR-0088, O1163–O1168); O1160–O1162 stay with their stories |
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

- [x] T022-066 Owner gate for the slice: OD-2026-11-13-11 present in `docs/owner-decisions.md`; answers to O1163–O1168 recorded there and in `research.md`; ADR-0056 and ADR-0088 accepted or changed by the owner. FR-022-017..022,039..050. Owner-gate: every task below that builds behaviour. Evidence: the recorded answers; ADR status set by a person.
  - 2026-11-13, partly done: O1163–O1168 answered, every recommendation accepted ("Aceitar todas", OD-2026-11-13-16, recorded in `research.md`; the row in `docs/owner-decisions.md` is added on `develop` by the coordinator). Not done: ADR-0056 and ADR-0088 are still Proposed; the task stays open until a person sets their status.
  - *Amended 2026-11-13 after review* (D022-023): ADR-0056 and ADR-0088 were accepted on 2026-11-13 (OD-2026-11-13-17, commit `1a21cbdc`). At the review revision, `origin/develop`'s `docs/owner-decisions.md` holds none of OD-2026-11-13-11, -16, -17 (checked with `git show origin/develop:docs/owner-decisions.md`), so the task stays open until the coordinator's rows land; ADR-0089 and O1169 are T022-107.
  *Done 2026-11-15 (0.3.0 slice).* Evidence: `docs/owner-decisions.md` on `develop` (`f0c0c3c2`) holds
  OD-2026-11-13-11 (throughput control in 0.3.0), OD-2026-11-13-16 (O1163–O1168, every recommendation) and
  OD-2026-11-13-17 (ADR-0056 and ADR-0088 accepted), added by `69dc08e2`; `research.md` records the answer under each
  of O1163–O1168; the status lines of ADR-0056 and ADR-0088 read `Accepted (2026-11-13, OD-2026-11-13-17)`, set by
  `1a21cbdc`. `python3 scripts/check-design-docs.py` passes.
- [ ] T022-067 [P] [US2] Rust SDK `concurrency` builder in `sdks/rust` (the sixth SDK, D022-016). FR-022-035,050. Evidence: unit tests; canonical fixture byte-identical to the other five.
- [x] T022-068 [US7] `RateLimitPolicy` and its validation in `crates/domain/src/workflow/flow_control.rs`; events `rate_limit.queued/granted/wait_timed_out/dequeued`, wait kind `RATE_LIMIT` for runs and steps and their reducer rules in `crates/domain/src/execution/{event.rs,state.rs,flow_control.rs}`. FR-022-039,045. Evidence: unit and property tests; one replay test per event of `contracts/journal-events.md`; old definitions and journals unchanged.
  *Done 2026-11-15 (0.3.0 slice).* `RateLimitPolicy` and `RatePeriod`, the five `rate_limit.*` events (`returned` with them) and the gate wait (`WaitingOn::Gate`, per-gate state in `Gates`). Evidence: as T022-004 and T022-008 (`flow_control` 30, `flow_control_properties` 3 at 2,000 cases; one replay test per event; codec round trip); old journals unchanged (the workspace unit run, `reset.rs`).
- [x] T022-069 [US7] IR codec for `rateLimit` and the `rateLimits` gate in `crates/infra-ir/src/{flow_control.rs,convert.rs}`, with `RATE_LIMIT_POLICY_INVALID`, `SCOPED_RATE_LIMIT`, `FLOW_CONTROL_COMBINATION` and the `RATE_LIMIT_MISMATCH` warning. FR-022-039,050; SC-022-013. Evidence: contract test over the fixture corpus of T022-002; previous corpus unchanged with the gate on and off; a disabled gate refuses before a version exists.
  *Partly done 2026-11-15 (0.3.0 slice), not ticked.* Built: the `rateLimit` codec (top level and root nodes, `RATE_LIMIT_POLICY_INVALID`, `SCOPED_RATE_LIMIT`, `FLOW_CONTROL_COMBINATION`) and the `rateLimits` gate (`SAGAT_FLOW_CONTROL_RATE_LIMITS_ENABLED`), with the evidence of T022-005/006. Open: the `RATE_LIMIT_MISMATCH` warning, which needs the declarations of the namespace's other definitions (G1187).
  *Done 2026-11-16 (0.3.0 slice).* The `RATE_LIMIT_MISMATCH` warning (and `CONCURRENCY_LIMIT_MISMATCH`, the same mechanism, part of T022-025): each published version writes its explicit groups to `flow_control_group_declarations` (migration `20270130000400`, immutable, classified), and publication and validation compare with the active versions of the namespace's other workflows (D022-041, G1187 closed). Evidence: `integration_flow_control_step_policies::publication::a_group_declared_elsewhere_with_another_limit_is_warned_and_nothing_else_is` (a different limit and a different window warned, naming the workflow; same values, another level, a derived group and another tenant's namespace not; validation reports it at `/concurrency`), red first for a real defect (`PgRunStore` delegated nothing and silenced it; the store method now has no default); hand mutations caught: window ignored, level ignored, tenant and namespace filter removed; `publication::the_warning_reads_this_namespace_and_the_active_versions_only` (reviews of 2026-11-16: another namespace of the tenant and an inactive version do not count; red with the namespace filter removed and with the active-version join removed); `migrations::the_quota_and_declaration_migrations_roll_back_only_once_their_rows_are_drained` (a declaration is never updated or deleted, `23001`).
- [x] T022-070 [US7] Additive migration for `rate_limit_windows`, `rate_limit_grants`, `rate_limit_waiters` (`data-model.md`) with `sagat_app` grants, a refusing down migration and entries in `storage-classification.json`; the digest key rotation refusal of T022-007 extended to them. FR-022-040. Evidence: up/down test; runtime-privilege sweep; manifest test.
  *Done 2026-11-15 (0.3.0 slice).* Migration `20270130000100_flow_control_rate_limits` (windows, grants, waiters; counters, `max_period_ms` and `grants_total` per life of the row (D022-039) by triggers; a grant's lease instant written once; the rotation refusal extended to windows), its entries in `storage-classification.json`. Evidence: as T022-009 and T022-010.
- [ ] T022-071 [US7] The pure window decision in `crates/domain/src/execution/flow_control.rs`: which grants count for an acquirer (run scope from the grant, step scope from the lease or while unleased), admit or wait, position and estimate, as a function of instants passed in. FR-022-040,048; D022-003,005,014. Evidence: property tests that no sequence of decisions puts more than `limit` counted instants in any window, for mixed acquirer limits and windows; clock-backwards cases.
- [x] T022-072 [US7] Grant or buffer at admission (T19) through the one admission-gate port, for every run creator on the tree; permit and slot together when both are declared; queue-full and key-error outcomes; start delay evaluated at release. `crates/application/src/{rate_limits.rs,concurrency.rs,runs.rs}`, `crates/infra-postgres/src/{rate_limits.rs,execution.rs}`. FR-022-041,042,044,047. Evidence: integration tests for API, event, cron and child starts; `RATE_LIMIT_QUEUE_FULL`; a run never commits with a permit and no slot or the reverse.
  *Partly done 2026-11-20 (0.3.0 slice), not ticked (decisions D022-064 and D022-065).* A start that declares
  `rateLimit` is granted or buffered in its admission transaction by the one decision of both levels and both gates
  (`gates_of`, `decide_filed`, `start_may_wait` in `crates/infra-postgres/src/execution/readiness_gate.rs`); a run's slot
  stores `leased_at = granted_at`; permit and slot together or neither, one deadline; refusals rate first by the key that
  is full. Evidence: `integration_flow_control_step_policies::rate_gate::{a_rate_limited_start_is_granted_its_slot_from_its_grant_and_the_next_waits_until_the_window_ends, a_buffered_start_is_granted_once_the_window_of_the_slot_ahead_has_passed, a_dual_gated_start_takes_its_permit_and_slot_together_or_waits_for_both, a_start_beyond_a_rate_ceiling_is_refused_rate_first_and_nothing_of_it_exists, a_buffered_start_past_its_deadline_fails_rate_limit_wait_timeout}`
  and `admission::publication_and_starts_wait_for_the_activation_and_a_serving_process_admits_a_rate_limited_run`; the
  unit test `readiness_gate::tests::a_refused_rate_limited_start_is_refused_rate_first_and_counted_by_each_family`.
  Open: the event, cron and child starts of a rate-limited definition, a delayed start's release, a dual-gated start
  granted from the queue and its timeout (correctness review, minor 6); the reset of a rate-limited start is refused
  `BINARY_CAPABILITY_MISSING` until task T022-086 (gap G022-016).
  *Done 2026-11-20 (0.3.0 slice).* `rate_creators::{an_event_trigger_admits_a_rate_limited_start_and_retries_an_event_the_key_refuses,
  a_cron_trigger_admits_a_rate_limited_start, a_schedule_action_admits_a_rate_limited_start,
  a_child_run_of_a_rate_limited_definition_is_buffered_and_granted_from_the_queue,
  a_delayed_rate_limited_start_is_decided_at_its_release,
  a_dual_gated_start_is_granted_both_from_the_queue_or_times_out_rate_first}` (event, cron, schedule and child
  starts, a delayed start's release with room, behind a holder and with no key, a dual-gated start granted both from
  the queue and timed out rate first), each checked against its rows and the consistency check. These pin built
  behaviour; the refusal `RATE_LIMIT_QUEUE_FULL` of an event's run is seen and its redelivery admitted. The reset of a
  rate-limited start stays task T022-086 (gap G022-016). `integration_flow_control_step_policies` 185 passed (184 in the full run; the one failure, `rate_gate::the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed`, whose stand-in window met the new empty-window row, fixed and rerun green) as the owner
  and 185 likewise as `sagat_app`.
  *Proved 2026-11-20 by hand mutation* (the tests above were written after the code). One mutation per rule, each applied
  alone, seen red, then every file restored and the binary built fresh, all 6 tests green again:
  mutation 1, the event dispatcher starts an event's run without its gates: the event test red (no run starts: the domain's
  `refuse_gated_start` refuses an ungated start of a gated definition, so the creator's bypass creates nothing);
  mutation 2, the same for a cron tick: the cron test red (no run); mutation 3, a start the key refuses consumes its event
  (`Started::No`): the event test red (`DISPATCHED`, not `PENDING`); mutation 4, a schedule's action starts without its
  gates: the schedule test red (the firing fails); mutation 5, a child starts without its gates: the child test red (no child
  run); mutation 6, a released start delay is never decided (`start_undecided` gives no `Release`): the delay test red
  (no start released); mutation 7, a key that does not evaluate at the release is taken as the group's single key: the delay
  test red (`Running`, not `Failed`); mutation 8, the queue's grant of a run gives only its first gate: the dual-gated test
  red (nothing granted); mutation 9, a run's timeout names the concurrency gate first: the dual-gated test red
  (`CONCURRENCY_WAIT_TIMEOUT`, not `RATE_LIMIT_WAIT_TIMEOUT`).
  - *Amended 2026-11-13 after review* (D022-033, D022-024): invariants 8–10 land in this task's change; per-key run waiters default 100; the child-start outcome is T022-102.
- [ ] T022-073 [US7] Grant pass (T20) and queue timeout (T21) in `crates/scheduler/src/flow_control.rs` and `crates/infra-postgres/src/rate_limits.rs`: shard fence, oldest admitted waiter first with skip-ahead, bounded grants per key and tenant rotation per pass, first commit wins against the timer. FR-022-041,042,047; ADR-0088. Evidence: admission-order grants; a fence rejects a stale replica; a waiter of another key and tenant is granted during a flood within two passes.
  - *Amended 2026-11-13 after review* (D022-021, D022-022): superseded by T022-097 (grant) and T022-098 (timeout), which serve both gates.
- [x] T022-074 [US7] Step slots: ask at readiness (T22) for the first attempt and each retry, step `WAITING` (`RATE_LIMIT`) until granted, lease recording and slot return by the pass (T23), step queue timeout through the failure route. `crates/infra-postgres/src/{rate_limits.rs,execution.rs}`, `crates/scheduler/src/flow_control.rs`; `crates/infra-postgres/src/workers.rs` is not edited. FR-022-040,043; D022-005,008. Evidence: no attempt spent while waiting; with no worker for three windows at most `limit` grants are unleased; a returning worker leases at most `limit` in its first window (SC-022-021).
  *Done 2026-11-20 (0.3.0 slice; decisions D022-064 and D022-065).* The lease pass (transaction T23,
  `Scheduler::rate_leases` before the grants of each tick, `MaintenanceStore::record_rate_leases`) copies the granted
  attempt's `step.leased` instant journaled after the grant onto the grant row, locking the grant rows only, then lowers
  each recorded window's hint; the rest rule, the hint after a grant and the purge (transaction T25, housekeeping) as the
  decisions say; `workers.rs` is not edited. Evidence: `rate_gate::{a_step_slot_counts_from_the_lease_the_pass_copies_from_the_journal, without_workers_at_most_limit_slots_wait_unleased_and_a_returning_worker_leases_at_most_limit, an_unleased_slot_rests_its_key_until_the_pass_start_plus_the_window, a_waiter_with_a_shorter_window_is_woken_when_the_grant_ages_out_for_it, a_grant_through_another_keys_pass_wakes_the_waiters_of_its_window, the_purge_keeps_a_grant_for_the_longest_period_and_a_minute_and_not_a_microsecond_more, the_purge_keeps_a_short_grant_for_the_longest_period_its_window_has_seen}`;
  the readiness and retry cases of a rate-limited step are the readiness gate's (task T022-096). Hand mutations caught:
  the lease instant taken from the grant (the lease test red), the rest over the grant's own window (the shorter-window
  test red), the hint after a grant removed (the other key's pass test red), the purge boundary made inclusive (the
  boundary test red). Reviews in `reviews/2026-11-20/rate-gate-*.md`. Suites on 2026-11-20:
  `integration_flow_control_step_policies` 143 passed as the owner and 143 with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
  `integration_claim_transition_efficiency` 22 passed in both roles; `contract_flow_control_step_policies` 10 passed.
  - *Amended 2026-11-13 after review* (D022-020, D022-028, D022-029): the readiness ask is T022-096 for every source; the grant names attempt `attempt() + 1` and lease recording accepts only that attempt's `step.leased` after the grant, locking the grant row only; returns are T022-100.
- [ ] T022-075 [US7] Lifecycle (T24): cancellation and run deadline of a waiter, pause skips a waiter, a lowered declaration and a lowered policy ceiling, a granted step cancelled before its lease, node-level `concurrency` with node-level `rateLimit`, a body `child` node starting a rate-limited workflow. FR-022-044,046. Evidence: one integration test per item (SC-022-022).
  *Evidence 2026-11-25 (0.3.0 slice), not ticked.* One integration test per item: the run deadline of a waiter
  `rate_lifecycle::a_run_deadline_dequeues_a_buffered_start`; pause skips a waiter
  `rate_lifecycle::a_paused_waiter_is_skipped_and_its_resume_lets_the_pass_grant_it` (hand mutation caught: the pass's
  run predicate always true, red with `queue::paused_heads_do_not_hold_back_the_waiters_behind_them_and_a_resume_rearms_the_key`;
  dropping only `NOT paused` is equivalent, a pause stores the run `PAUSED` as well, decision D022-068 item 7); a lowered
  declaration `rate_lifecycle::a_lowered_declaration_revokes_nothing_and_applies_to_the_next_start`. Proved where they
  were built (named in the module text of `rate_lifecycle.rs`): the cancellation of a waiter
  (`fault_flow_control_step_policies::rate_limits::a_grant_and_a_cancellation_of_a_buffered_start_are_decided_by_the_first_commit`,
  `readiness::a_cancelled_run_takes_its_queued_step_out_of_every_queue`), a lowered policy ceiling
  (`rate_limits::a_lowered_policy_ceiling_slows_later_grants_and_revokes_none`), a granted step cancelled before its
  lease (`returns::a_cancellation_gives_back_a_granted_unleased_step`), node-level `concurrency` with node-level
  `rateLimit` (`queue::a_dual_gated_head_is_not_starved_by_a_younger_waiter`,
  `queue::a_dual_gated_waiter_times_out_on_the_gate_that_refuses_it_rate_first`), a body `child` node (refused at
  publication on this tree: `child_starts::a_child_node_inside_a_collection_body_is_refused_at_publication`; a root one
  is `rate_creators::a_child_run_of_a_rate_limited_definition_is_buffered_and_granted_from_the_queue`).
- [x] T022-076 [US7] Hot-path budget: the budget of `tests/integration/claim-transition-efficiency/work_budget.rs` unchanged for a definition without the new fields; a new line for the claim of a rate-limited step equal to the plain claim; recorded lines for a completion whose next step declares `rateLimit` and for an admission that declares it. NFR-022-007; FR-038-001. Evidence: `integration_claim_transition_efficiency` passes; seen failing first with a statement added to the claim by hand. Lands before any call site in a completion.
  - *Amended 2026-11-13 after review* (D022-030): moved to Phase 2, after T022-091 and before T022-022.
  *Not done 2026-11-15.* Its lines measure the claim, completion and admission of a definition that declares `rateLimit`; in this build no such run can be admitted (D022-037), so there is nothing to measure. T022-091 holds the plain-admission line meanwhile, and the selector covers the files that can move it (T022-092). Lands with T022-096, before its call site.
  - *Follow-up 2026-11-16:* this task lands in the change that builds T022-096 (the readiness gate), with its lines measured before that change's call site in a completion; T022-096 is not ticked without it.
  *Done 2026-11-19 (0.3.0 slice), with T022-096.* `tests/integration/claim-transition-efficiency/work_budget.rs::a_rate_limited_step_costs_what_the_budget_says` (a chain whose middle step declares `rateLimit`, through `support::flow_control::gated_world`): the claim of the rate-limited step equals the plain `LEASE` (38, 2, 5); `COMPLETION_WITH_A_RATE_LIMITED_NEXT_STEP` = (58, 1, 13) against the plain (50, 1, 11); `RATE_LIMITED_ENTRY_ADMISSION` (an admission whose entry declares `rateLimit`; a run-level `rateLimit` is not admitted by this build, D022-037) = (41, 1, 13) against `PLAIN_ADMISSION` (33, 1, 11); the completion of the rate-limited step and the one that ends the run are the plain lines. The plain lines are unchanged: `LEASE` (38, 2, 5), `COMPLETION_WITH_A_NEXT_STEP` (50, 1, 11), `COMPLETION_THAT_ENDS_THE_RUN` (47, 1, 9), `PLAIN_ADMISSION` (33, 1, 11) before and after the change. Evidence: `integration_claim_transition_efficiency` 19 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`; seen failing with a `SELECT 1` added to the lease of a gated step by hand (the line it measures).
- [x] T022-077 [US7] Ceilings of the plan's rate-limit table in `crates/domain/src/quota.rs`, `crates/application/src/quotas.rs`, `crates/infra-postgres/src/quotas.rs`; buffered runs in `active_runs`, `tenant_active_runs` and `queued_runs`. FR-022-047; FR-022-034. Evidence: an at-bound and an over-bound test per ceiling; zero refuses.
  *Done 2026-11-20 (0.3.0 slice; decision D022-064).* Four quota resources (`rate_limit_live_keys`,
  `rate_limit_per_second`, `rate_limit_per_minute`, `rate_limit_per_hour`), migrations `20270206000000` and
  `20270206000100`, the canonical OpenAPI's two enums and the Studio client (`npm run api && npm run check:api`: matches);
  applied at readiness and at admission. Evidence: `cargo test -p sagat-domain --test flow_control_quota` 13 passed (each
  resource at and over its bound, zero refuses; hand mutations caught: the minute's ceiling read as the second's, the
  live rate keys judged with `>`); `rate_gate::{a_start_beyond_a_rate_ceiling_is_refused_rate_first_and_nothing_of_it_exists, the_namespace_ceiling_of_a_window_length_clamps_a_declared_rate_limit}`
  (hand mutation caught: the declared limit used unclamped). Buffered runs count in `queued_runs` (one statement with the
  concurrency waiters) and in `active_runs` as every created run.
  *Re-review 2026-11-20 (decision D022-066 item 1; architecture Blocker 1 of the re-review):* `rate_limit_live_keys`
  counts window rows again, and a decision that lacks room reclaims its own namespace's purgeable windows first.
  Evidence: `rate_gate::an_aged_window_counts_against_the_live_keys_ceiling_until_it_goes` (red with the live-window
  count restored by hand), `rate_gate::a_new_key_at_the_ceiling_reclaims_a_window_only_purgeable_grants_keep` (red with
  the reclaim returning 0), `rate_gate::the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed`
  (the owner's bound, OD-2026-11-20-11; red with the live count read as 0); `integration_flow_control_step_policies`
  148 passed as the owner and as `sagat_app`; `security_multitenancy::quotas::the_tenant_view_has_no_single_usage_for_a_per_namespace_resource`
  lists the four rate resources as per namespace.
- [x] T022-078 [US7] `Run.rateLimit`, the step's waiting state and `GET /namespaces/{namespaceId}/rate-limit-keys` in `crates/api-http/src/{dto_flow_control.rs,routes/rate_limits.rs}`; position and estimate computed at read. FR-022-048. Evidence: REST tests; the estimate is absent in the cases of D022-014; raw-key canary absent.
  - *Built 2026-11-20 (0.3.0 slice; decision D022-067), not ticked:* `Run.rateLimit` (getRun and listRuns; position
    and estimate in getRun only, the estimate withheld from a caller without the reveal grant), `waitingOn` `rate_limit`
    and `concurrency`, `GET /namespaces/{namespaceId}/rate-limit-keys`, migrations `20270207000000` and `20270207000100`,
    the canonical OpenAPI, the API reference and the Studio client. Evidence so far: `rate_rest::*` 5 passed as the owner
    and as `sagat_app`, each red under its hand mutation (the estimate a microsecond early, room giving no estimate, the
    rate cursor tagged as the concurrency one, the minute truncation of `nextSlotAt` dropped, the `waitingOn` mapping
    dropped, the reveal check of the estimate dropped); `integration_flow_control_step_policies` 153 passed in both roles.
    Open: the Majors of `reviews/2026-11-20/rate-reads-responses.md`.
  - *Done 2026-11-20 (0.3.0 slice; decisions D022-067 and D022-068).* Every Blocker and Major of the reviews and
    re-reviews `reviews/2026-11-20/rate-reads-*.md` is answered in `rate-reads-responses.md` (0 open). Evidence:
    `rate_rest::*` 14 tests (the k-th lease, the waiters the pass skips, the deadline rule, both gates, `waitingOn`
    `rate_limit` and `concurrency`, the place read past its timeout, the page cost by `EXPLAIN`, a key of two windows,
    one group's pages, a forced version's cursor, bodies held to the canonical schemas); unit tests
    `flow_control_keys::tests` 8 passed (each edge of `k`, one microsecond either side of the deadline, the listings'
    tags both ways); `consistency::{a_windows_oldest_wait_that_is_not_its_waiters_is_found,
    one_statement_that_adds_and_removes_waiters_of_a_window_leaves_its_oldest_wait}`; each red under its hand
    mutation, the table in `rate-reads-responses.md`. `integration_flow_control_step_policies` 185 passed (184 in the full run; the one failure, `rate_gate::the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed`, whose stand-in window met the new empty-window row, fixed and rerun green) as the owner and
    185 likewise as `sagat_app`; `runtime_privileges::` 47 passed as `sagat_app`; `check-architecture.py` (the rate route under
    the key-value-free rule) passed.
- [x] T022-079 [US7] Invariants 8–10 of `data-model.md` in `verify_run` (`crates/infra-postgres/src/execution.rs`) and `assert_consistent` (`tests/support/engine.rs`), with the journal window check. FR-022-040,045. Evidence: fails on a seeded extra grant and on a waiter without its event; passes after every fault case.
  - *Amended 2026-11-13 after review* (D022-032, D022-033): invariant 10 per grant over `(t_e − period_e, t_e]`; the `verify_run` checks land with T022-072 and T022-096, this task keeps `assert_consistent`.
  - *Partly done 2026-11-20 (0.3.0 slice), not ticked.* `verify_run` checks invariants 8 to 10 for a run's start
    (`verify_start_rate`); `rate_gate::verify_run_refuses_a_start_whose_grant_row_is_gone_while_it_still_counts` is the
    seeded breach. Open: the step-scope rows, `grants_total`, and `assert_consistent` across runs (architecture review,
    Major 3).
  - *Done 2026-11-20 (0.3.0 slice).* `support::flow_control::key_consistency`, run by `World::assert_consistent` after
    every test of the suite, now checks the rate tables across runs: a window's counters against its rows, no empty
    window, every waiter row against its run's or step's projection (`QUEUED`, attempt, pool, group, label, deadline)
    and the reverse, every grant row against its `rate_limit.granted` (run, node, attempt, key, instant), every run's
    slot that still counts against its grant row, `grants_total` between its rows and the key's journaled grants, and a
    window's oldest wait; the runs these rows name are checked by `verify_run`. Evidence: `consistency::*` (seven new
    seeded breaches, among them "a seeded extra grant" and "a waiter without its event", each found by its own row);
    `integration_flow_control_step_policies` 185 passed (184 in the full run; the one failure, `rate_gate::the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed`, whose stand-in window met the new empty-window row, fixed and rerun green) as the owner and 185 likewise as `sagat_app` (every test passes the
    check).
  - *Production gate (2026-11-20, D022-066 item 5):* `docs/runbooks/flow-control.md` section 6 forbids activating the rate capability in production before this task is ticked.
- [ ] T022-080 [P] [US7] Fault and race suite `tests/fault/flow-control-step-policies/rate_limits.rs`: grant versus timeout and grant versus cancel (20 rounds each, barriers, no sleeps), a crash at each commit point of T19–T24, a duplicate grant across a shard takeover, a lost hint, a restart after several windows, a flood of 1,000 starts on one key, the limit lowered while runs wait, replicas with offset process clocks, the database clock set back. SC-022-017,018,022,023; NFR-022-001,008. Evidence: results recorded as ADR-0088 evidence.
  - *Amended 2026-11-13 after review* (D022-027): the set-back clock uses `libfaketime` inside the PostgreSQL test container; the grant instant is `clock_timestamp()` after the window lock.
  - *Amended 2026-11-21 (D022-072):* the set-back clock is tested by moving the counted grant instants one window ahead, not with `libfaketime`.
  - *Amended 2026-11-24 (decisions D022-076 and D022-078):* the answers to the re-review of this task (`reviews/2026-11-24/rate-faults-rereview-correctness-reviewer.md`) are built with it: the hint a waiter queued with room writes, the hint of a full window of mixed lengths, a reset decided by the reservation rule, the order bound within a pass, the place and the run view on the effective limit, the weak tests made to fail, and the waiter's declaration (`pinned_limit`) bounding it under the current ceiling. Each is evidence of this task; the timing-dependent cases of this suite are driven by the database clock or by moved counted instants.
  *Built 2026-11-21, not ticked:* `tests/fault/flow-control-step-policies/rate_limits.rs`, 15 cases (crashes in T19,
  T20, T21, T22, T23 at both commits and T24; grant against timeout and grant against cancel, 20 rounds each; takeover;
  lost hint; restart window after window; a flood of 1,000 starts on one key across a restart; a lowered policy
  ceiling; replicas with process clocks ten minutes apart; the database clock set back). The lowered-ceiling case was
  red first for the right reason (two grants 114 ms apart against a lowered ceiling of 1 per second) and found a defect:
  the grant from the buffer did not apply the namespace's current ceiling (D022-071, fixed in `gate_queue.rs`; the
  concurrency twin is gap G022-020). Hand mutations caught: no deadline check under the locks (grant against timeout,
  round 3), the waiters ahead not counted at admission (the flood), the window measured from the decision's instant
  (the set-back clock). Not caught, as the review found: `AND g.leased_at <= now` in the count (the set-back test never
  decides while the moved grant is in the future). The correctness review
  (`reviews/2026-11-21/rate-faults-correctness-reviewer.md`) found 0 Blockers and 6 Majors, open: rounds 0 and 1 of the
  timeout race do not reach the grant; the set-back case; the place estimate and the run view still use the filed limit;
  the waiters-ahead rule over mixed periods; the flood's admission order on a slow runner (a question for the
  coordinator: strict order or the reservation rule); crashes only before the first write of T20, T21 and T24.
  Runs on this tree: `fault_flow_control_step_policies` 23 passed as the owner and as the runtime role;
  `integration_flow_control_step_policies` 191 as the owner and 191 as the runtime role (2 lost to the Docker disk,
  `No space left on device` at migration, rerun alone and green); `integration_claim_transition_efficiency` 22 in both
  roles; `integration_security_multitenancy` 126 as the owner and, as the runtime role, 112 plus the 14 lost to the
  same disk, rerun with their modules (`secret_pools::`, `secrets::`, 49 passed).
  *Evidence 2026-11-25 (residuals of the batch of 2026-11-24), not ticked.* Decision D022-077 item 1 at its call site:
  `rate_limits::a_grant_pass_visits_at_most_256_keys_whatever_its_batch` (a batch of 300, 257 keys due: one pass
  visits 256 and leaves one due; red with the cap removed from `grant_pass_keys` by hand).
  `rate_limits::a_grant_that_ages_out_during_a_pass_never_puts_a_younger_waiter_first` no longer depends on real elapsed
  time (decision D022-072): the holders' counted instants are moved ahead while the waiters are admitted and while the
  pass selects, and set to one window before the database clock read once the pass is blocked, so they count as of the
  selection and not at the decision; the windows then age on the database clock before the next pass only so that the
  journals hold no two grants in one window (a wait that decides nothing). Red with the count taken at the decision's
  own instant (`least` turned into `greatest` in `gate_queue.rs`) by hand.
- [x] T022-081 [P] [US7] Lock-order barriers `tests/integration/flow-control-step-policies/rate_limit_locks.rs` for each path of ADR-0088 decision 4 (admission, grant, timeout, readiness, lease recording, cancellation, purge), alone and with a semaphore. FR-022-044; SC-022-019. Evidence: no deadlock; the reversed-order control waits as expected.
  *Done 2026-11-20 (0.3.0 slice).* `tests/integration/flow-control-step-policies/rate_limit_locks.rs`, against references
  in the order of the slots from every start (`support::lock_order::against_at`, the new slot `RateWindows`, 8b):
  admission (counter row, semaphore, window), queue timeout and cancellation (run, counter row, semaphore, window),
  readiness (run, window), each alone and with a permit; the grant pass passes a held run by (`SKIP LOCKED`) and, with
  a permit, lets a reference of the semaphore then the windows through; lease recording and the purge complete while
  the run is held (they take no run). The control `a_plan_that_takes_a_window_before_its_run_deadlocks_with_the_reference`
  deadlocks with one victim. 7 passed; `integration_flow_control_step_policies` 185 passed (184 in the full run; the one failure, `rate_gate::the_rate_live_keys_count_misses_only_the_keys_other_transactions_have_not_committed`, whose stand-in window met the new empty-window row, fixed and rerun green) as the owner and 185 likewise as
  `sagat_app`.
  *Proved 2026-11-20 by hand mutation* (the tests were written after the code). One mutation per rule, each applied alone,
  seen red, then every file restored and the binary built fresh, all 7 tests green again: mutation 1, an admission files its
  window before its semaphore: the admission test red (a deadlock with the reference, the path its victim); mutation 2, an
  admission locks the windows before its active-run counter row: the admission test red (deadlock); mutation 3, a commit
  that ends the run gives its gates back before it locks its counter row: the timeout and the cancellation tests red
  (deadlock); mutation 4, the returns lock the windows before the semaphores: both red with a permit (deadlock); mutation 5,
  the grant pass waits for a held run instead of passing it by (`Lock::Wait` for `SKIP LOCKED`): the grant test red (the pass
  did not finish within 30 seconds); mutation 6, the grant locks the window before the semaphore: the grant test red
  (deadlock, nothing granted); mutation 7, a completion locks the windows before its run: the readiness test red
  (deadlock); mutation 8, lease recording locks the runs of its grants: the lease and purge test red (not finished within 30
  seconds while the run is held); mutation 9, the purge locks the runs of its grants: the same test red at the purge;
  mutation 10, the control plan taken in the slots' order: the control red (no deadlock).
  - *Amended 2026-11-13 after review* (D022-026): with the `flow_control_counts` rows last.
  - *Production gate (2026-11-20, D022-066 item 5):* `docs/runbooks/flow-control.md` section 6 forbids activating the rate capability in production before this task is ticked.
- [ ] T022-082 [P] [US7] Metrics and spans of the plan in `crates/telemetry/src/{metrics.rs,span.rs}`; alerts `rate_limit_grant_lagging` and `rate_limit_queue_saturated` in `crates/scheduler/src/monitor.rs`. FR-022-049. Evidence: telemetry tests reject unknown labels; each alert raised and cleared in a test.
  - *Amended 2026-11-24 (decision D022-077 items 4, 6 and 7):* this task also builds `CONCURRENCY_QUEUE_STUCK` (the runbook documents three alerts) and the alert `FLOW_CONTROL_WAITER_ORPHANED` (the code logs and counts `outcome="orphan"` only, gap G022-005), with the runbook's thresholds: `CONCURRENCY_QUEUE_STUCK` when a key has a waiter older than `SAGAT_ALERT_UNCLAIMED_SECS` (300) and no grant in that time; `RATE_LIMIT_GRANT_LAGGING` when a key is due for longer than `SAGAT_ALERT_LEASE_GRACE_SECS` (60) with no grant in that time; `RATE_LIMIT_QUEUE_SATURATED` at 80 % of the effective `flow_control_key_waiters` of run starts.
  - *Built in part 2026-11-19 (G022-001, D022-045), not ticked:* the step-scope decisions, grants, timeouts, dequeues and returns are counted after commit in the closed vocabulary, and the four spans name the group, never the key. Evidence: `cargo test -p sagat-infra-postgres --lib readiness_gate::tests::each_committed_gate_outcome_is_counted_in_the_closed_vocabulary` (mutation of the from-queue outcome caught). Open: the wait histograms, waiter gauges, grant lag, unleased grants and the two alerts.
- [ ] T022-083 [P] [US7] `rateLimit` builder in the six SDKs (`sdks/typescript/src/workflow.ts`, `sdks/go/workflow/workflow.go`, `sdks/python/sagat_octopus_orchestrator/workflow.py`, `sdks/dotnet/src/SagatOctopusOrchestrator.Sdk/Workflows/Flow.cs`, `sdks/java/sdk/src/main/java/dev/sagatoctopusorchestrator/sdk/workflow/WorkflowBuilder.java`, `sdks/rust`). FR-022-050. Evidence: unit tests and each SDK's lints; canonical fixture byte-identical in six languages.
- [ ] T022-084 [US7] Scenario `flow-rate-limit` of `contracts/conformance.md` in `tests/contract/sdk-conformance/scenarios/` with independent assertions and reviewed goldens. SC-022-023. Evidence: `SAGAT_SDK_SCENARIO=flow-rate-limit cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture` passes in six languages, rerun without the bless switch.
- [ ] T022-085 [US7] Studio: `rateLimit` fields beside `concurrency` in `frontend/apps/studio/src/features/workflows/` (settings and node inspector), a run or step waiting for a rate slot with position and estimate in `features/runs/`, the rate-limit keys list in `features/operations/`; unknown capability shown as unsupported (FR-007-010); WCAG AA. FR-022-048,050. Evidence: Vitest; a browser journey including keyboard; same IR as the SDKs.
- [x] T022-086 [US7] Run-creator cases of T022-064 extended to the rate gate, failing when a creator bypasses the port; with 018 on the tree, its creators, the reset refusal `RATE_LIMIT_SLOT_UNAVAILABLE` and the reset rules of the five `rate_limit.*` events registered in 018's registry (G1181). FR-022-041,046; FR-022-038. Evidence: `tests/integration/flow-control-step-policies/run_creators.rs`; the 018 cases reported pending while its branch is not merged.
  - *Amended 2026-11-21 (D022-069 item 7):* this task is the reset; the rate twins of the other creators of T022-064 are T022-113.
  - *Amended 2026-11-13 after review* (D022-031): the schedule-firing case is built now; G1181 corrected.
  - *Production gate (2026-11-20, D022-066 item 5):* `docs/runbooks/flow-control.md` section 6 forbids activating the rate capability in production before this task is ticked; gap G022-016 is a release gate, and the interim refusal is pinned by `run_creators::a_reset_of_a_rate_limited_start_is_refused_until_its_rules_are_built`.
  *Built 2026-11-21 (decision D022-069; closes gap G022-016), not ticked until its independent review:* the five
  `rate_limit.*` reset rules of `contracts/journal-events.md` (`crates/domain/src/execution/reset.rs`), a restored gate
  event of either family decoded with its marker (`event_codec.rs`), a reset's fresh slot or `RATE_LIMIT_SLOT_UNAVAILABLE`
  rate first (`readiness_gate::reset_refused`), the reason in the API (409 `CONFLICT`), the batch item (`CONFLICT`), the
  pending reset's `FAILURE_REASONS` and migration `20270209000000`/`000100` (the reset register's reason list, `NOT VALID`
  then validated; a down that refuses while an entry carries it); `verify_run`'s invariant 10 leaves out a grant a reset
  copied with its prefix. Evidence: `run_creators::{the_reset_rules_of_the_rate_limit_events_are_the_contracts,
  a_reset_takes_a_fresh_slot_right_after_run_reset, a_reset_without_a_free_slot_is_refused_rate_first_and_never_waits,
  a_pending_reset_takes_a_fresh_slot_or_fails_rate_limit_slot_unavailable}` and
  `migrations::the_reset_slot_reason_rolls_back_only_once_no_entry_carries_it`, each seen red first for the right reason
  (the rules `EXCLUDE`, no point after the grant, the reason refused by the old list; the first green run found that
  invariant 10 counted the restored grant twice, fixed in `verify_run`); `cargo test -p sagat-domain --test reset` 21
  passed; `cargo test -p sagat-infra-postgres --lib event_codec` (red first: a restored rate event did not decode) and
  `readiness_gate::tests::a_refused_reset_is_named_by_its_first_refusing_gate_and_counted_by_each_declared_family`;
  `cargo test -p sagat-api-http --test flow_control_errors` 11 passed. Hand mutations caught: the last refusing gate
  named instead of the first (rate first lost), a concurrency refusal counted for a rate-only reset, the API answering
  `UNAVAILABLE`, the pending reset's reason mapped to `ADMISSION_REFUSED`, the down migration without its guard.
  *Ticked 2026-11-21 after the re-reviews* (`reviews/2026-11-21/reset-slot-rereview-*.md`: security, correctness and
  architecture each 0 Blockers, 0 Majors; answers in `reset-slot-responses.md`, section "Re-review"; closes gap G022-016;
  the runbook's production gate no longer lists this task). Added with the re-review: `limit: 3` in
  `run_creators::a_reset_after_a_rate_limited_step_restores_its_slot_as_history` (correctness Major 2 of the first
  review) and `run_creators::a_reset_whose_prefix_returned_a_step_slot_restores_it_as_history`, written after the code and
  proved by hand mutation: one grant more counted in a step window (`readiness_gate::key_count`) turns both red at "only
  `b`'s fresh slot", and with the old `limit: 4` the first stays green (the slack the reviewer found); the codec refusing a
  restored `rate_limit.returned` turns the new case red at load and leaves the other green; fresh build after restoring.
  Evidence on the tree of this tick: `cargo test -p sagat-tests --test integration_flow_control_step_policies` 191 passed
  as the owner and 191 passed as the runtime role (`SAGAT_TEST_AS_RUNTIME_ROLE=1`); `npm run api` then
  `npm run check:api` ("src/api/schema.d.ts matches the OpenAPI contract"), `vitest run src/features/runs/ResetRun.test.tsx`
  49 passed; every `scripts/check-*.py` passes (`check-docs-first.py` and `check-plain-words.py` with
  `--base origin/develop`). The privilege audit (`integration_platform_foundation`, `runtime_privileges`) was not rerun:
  no SQL changed since `8dcd7f37` (its migration changes a CHECK list only, no grant and no function), and the host disk
  was under the 15 GiB floor for another build.
- [ ] T022-087 [US7] Merge the rate-limit parts of the fragments into `specs/001-platform-foundation/contracts/{openapi.yaml,workflow.schema.json}` and regenerate the Studio client; the OpenAPI baseline is changed only after reading the diff. FR-022-039,048. Evidence: contract lint and compatibility tests; `(cd frontend/apps/studio && npm run api && npm run check:api)`.
- [ ] T022-088 [P] [US7] Rate-limit sections of `docs/runbooks/flow-control.md` (the two alerts, a stuck or saturated key, the clock, lowering a limit) and `docs/runbooks/alerts.md`; `docs/architecture/execution-semantics.md`; `docs/traceability.md`; the release notes of 0.3.0; the quickstart's section 7 run as a demo of the happy and the failure scenario. FR-022-049; NFR-022-005. Evidence: reviewed docs; `contract_developer_experience` without the bless switch.
- [ ] T022-089 [P] [US7] Benchmark `tests/performance/flow-control-step-policies/` for rate limits: grant lag at 1, 16 and 256 waiters on one key, one key with 10,000 waiters, 1,000 keys with one waiter each, admission cost with and without the declaration. NFR-022-004; G1174, G1178. Evidence: report with environment, raw data and failures; no figure presented as a commitment.
  - *Amended 2026-11-13 after review* (D022-021; SC-022-029): three engines, `limit: 100, per: second`, saturated buffer, floor of 90 grants a second for 60 seconds; the per-second hard ceiling of 100 is published only when the floor is met.
- [ ] T022-090 [P] [US7] `GET .../rate-limit-keys` and the new run fields in `tests/contract/security-multitenancy/rest/battery.rs`; raw-key canaries in logs, spans, metrics and listings. SC-022-023; NFR-022-002. Evidence: battery and canaries pass, also as the runtime role.

### Order of the slice

- T022-066 first. Then T022-002, 003; then T022-004..010, 012..014 (narrowed) and T022-068..070.
- US02: T022-021 → 022 → 023 → 024 → 025..027, with T022-028 and T022-029 once 023 and 024 exist.
- US07 after T022-022 (it extends the same port): T022-071 → 072 → 073 → 076 → 074 → 075, 077..079, with
  T022-080 and T022-081 once 073 and 074 exist. T022-076 precedes 074's call site in a completion.
- Surfaces after the story they show: T022-048..052, 067, 083 after the codec; T022-053, 084 after the
  stories; T022-054..056, 085 after T022-026 and 078; T022-060, 087 before T022-065.
- Parallel: T022-067 and 083 per language; T022-082, 088, 089, 090.

### Phase 11: follow-up tasks of the review of 2026-11-13 (T022-091..107)

Added for the findings recorded in `reviews/2026-11-13/` (D022-019..034). The slice now has 80 tasks.

- [x] T022-091 Budget line for a plain admission (no gate declared) in `tests/integration/claim-transition-efficiency/work_budget.rs`, in Phase 2 before T022-022. NFR-022-007; D022-030. Evidence: `integration_claim_transition_efficiency` passes with the new line; seen failing with a statement added to `create` by hand.
  *Done 2026-11-15.* `PLAIN_ADMISSION = (33 statements, 1 transaction, 11 rows written)` in `work_budget.rs`. Evidence: `integration_claim_transition_efficiency` passes (owner and runtime role); seen failing with a `SELECT 1` added to `PgRunStore::create` by hand, and unchanged with the gate block disabled (the gate adds nothing to a plain admission).
- [x] T022-092 [P] Extend the budget rule of `scripts/select-test-targets.py` to `crates/scheduler/`, `crates/domain/src/execution/` and `crates/application/src/{concurrency,rate_limits,flow_control}.rs`, with a fixture in `scripts/tests/test_select_test_targets.py`. NFR-022-007; D022-030. Evidence: `python3 -m pytest scripts/tests/test_select_test_targets.py` passes; the fixture fails before the change.
  *Done 2026-11-15.* The budget rule also covers `crates/scheduler/`, `crates/domain/src/execution/`, `crates/domain/src/workflow{.rs,/}` and `crates/application/src/{concurrency,rate_limits,flow_control,keyed_digest}`. Evidence: `python3 -m unittest scripts.tests.test_select_test_targets` — 56 OK; the new fixtures failed (7) before the rule changed.
- [x] T022-093 The gate inside `PgRunStore::create`; an assertion in both inserts of `crates/infra-postgres/src/chain_progress.rs` that the definition declares no run-level gate; a rule in `scripts/check-architecture.py` that `INSERT INTO runs` appears only in `execution.rs` and `chain_progress.rs`. FR-022-059; D022-031. Evidence: the script's test in `scripts/tests/test_check_architecture.py` with a negative fixture; an integration test that a gated definition reaching a successor is refused.
  *Done 2026-11-15.* `PgRunStore::create` refuses a gated definition before any admission lock (`flow_control_admission`); both continuation inserts call `refuse_gated_successor` (any gate); `check-architecture.py` refuses an `INSERT INTO runs` outside `execution.rs` and `chain_progress.rs` and one not preceded by its guard in the same function. Evidence: `python3 -m unittest scripts.tests.test_check_architecture` — 40 OK, with negative fixtures (mutation allowing every file: 2 failures); `integration_flow_control_step_policies::admission::a_gated_definition_that_reaches_a_continuation_successor_is_refused` (run-level and node-level tamper, then the same hand-off advances; red first for the node-level case).
- [x] T022-094 Instance capabilities `flow-control-concurrency-v1` and `flow-control-rate-v1` registered by control-plane and engine binaries through 018's registry; `FLOW_CONTROL_ACTIVATION_PENDING` until activation; the activation command. Prerequisite: T018-080 (pull request 98) on `develop`. FR-022-053; SC-022-027; D022-023. Evidence: with one live engine lacking a capability, activation is refused and publication and start are refused `FLOW_CONTROL_ACTIVATION_PENDING`; after activation a process without it answers `503`/`BINARY_CAPABILITY_MISSING`.
  *Done 2026-11-15.* Prerequisite verified: T018-080 is on `develop` (`91977bfb`, pull request 98; migration `20270113000000_instance_registry`). `Capability::{FlowControlConcurrencyV1, FlowControlRateV1}` and migration `20270130000200_flow_control_capabilities`; 018's rule 5 lists them; no process of this build lists them (D022-037); `control-plane flow-control-activate`, whose step refuses an empty fleet. Evidence: `integration_flow_control_step_policies::admission` — activation refused while a live engine lacks the capability and recorded once every live process lists it; publication refused `FLOW_CONTROL_ACTIVATION_PENDING`, start refused `BINARY_CAPABILITY_MISSING` after activation, `ACTIVATION_PENDING` after removal, `CAPABILITY_DISABLED` with the switch off, no run created; an empty fleet refused (mutation caught); `instances::tests::a_process_of_this_build_lacks_a_required_flow_control_capability` (a process without it is closed by its serving gate, which answers `503`/`BINARY_CAPABILITY_MISSING`); `integration_schedules_bulk_operations -- instances` 60 passed (code, schema and data model list the same values).
- [x] T022-095 `flow_control_key_versions` row read in each deciding transaction; per-key rotation (both digests during a rotation, existing row wins, retirement when unused). FR-022-053, FR-022-056; D022-025. Evidence: rotation with live holders, waiters and grants never splits a key; the old version retires when its last row goes.
  - *Follow-up 2026-11-16:* Owner-gate: ADR-0093 (proposed) and O1170 in `research.md`. The per-key rotation supersedes the last sentence of ADR-0056 decision 7, which the owner accepted; the task is not ticked until the owner accepts ADR-0093 and answers O1170.
  *Built 2026-11-16 (0.3.0 slice; D022-042, D022-043), not ticked (owner gate above).* Migration `20270130000500_flow_control_key_rotation` (activation with live rows: the old version `RETIRING`, out of force the moment its last row goes and recorded `RETIRED` by the next activation; at most one `RETIRING`; `ROTATION_IN_PROGRESS` for a third version; `activated_role`; `flow_control_key_versions_in_force()` for the runtime role; the versions the earlier command left `RETIRING` retired; a down that refuses during a rotation and restores the command of `20270130000100`), `KeyRing::candidates` (`crates/application/src/keyed_digest.rs`) and `key_versions` / `file_keys` (`crates/infra-postgres/src/flow_control.rs`; a retiring row that vanishes before its lock is the retryable `KEY_ROW_VANISHED`), the one way a deciding transaction reads the versions in force and files its keys (a rule of `scripts/check-architecture.py`). Evidence: `cargo test -p sagat-application --lib keyed_digest` 5 passed; `integration_flow_control_step_policies::migrations::{a_rotation_with_live_holders_waiters_and_grants_never_splits_a_key, a_rotation_runs_with_live_rows_and_retires_the_old_version_with_its_last_row, a_retiring_row_that_goes_away_while_it_is_filed_is_a_retryable_refusal, the_rotation_migration_retires_what_the_old_command_left_and_its_down_refuses_during_a_rotation}` (as the runtime role; races with held locks and `pg_blocking_pids`, no sleeps), owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`; hand mutations caught: the retiring row ignored, a third version allowed, the retiring version in force while unused, a vanished row filed anyway; `python3 -m unittest scripts.tests.test_check_architecture` 44 OK (the new rule's test red when the rule allows every file).
  *Done 2026-11-19 (0.3.0 slice).* Owner-gate: OD-2026-11-19-2 (O1170 (a) with (b) for incidents; ADR-0093 Accepted, the operator's force is T022-108). Evidence rerun 2026-11-19 on the tree of this change: `cargo test -p sagat-application --lib keyed_digest` 5 passed; `integration_flow_control_step_policies` 54 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`, among them the four `migrations::` tests named above; `python3 -m unittest scripts.tests.test_check_architecture` OK. The task's "never splits a key" holds for the per-key rotation; after a force a key has at most two rows (T022-108, ADR-0093 decision 5).
- [x] T022-096 Readiness gate: the reducer maps `step.released`, `step.retry_due`, `step.retry_requested` of a gated node to `WAITING` (`crates/domain/src/execution/{state.rs,state_effects.rs,flow_control.rs}`); the adapter at `persist` (`crates/infra-postgres/src/execution.rs`) locks the keys once and journals grant or queued; no task-ready hint for a waiting step; invariant 11 in `verify_run`. FR-022-051; SC-022-025; D022-020. Evidence: one test per readiness source (worker completion, retry pass, timer, signal, approval, update, finished child, operator retry, admission, resume); a seeded breach fails `verify_run`.
  *Built 2026-11-19 (0.3.0 slice; D022-044), not ticked: three of the ten sources have no test yet, and the placement of the decision diverges from ADR-0089 decision 2 (ADR-0094 proposed, O1171, owner gate).* Reducer: `RunState::with_gated_steps`, the three events of a gated node to `WAITING` (`state.rs`, `state_effects.rs`), `READY` only on the grant that completes every declared gate, a lease without them refused, `RunState::flow_control_breach` (invariant 11), `Evaluation::Gates` and `Resolution::Gates` (`plan.rs`, `command.rs`), a stopped run's queued step gates dequeued (rate first) before its steps are cancelled. Application: `Engine::decide_gates` (keys evaluated, `FLOW_CONTROL_KEY_INVALID` through the failure route), `RunTx::decide_step_gates` (refuses by default). Adapter: `crates/infra-postgres/src/execution/readiness_gate.rs` (key versions, `file_keys` semaphores then windows, counts under the locks, holder/grant or waiter rows, the gates' projection), `persist` (returns and dequeues first, no hint for a waiting step), `verify_run` (the journal alone agrees; invariant 11); migrations `20270130000600_flow_control_step_gates` and `20270130000700_flow_control_validate`. Evidence: `cargo test -p sagat-domain --test readiness_gate` 11 passed (hand mutations caught: the released mapping, the retry-due mapping, the all-gates grant, the planner's ask, the answer check, the stop dequeue); `integration_flow_control_step_policies::readiness` 10 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`, one test per source for the run's start, a worker completion, the retry pass, a timer, a signal, a finished child and a resume, plus a key that does not evaluate, a cancellation of a queued step, a release and a decision on one key in one commit (granted at `limit: 1`), the sorted lock order against a held key row (a barrier on `pg_stat_activity`, no sleep) and a seeded `READY` step without a grant that `verify_run` refuses (hand mutations caught: a hint for a waiting step, a gate that always admits, the returns skipped, the returns left out of the round's lock: both new tests red). Reviews of 2026-11-19 in `reviews/2026-11-18/`, their fixes in D022-044. Not yet: tests for an approval, an Update and an operator's retry as sources; the collection- and loop-finish paths refuse a gated successor (G1197); the keys reach no deployment (G1198); the namespace ceilings and the live-keys bound are not applied at readiness (G1199); no metric or span is recorded (G022-001).
  *Evidence added 2026-11-19 (0.3.0 slice), still not ticked: the owner gate of ADR-0094 (O1171) stands.* The three sources without a test: `integration_flow_control_step_policies::readiness_sources::{an_approved_request_releases_its_gated_route_into_its_gates, an_operators_retry_decides_the_next_attempt, an_applied_update_leaves_a_queued_gated_step_as_it_was}` (an approval's route granted, then queued behind the holder; an operator's retry queued while the key is held and granted while it is free; an Update is no readiness source on this tree: it journals `state.updated` only and leaves a queued step exactly as it was). 3 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`; hand mutations caught: the retry-requested mapping to `READY` (the retry test red), the released mapping off for `g` (the approval test red); the Update test has no code to mutate (the Update path writes no step) and guards a future one. With them every source of the task's list has a test. G1199 and G022-001 are closed in part by D022-045 (under T022-101 and T022-082).
  *Done 2026-11-19 (0.3.0 slice).* Owner-gate: OD-2026-11-19-3 (O1171 (a); ADR-0094 Accepted). Evidence rerun 2026-11-19 on the tree of this change: `cargo test -p sagat-domain --test readiness_gate` 17 passed; `integration_flow_control_step_policies` 54 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1` (`readiness::` 10 tests, one per source of the run's start, a worker completion, the retry pass, a timer, a signal, a finished child and a resume, and the seeded `READY` step `verify_run` refuses; `readiness_sources::` 3 tests, an approval, an operator's retry and an Update). What the task lists as not yet built stays with its own items: G1197 (collection- and loop-finish successors), G1198 (the key ring in the deployments), G022-001 (the rest of the telemetry, T022-082).
- [x] T022-097 One grant routine (T26) for runs and steps and both gates: key-driven pass with `key_shard`, `next_eligible_at`, the partial index and `crates/scheduler/src/fairness.rs`; the reservation rule; skip past-deadline waiters; orphaned waiter alert. Replaces the grant parts of T022-023 and T022-073. FR-022-044, FR-022-052; SC-022-017, SC-022-026; D022-021. Evidence: dual-gated head not starved; a saturated second key granted at its limit; many keys and tenants beyond one pass's budget each granted within two passes; fence rejects a stale replica.
  *Done 2026-11-19 (0.3.0 slice; D022-045), for step keys, resting on the readiness gate of ADR-0094 (proposed, O1171 pending).* `Scheduler::gate_grants` (keys due on held home shards, fair between tenants, `batch` keys and 256 waiters a key a pass), `RunTx::grant_queued_step` / `crates/infra-postgres/src/execution/gate_queue.rs` (run, key-shard fence `holds_key_shard`, sorted key locks, the reservation rule, both or neither, never past the deadline), `execution::grant_step_gates`, the grant's task hint in `persist`, the hint kept by queue, release, return, dequeue, pause and resume and rested only by the key's shard holder. Evidence: `cargo test -p sagat-domain --test readiness_gate` 17 passed (the grant cases; mutation of the readiness check caught); `integration_flow_control_step_policies::queue::{a_released_permit_is_granted_to_the_oldest_waiter_by_the_next_pass, a_saturated_second_key_is_granted_at_its_limit, a_dual_gated_head_is_not_starved_by_a_younger_waiter, many_keys_and_tenants_beyond_one_pass_are_each_granted_within_two_passes, a_stale_replica_is_refused_by_the_key_fence, paused_heads_do_not_hold_back_the_waiters_behind_them_and_a_resume_rearms_the_key, a_waiter_queued_behind_an_ineligible_one_is_granted_without_a_release}`, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`; hand mutations caught: the reservation's waiters ahead ignored (the dual-gated head red), the key fence always held (the stale replica red), the grant's task hint removed (the oldest-waiter test red); the last two tests red before the fixes of the reviews of 2026-11-19 (`reviews/2026-11-19/`). Not in it: run waiters (no run is admitted gated, T022-022, T022-072); a rate key frees by time only once a grant's lease instant is recorded (T022-074, G022-003); the orphan alert (G022-005).
  - *Amended 2026-11-21 (decision D022-073, strict admission order):* the case `queue::a_waiter_queued_behind_an_ineligible_one_is_granted_without_a_release` (cited above and by task T022-023) is now `queue::a_step_behind_only_an_ineligible_waiter_is_granted_at_once`: an older waiter refused by its own lower limit is no longer counted at admission, so the younger step is granted at once instead of queued for the next pass. The hint of a waiter queued with room is proved by `rate_gate::a_start_behind_an_eligible_waiter_queues_although_the_window_has_room_and_the_next_pass_grants_it`.
- [x] T022-098 One timeout routine (T27): one deadline per waiter; the refusing gate's `wait_timed_out` (rate first) and the other's `dequeued`. Replaces the timeout parts of T022-023 and T022-073. FR-022-052; SC-022-026; D022-022. Evidence: each precedence case; races against T26 decided by first commit over 20 rounds.
  *Done 2026-11-19 (0.3.0 slice; D022-045), for step waiters, resting on the readiness gate of ADR-0094 (proposed, O1171 pending).* `Scheduler::gate_timeouts` (due waiters on held run shards, under the run's fence), `RunTx::refusing_gate` (rate first; read without the keys' locks so a gated failure route keeps the one lock order), `execution::time_out_step_gates` (`wait_timed_out` of the refusing gate, `dequeued` of the other, `step.failed` with `CONCURRENCY_WAIT_TIMEOUT` or `RATE_LIMIT_WAIT_TIMEOUT` through the route, no attempt spent), the waiter rows given back by the round or by `persist`. Evidence: `cargo test -p sagat-domain --test readiness_gate` (the timeout cases; mutations caught: the deadline check, the refusing gate first); `integration_flow_control_step_policies::queue::{a_waiter_past_its_deadline_fails_through_its_route, a_dual_gated_waiter_times_out_on_the_gate_that_refuses_it_rate_first (both refuse, only the permit, only the slot), a_timeout_and_a_grant_of_one_waiter_are_decided_by_the_first_commit (20 rounds, three orders, held key lock and `pg_stat_activity` barriers), a_timeout_and_a_signal_on_one_run_both_land_once_in_either_order, a_timeout_whose_route_is_gated_decides_the_route_in_its_commit}`, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`; hand mutations caught: the grant ignoring the deadline (the race red), the refusing gate ignoring the counts (precedence red). Not in it: run waiters (T022-022, T022-072); `verify_run` does not check a timeout against its journaled deadline (G022-004).
- [x] T022-099 Poll-window test: 64 or more steps of key A of one activity waiting for a permit (and, separately, for a slot) and one step of key B; the next poll leases B; the claim's probe counters show no candidate of A. FR-022-019; SC-022-024; FR-038-002. Evidence: `integration_claim_transition_efficiency` or the flow-control suite passes; seen failing with ADR-0056 decision 3 as written (permit at claim) by hand.
  *Done 2026-11-20 (0.3.0 slice).* `tests/integration/claim-transition-efficiency/gated_window.rs::{steps_waiting_for_a_permit_are_not_candidates_of_the_claim, steps_waiting_for_a_slot_are_not_candidates_of_the_claim}`: 64 steps of key A wait behind its leased holder, one step of key B is ready; the next poll (of 8) leases B, and `worker_claim_candidates_total` moves by one in each stage (fetched, probed, opened, leased). 2 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`. Seen failing by hand with a waiting gated step written `READY` (the nearest thing to ADR-0056 decision 3 this tree can build: the claim no longer refuses a permit-less step): both tests red, at the start, where the projection check refuses the row. A row in `docs/engineering/architecture-principles.md`.
- [x] T022-100 Return slot and permit of a granted, unleased step on pause, review marking, cancellation and run end (T29, `rate_limit.returned`, `concurrency.released`); ask again on resume and retry. FR-022-040, FR-022-046; D022-028. Evidence: one test per path; at no instant more than `limit` granted and unleased steps of a key.
  *Done 2026-11-20 (0.3.0 slice; D022-048). Owner-gate met: OD-2026-11-20-2 ("Aceitar ADR-0096", O1175 (a)); ADR-0096 is Accepted and partly supersedes ADR-0094 decisions 2 and 3. Evidence rerun on 2026-11-20 after the answer: `cargo test -p sagat-domain --test readiness_gate` 20 passed; `integration_flow_control_step_policies returns` 7 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`.* Build: `execution::pause_run` settles the run; the planner and the reducer give back the permit and slot of a granted, unleased root step on pause and make it wait; `RunState::waits_for_resume` (planner, invariant 11, `persist`); a re-decision over a slot the attempt gave back; a pause or resume locks the keys of its returns and of the run's waiters in one sorted pass. Evidence: `cargo test -p sagat-domain --test readiness_gate` 20 passed (`a_pause_returns_the_gates_of_a_granted_unleased_step_and_its_resume_asks_again`, `a_step_granted_while_its_run_is_paused_gives_its_gates_back_in_that_commit`, `a_review_hold_gives_back_and_a_retry_asks_again_for_the_same_attempt`); `integration_flow_control_step_policies::returns` 7 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1` (pause and resume, hold for review then retry, cancellation, run end, the retry pass while paused, a cancellation of a paused run, the pause's sorted lock order with a held semaphore row and a `NOWAIT` probe); invariant 13 checked at each step (holders and unleased rate grants per key). Hand mutations caught: the planner's return on pause, the return on leaving `READY`, the re-decision over `ENDED`, invariant 11's exemption, the planner asking while paused, the rearmed keys left out of the sorted pass. Reviews in `reviews/2026-11-20/returns-*.md`.
- [ ] T022-101 A step key that fails to evaluate or exceeds 256 bytes fails `FLOW_CONTROL_KEY_INVALID`, a live-keys ceiling `FLOW_CONTROL_LIMIT`, through the failure route, no attempt spent; rows in `contracts/errors.md`. FR-022-054; D022-025. Evidence: one test per cause; the step is never granted.
  - *Built in part 2026-11-19 (G1199, D022-045), not ticked:* the live-keys cause: a step key that would be a new semaphore beyond the namespace's `concurrency_live_keys` (or a namespace ceiling of zero) fails `FLOW_CONTROL_LIMIT` through its route, nothing filed; the namespace `concurrency_limit` and `flow_control_queue_timeout_ms` bound a declaration. Evidence: `integration_flow_control_step_policies::ceilings::{a_new_step_key_beyond_the_live_keys_ceiling_fails_flow_control_limit, the_namespace_limit_and_queue_timeout_bound_a_declaration}` (mutations caught: the refusal ignored, the namespace limit and the timeout ignored). Open: the 256-byte cause's own test, the rows of `contracts/errors.md`, the concurrent overshoot (G022-002, O1172).
  *Evidence 2026-11-25 (0.3.0 slice), not ticked.* The 256-byte cause:
  `readiness::a_key_longer_than_256_bytes_fails_the_step_through_its_route` (a canonical value of 257 bytes fails
  `FLOW_CONTROL_KEY_INVALID` through `onError`, attempt 0, no grant, waiter or window row and no `rate_limit.*` event; one
  of exactly 256 bytes is granted). The code was built by T022-096, so the test passed at once; hand mutation caught:
  `>` turned into `>=` in `Engine::decide_gates` (the 256-byte key refused, red). The rows of `contracts/errors.md`
  ("Durable failure codes") were present and are kept. The concurrent overshoot is settled by the owner (O1172,
  G022-002): nothing to build.
- [ ] T022-102 A child node (root and in a collection body) meeting a full buffer, `queued_runs` or the live-keys ceiling fails with its typed retryable code; no child run is created. FR-022-055; D022-024. Evidence: integration test with the retry policy retrying it and the failure route taking it.
  - *Follow-up 2026-11-20 (D022-062, re-review of 39157608 minor 7):* also a child start refused `CAPABILITY_DISABLED` or
    `FLOW_CONTROL_ACTIVATION_PENDING` (a running parent while the switch is off): today the pass fails every tick and the
    parent's node neither completes nor fails; it must fail with its typed code through the route.
  *Evidence 2026-11-25 (0.3.0 slice; decision D022-080), not ticked.*
  `child_starts::{a_child_refused_by_a_full_buffer_or_a_ceiling_fails_its_node_through_the_route,
  a_child_refused_by_its_capability_fails_its_node_through_the_route, a_child_node_inside_a_collection_body_is_refused_at_publication}`:
  `RATE_LIMIT_QUEUE_FULL`, `CONCURRENCY_QUEUE_FULL`, `FLOW_CONTROL_LIMIT` (`queued_runs`, live keys),
  `CAPABILITY_DISABLED` and `FLOW_CONTROL_ACTIVATION_PENDING` each fail the child node through `onError`, no child run,
  the pass reports a finished child and fails nothing, and the next pass finds nothing left. Red before the change: the
  pass failed (`failed: 1`, nothing finished), seen again by hand with the refusal arms disabled. A `child` node has no
  retry policy in IR v1, so the failure route is what handles it (amendment under FR-022-055); a body `child` node is
  refused at publication on this tree. `cargo test -p sagat-application --lib flow_control` covers
  `a_refused_child_fails_with_the_refusals_own_reason`.
- [ ] T022-103 Publication refuses identity-only keys (`FLOW_CONTROL_KEY_UNBOUNDED`); window rows keep `max_period_ms`; the purge keeps grants for that period plus one minute and decrements `live_grants` by the rows it actually deleted. FR-022-056; D022-025. Evidence: codec tests; purge test with mixed periods.
  - *Follow-up 2026-11-16 (from T022-002):* its evidence also adds the invalid fixture document of `FLOW_CONTROL_KEY_UNBOUNDED` to `tests/contract/flow-control-step-policies/fixtures/` and removes the code from `PENDING` in `ir.rs`.
  *Evidence 2026-11-25 (0.3.0 slice; decision D022-081), not ticked.* Codec:
  `contract_flow_control_step_policies::ir::{a_key_that_reads_the_runs_identity_is_refused_as_unbounded,
  every_invalid_fixture_is_refused_with_its_code_before_a_version_exists}` with the fixture
  `invalid-flow-control-key-unbounded.workflow.json`; `FLOW_CONTROL_KEY_UNBOUNDED` left `PENDING` in `ir.rs` (both red
  with the rule disabled by hand). Purge with mixed periods:
  `rate_gate::the_purge_of_a_key_with_mixed_periods_counts_down_the_rows_it_deleted` (the window keeps an hour, a
  per-second grant is kept an hour and a minute, at most `limit` rows a key, `live_grants` down by exactly the rows each
  purge deleted; red with the purge's `LIMIT` removed by hand). The window's `max_period_ms` and the purge's predicate
  were built by T022-074; this task adds the rule and the evidence.
- [ ] T022-104 Key classification: `FLOW_CONTROL_KEY_SECRET` refused, `FLOW_CONTROL_KEY_PII` warned; `keyLabel` `null` for callers who may not disclose every location the key reads. FR-022-057; G1185. Evidence: codec tests over 016 carriers; REST tests with and without disclosure rights.
  - *Follow-up 2026-11-16 (from T022-002):* its evidence also adds the fixture documents of `FLOW_CONTROL_KEY_SECRET` (invalid) and `FLOW_CONTROL_KEY_PII` (warning) and removes both codes from `PENDING` in `ir.rs`.
- [ ] T022-110 Close G022-013: a takeover test that races a real keeper takeover (alpha's short lease run out by the database clock while its grant waits for a held key, `beta.round()` the second lock waiter), and SC-022-004 at run level (five same-key starts with `limit: 1`: at most one holder, grants in admission order; a mixed-limit pair sharing a group). Evidence: `fault_flow_control_step_policies` with hand mutations.
  *Evidence 2026-11-25 (0.3.0 slice), not ticked.*
  `fault_flow_control_step_policies::concurrency::{a_real_takeover_skips_the_shard_of_a_grant_in_flight_and_nothing_is_granted_twice,
  five_same_key_starts_hold_at_most_one_permit_and_are_granted_in_admission_order,
  a_mixed_limit_pair_sharing_a_group_applies_each_limit_to_the_shared_holders}`. The takeover is a real keeper's: alpha's
  one-second leases run out on the database clock while its grant waits for the held key; beta's round takes every shard
  but the fenced one (the acquisition skips a locked shard, so beta does not wait as the gap's text supposed: it cannot
  take the key's shard until the grant commits), its pass grants nothing; after the commit beta takes the last shard and
  grants nothing twice. Hand mutation caught: the pass's permit waiters ordered youngest first (the five-start test red).
  A mutation of the acquisition's `SKIP LOCKED` would hang the takeover test rather than fail it, so it was not run.
- [ ] T022-111 The aggregated key listing for a caller who may not see labels (D022-061, G022-014): one row per (pool,
  scope, group) with holders, waiters and the group's oldest holder and oldest wait, from per-group counter rows kept by
  the triggers that keep the key rows' counters (with T022-105), so a page costs its own groups; per-key rows stay the
  view of callers who may see labels (with T022-104, per location). Release item beside T022-104. FR-022-022,
  FR-022-057. Evidence: REST tests (a viewer reads group rows only, whole instants; a revealer reads per-key rows); a plan
  test that a page reads only its own groups; a canary that no per-key value reaches a viewer.
  - *Amended 2026-11-20 (decision D022-068 item 6):* the two key listings' handlers share one opening, parameterised by
    `KeyListing` (`KeyListingQuery`, `key_cursor_in_force`), when this task reshapes them (architecture minor 3 of the
    review of task T022-078).
- [ ] T022-112 [US7] One text of the grant pass's eligibility predicate (`deadline_at > now()`, the run running or waiting
  and not paused) for its five uses (`execution/gate_queue.rs` twice, `maintenance.rs` twice, `RATE_PLACE_SQL`), as
  `purgeable_grant!` is one text for the purge, and a gate-neutral run permit state (`RunPermitState` documents and
  prints the concurrency gate's meaning for a rate slot). Decision D022-068 item 6 (architecture minors 2 and 4 of the
  review of task T022-078). Evidence: the suites unchanged and green; the five uses expand one macro.
  *Evidence 2026-11-25 (0.3.0 slice), not ticked.* `eligible_gate_waiter!` and `live_gate_run!`
  (`crates/infra-postgres/src/lib.rs`) are the one text of the eligibility for its seven uses (the five of the task and
  the two of admission's eligible waiters, `readiness_gate.rs`) and the place estimate's "skipped";
  `scripts/check-architecture.py` refuses a copy written out (`TheEligibilityOfAWaiterIsOneText`, three cases, red on a
  copy); `gate_predicate_tests::the_eligibility_of_a_waiter_is_one_text`. `RunPermitState` became the gate-neutral
  `RunGateState` (`Held`, `Waiting`), with no words of its own: each body names it (`HOLDING`, `GRANTED`, `WAITING`).
  Suites unchanged and green (gate lines of this change).
- [ ] T022-113 [US7] Rate twins of the run creators of T022-064 that `rate_creators.rs` does not cover: the start under a business identifier, the rerun as new, Signal- and Update-with-Start, the batch rerun, the ad hoc run and its private rerun, each admitted through its key (`rate_limit.granted` with a grant row, or `rate_limit.queued` with a waiter row) and checked by the consistency check. FR-022-041; D022-069 item 7. Evidence: `tests/integration/flow-control-step-policies/rate_creators.rs`.
  *Evidence 2026-11-25 (0.3.0 slice), not ticked.* `rate_creators::{the_start_api_with_a_business_identifier_admits_a_rate_limited_start,
  a_rerun_as_new_of_a_rate_limited_run_admits_through_the_key,
  a_signal_with_start_of_a_rate_limited_definition_takes_its_slot_or_creates_nothing,
  an_update_with_start_of_a_rate_limited_definition_admits_through_the_key,
  a_batch_rerun_of_a_rate_limited_run_admits_through_the_key,
  an_ad_hoc_rate_limited_workflow_and_its_private_rerun_admit_through_the_key}`, each admitted through its key and
  checked by the consistency check (the helpers of `run_creators.rs` shared). The admission was built by T022-072, so
  they passed at once; hand mutation caught: `Engine::start` treating a gated start as ungated (every twin red).
- [ ] T022-114 [US7] A `CANCEL_SOURCE` reset of a rate-limited source whose key's window is full when the request arrives: an advisory refusal `RATE_LIMIT_SLOT_UNAVAILABLE` at the request, or a warning in the reset preview (D022-069 item 3; reviews of 2026-11-21, architecture Major 1, security minor). Evidence: an integration case with `limit: 1, per: hour`.
  *Evidence 2026-11-25 (0.3.0 slice; decision D022-079: the advisory refusal at the request), not ticked.*
  `run_creators::a_cancel_source_reset_on_a_full_window_is_refused_at_its_request_and_cancels_nothing` (`limit: 1, per:
  hour`: refused `RATE_LIMIT_SLOT_UNAVAILABLE`, the source still running and not stopping, no entry, no run; a source
  with room accepted); `run_creators::a_pending_reset_takes_a_fresh_slot_or_fails_rate_limit_slot_unavailable` now fills
  the window after the request, so the completion's refusal stays proved. Red with the refusal disabled by hand. The
  contracts (`resetRun` of 001's OpenAPI, the reset of 018's REST fragment) and both runbooks say so; the Studio client
  matches (`npm run api && npm run check:api`).
- [ ] T022-115 [US7] The second clause of SC-022-017: a second key whose own buffer is saturated at the same time, its grants coming from the pass, is served at its own limit; and with more keys and tenants holding waiters than one pass visits, every eligible key of another tenant is granted within two passes (review of task T022-080, minor 5; decision D022-075 item 7). Evidence: a fault case in `tests/fault/flow-control-step-policies/rate_limits.rs`.
  *Evidence 2026-11-25 (0.3.0 slice), not ticked.*
  `fault_flow_control_step_policies::rate_limits::a_second_saturated_key_is_served_at_its_own_limit_and_every_tenant_within_two_passes`:
  `sat-a` (`limit: 1`) and `sat-b` (`limit: 2`) saturated together, one pass grants one and two, `sat-b`'s third waits;
  six keys of two tenants behind a pass of four keys: four in the first pass (the second tenant served), two in the
  second. Each start queues at a full window whatever the runner's speed (the holder's instants moved ahead while it is
  admitted, decision D022-072).
- [ ] T022-105 `flow_control_counts` slot rows with `SECURITY DEFINER` triggers and `SELECT`-only grants; foreign keys `(tenant_id, namespace_id, run_id)`; column-limited `UPDATE` grants on semaphores and windows (`next_eligible_at`, `updated_at`). FR-022-058; D022-026. Evidence: runtime-privilege sweep; a direct `UPDATE` of a counter as `sagat_app` is refused; a dual-gated run counts once.
  - *Built in part 2026-11-20 (0.3.0 slice; D022-051), not ticked:* the exactness of `queued_runs` that the owner made a merge condition: proved exact under the namespace's admission lock by `run_admission::two_starts_at_the_queued_runs_ceiling_are_decided_one_after_the_other` (held lock, both starts observed waiting, exactly one queued and one refused `FLOW_CONTROL_LIMIT`), owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1`; hand mutations caught: the admission lock removed, the ceiling ignored. Open: the `flow_control_counts` slot rows and their triggers, the foreign keys, the column-limited grants.
- [ ] T022-106 A grant from the buffer does not re-authorize: a buffered run whose initiator lost `run:start` is granted, and its journal and audit keep the original initiator. FR-022-060; D022-034. Evidence: integration test.
  *Evidence 2026-11-25 (0.3.0 slice), not ticked.*
  `rate_gate::a_grant_from_the_buffer_does_not_reauthorize_and_keeps_the_initiator`: a developer's run buffered for both
  gates is granted after she is demoted to viewer (her own start then refused `Forbidden` `run:start`); `runs.started_by`,
  the actor of `run.started` and the one `run.started` audit record stay hers. The grant path has no authorization to
  mutate, so the test guards a future one.
- [x] T022-107 Owner gate for ADR-0089 and O1169: the owner's answer recorded in `docs/owner-decisions.md` and `research.md`; ADR status set by a person. Owner-gate: T022-024, T022-096, T022-097, T022-098, T022-099. Evidence: the recorded answer.
  *Done 2026-11-15.* Evidence: OD-2026-11-13-19 in `docs/owner-decisions.md` on `develop` ("Aceitar o ADR-0089",
  added by `69dc08e2`); the status line of ADR-0089 reads `Accepted (2026-11-13, OD-2026-11-13-19)`, set by
  `20789a40`. Accepting ADR-0089 is option (a) of O1169, so node-level `concurrency` ships under ADR-0089; the answer
  is recorded under O1169 in `research.md` by this change.

### Phase 12: follow-up tasks of the owner decisions of 2026-11-19 (T022-108..)

- [x] T022-108 The operator's force of ADR-0093 decision 5: `flow_control_key_force_retire` retires a `RETIRING` version at once, audited (append-only history: operator, database role, reason, rows left), one forced version draining at a time (`FORCED_VERSION_DRAINING`); `flow_control_key_version_runs` for the operator; the live-keys ceiling counts the versions in force only; runbook `docs/runbooks/flow-control.md` sections 1 to 4. FR-022-053, FR-022-056; D022-046, D022-047. Owner-gate: OD-2026-11-19-2. Evidence: a key over-admitted by at most what its old row holds, with held locks; the force waits for a decision in flight.
  *Done 2026-11-19 (0.3.0 slice).* Migration `20270202000000_flow_control_key_force` (up and down; the down refuses while a force is recorded), `refused_by_ceilings` (`crates/infra-postgres/src/execution/readiness_gate.rs`). Evidence: `integration_flow_control_step_policies::{rotation::a_forced_retirement_over_admits_a_key_by_at_most_its_old_rows_holders_until_they_drain, rotation::a_key_filed_again_after_a_force_is_not_refused_by_the_live_keys_ceiling, migrations::the_force_is_the_operators_audited_and_retires_only_a_retiring_version}` 3 passed, owner and `SAGAT_TEST_AS_RUNTIME_ROLE=1` (the force against a decider holding the versions, a barrier on `pg_stat_activity`; a contender on the new row's held lock; each row within its own limit and no waiter added to the old one, semaphores and windows); red before the build (no function), and red before each review fix (a padded reason, a control character in the operator's name, a second force while the first drains, a key filed again refused `FLOW_CONTROL_LIMIT`); hand mutations caught: the force without the version table's lock, the forced version left `RETIRING`, the draining guard off. Reviews in `reviews/2026-11-20/`. Open: O1173, O1174; G022-006 (no API, CLI or telemetry for the digest-key commands); rate windows drain only with T022-074 (G022-003).

- [ ] T022-109 Signal-with-Start of a definition whose start queues: hold the signal with the queued run and deliver it
  in the grant's transaction (D022-050 item 6, G022-010): a durable held signal (classified, payload by digest), its
  delivery by the grant routine as `signalRun` would deliver it, the receipt a retry of the request replays, and the
  answer's field that says it is held. FR-022-018; FR-018-024. Evidence: integration test (queued, granted, delivered once;
  a retry answers the same; a timeout or cancellation of the queued run drops the held signal with its audit).

### Order of the slice, revised (2026-11-13)

- Prerequisite: 018's instance-capability registry (T018-080, pull request 98) merged into `develop`; then
  T022-094. T022-107 before the tasks it gates.
- Phase 2 gains T022-091, T022-092, T022-093 and T022-076 (moved), before T022-022; then T022-095.
- US02 and US07 share T022-096 (readiness gate), T022-097 (grant) and T022-098 (timeout); T022-022 and
  T022-072 each land their invariants (D022-033); T022-099 and T022-100 after T022-096.
- T022-101..106 after the codec (T022-069) and the gate (T022-096).
