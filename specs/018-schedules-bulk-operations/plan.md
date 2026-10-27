# Implementation Plan: Schedules and bulk run operations

**Branch**: `feature/018-schedules-bulk-operations` | **Date**: 2026-10-26 | **Spec**: [spec.md](spec.md)

**Input**: Feature specification from `specs/018-schedules-bulk-operations/spec.md`

Status: Proposed — not started.

## Summary

Add four capabilities on top of the existing run engine, each as a short, keyed, one-transaction
admission that never holds a second run's lock and never reopens a run:

1. **Schedules** — a namespace resource with immutable revisions, fired by a new sharded scheduler
   pass that checks the schedule's shard fence and admits the run in the same transaction, keyed by
   (schedule, nominal time). Existing cron triggers become trigger-owned schedules with the 004 key.
2. **Business identifiers and start options** — a namespace-unique binding claimed in the run's
   creation transaction, conflict and reuse policies, and a start delay released by a fenced pass.
3. **Signal-with-Start / Update-with-Start** — the binding lock serializes "signal or start";
   a new run's signal is delivered after its initial reduction in the same transaction or the whole
   request rolls back (no buffer).
4. **Batch jobs and reset** — a snapshot of items applied one run per transaction at a durable rate;
   reset re-emits an eligible journal prefix into a new run with provenance and optional reapply.

The rules a later change could not reverse without migrating data are proposed in
[ADR-0050](../../docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md).

## Technical Context

**Language/Version**: Rust (pinned stable toolchain, `rust-toolchain.toml`); TypeScript (Studio);
the five SDK languages at their pinned versions.

**Primary Dependencies**: Existing only: Tokio, Axum, SQLx, `time`, `croner` and `chrono-tz` (already
in `crates/infra-ir/src/cron.rs`), `sha2`. No new crate dependency is planned.

**Storage**: PostgreSQL (authority). New tables listed in [data-model.md](data-model.md); additive
nullable columns on `runs`. NATS carries only the existing task hints.

**Testing**: `sagat-tests` targets over real PostgreSQL (`tests/Cargo.toml`); fault injection per
statement; the shared SDK conformance suite (`tests/contract/sdk-conformance`); Studio unit and real
browser tests.

**Target Platform**: Linux containers; control plane and execution engine as today.

**Project Type**: Rust modular monolith + web Studio + five SDKs.

**Performance Goals**: None promised. Benchmark criteria (G1005): schedule firing lag p50/p95/p99 at
1,000 and 10,000 schedules; batch item rate per job and across 16 jobs; business-id claim latency
under 32-way contention; reset transaction time for 1,000 and 20,000 restored events.

**Constraints**: One transaction per transition; database clock; finite ceilings; no new
`Error.code`; no worker-protocol change; one optional additive IR field (`reapplyOnReset`); additive
REST and AsyncAPI; the run-permit invariant of ADR-0056 (022) and the search-attribute fold of 020.

**Scale/Scope**: 7 user stories, 52 FR, 6 NFR, 17 SC, 80 tasks.

## Constitution Check

Design checks, not implementation approval. Re-evaluated after the Phase 1 design below.

| Principle | Before research | After design | Evidence / future gate |
| --- | --- | --- | --- |
| I — durable atomic transitions, terminal never reopens, first commit wins | Pass | Pass | Transaction table below; one key per (schedule, nominal time), (job, run), reset request; reset and rerun create new runs; T018-020, T018-046, T018-055, T018-076 fault suites. |
| II — pure domain, inward dependencies, no empty mappers | Pass | Pass | Spec, policy, decision and eligibility types in `crates/domain`; calendar math behind an application port in `crates/infra-ir`; DTOs only where wire semantics differ; T018-003, T018-075 architecture check. |
| III — at-least-once effects, honest compensation | Pass | Pass | No terminate; `CANCEL_OTHER`/`CANCEL_SOURCE`/batch cancel use ordinary cancellation; reset re-execution acknowledged, new effect keys; T018-030, T018-047, T018-057. |
| IV — pinned versions, compatible contracts, five SDKs | Pass | Pass | Schedules pin or resolve the active version per action; worker protocol unchanged; one capability-gated additive IR field; REST/AsyncAPI additive; trigger-owned schedules keep the 004 identity; T018-010, T018-060..068. |
| V — tenant boundary, secrets, fail-closed authority | Pass | Pass | Tenant/namespace on every row and key; schedule authority rechecked per action; batch authority per item; opaque `NOT_FOUND`; 022 test runs refused; T018-021, T018-048, T018-070, T018-077. |
| VI — OTel, audit, typed errors | Pass | Pass | Spans/metrics/audit tables below; reasons in `contracts/errors.md`; T018-071. |
| VII — inbox/outbox, redeliverable hints | Pass | Pass | Every run admission writes its outbox in the causing transaction; passes find due work by index scan, not by broker message; lost-hint tests T018-020, T018-046. |
| VIII — failure tests, honest results | Pass | Pass | Failure matrix below maps each rule-9 case to a task; ceilings are design values pending T018-072. |

## Project Structure

### Documentation (this feature)

`spec.md`, `research.md`, `plan.md`, `data-model.md`, `quickstart.md`, `tasks.md`,
`checklists/requirements.md`, `contracts/` (`rest.openapi.yaml`, `schedule-spec.schema.json`,
`journal-and-events.md`, `errors.md`, `sdk-conformance.md`) and ADR-0050.

### Source placement

| Layer | Path | Adds |
| --- | --- | --- |
| Domain | `crates/domain/src/schedules.rs` (new) | Spec, calendar/interval/exclusion types with bounds, policies, `Schedule`, `ScheduleAction`, outcome enum, pure `decide(state, due, open_runs, now)` and deterministic jitter. No time-zone math. |
| Domain | `crates/domain/src/business_ids.rs` (new) | `BusinessId` value object, conflict/reuse policies, pure `admit(binding, holder_status, now)`. |
| Domain | `crates/domain/src/batch.rs` (new) | Selector, action set, item outcome, job state machine, rate slot arithmetic. |
| Domain | `crates/domain/src/execution/reset.rs` (new) | Eligibility over a replayed `RunState` plus suffix facts; `ResetPlan`; `run.reset`, `run.start_delayed`, `run.start_released` events in `execution/event.rs`. |
| Domain | `crates/domain/src/identity.rs`, `audit.rs`, `quota.rs` | Four permissions, audit actions, quota resources (additive). |
| Domain / IR | `crates/domain/src/workflow.rs`, `crates/infra-ir/src/` | Optional `reapplyOnReset` on Update declarations, capability-gated (ADR-0032). |
| Application | `crates/application/src/schedules.rs` (new) | `ScheduleService` (API use cases), `ScheduleFiring` (pass use case), ports `ScheduleStore`, `ScheduleCalendar`. |
| Application | `crates/application/src/runs/business_id.rs`, `runs/with_start.rs`, `runs/reset.rs` (new) | Start options, Signal/Update-with-Start, reset preview/execute; `NewRun` gains optional `business_id`, `schedule`, `delay`, `reset` admissions (precedent `NewRun.retained`). |
| Application | `crates/application/src/batch.rs` (new) | `BatchService`, `BatchExecutor`, port `BatchStore`. |
| Adapter | `crates/infra-ir/src/schedule_calendar.rs` (new) | `ScheduleCalendar` over `croner` + `chrono-tz`, sharing parsing with `cron.rs`. |
| Adapter | `crates/infra-postgres/src/{schedules,business_ids,batch,resets}.rs` (new), `execution.rs`, `dispatch.rs` | Stores, fences, the creation-transaction hooks, journal re-emission, migrations. |
| Scheduler | `crates/scheduler/src/schedules.rs`, `batches.rs` (new), `passes.rs`, `events.rs` | Schedule pass, backfill rounds, obligation executor; batch selection/item executor; delayed-start release and pending-reset passes; cron pass switch-off flag. |
| API | `crates/api-http/src/routes/{schedules,batch,resets}.rs` (new), `routes/execution.rs` | Routes of `contracts/rest.openapi.yaml`; start options and with-start on the existing run routes. |
| Apps | `apps/execution-engine` | Wire the new passes and settings; `apps/cli`: `sagat schedule …`, `sagat batch …`, `sagat workflow reset …`. |
| Studio | `frontend/apps/studio/src/features/schedules/`, `features/batch/` (new), `features/runs/` | Pages and dialogs of FR-018-047. |
| SDKs | `sdks/{typescript,go,python,dotnet,java}` | Client operations of `contracts/sdk-conformance.md`. |
| Tests | `tests/{integration,fault,contract,e2e}/schedules-bulk-operations/` (new), `tests/contract/sdk-conformance/scenarios/` | Targets registered in `tests/Cargo.toml`. |

No new crate is created: every new concern belongs to an existing bounded context (the domain and
application crates, the PostgreSQL adapter, the scheduler). `scripts/check-architecture.py` keeps
the dependency rules.

## Transaction boundaries

Every transaction starts with `BeginSafely::begin_safely` (ADR-0014) and reads time with the database
clock. "Admission" below is the existing `RunStore::create` path: tree and active-run admission,
managed-database admission (017), payload retention, run insert, initial reduction (journal, steps,
tasks, timers, subscriptions, outbox, audit).

| # | Operation | Locks, in order | Commits together | On refusal or crash |
| --- | --- | --- | --- | --- |
| 1 | Schedule create / update / delete / pause / resume (API) | namespace schedule-count quota fence → schedule row `FOR UPDATE` (update/delete/pause/resume) | revision row, head and state, `authorizedBy`, recomputed `next_wake_at`, audit, idempotency record | Nothing persists; `REVISION_CONFLICT` on stale revision. |
| 2 | Scheduled action that starts a run (pass) | shard fence `FOR SHARE` (owner, generation, expiry vs DB clock) → schedule row `FOR UPDATE` + state-version CAS → business-id row (if templated) → admission, whose membership step checks `authorizedBy` in feature 017's order (secret/catalog → membership `FOR SHARE`; membership alone without managed steps) and whose 022 step acquires or enqueues the run permit (semaphore after the other entity locks, ADR-0056) | action row (unique tenant, schedule, origin `SPEC`, nominal time), schedule state (next nominal, counters, remaining, buffer, tracked runs), run and its initial reduction (or its queued state), run idempotency record `schedule:{id}`/`nominal-{ms}`, audit `run.started` | Fence or CAS failure: no write, counted. Admission refusal (quota, 022 queue full): rollback, then transaction 4. Crash: nothing, the time is still due. |
| 3 | Scheduled decision without a run (skip, buffer, drop, coalesce, missed summary, authority revoked) | fence → schedule row (→ membership `FOR SHARE` for the authority check) | action rows, schedule state; for `AUTHORITY_REVOKED` also state `PAUSED`, system note, audit `schedule.auto_paused` | As 2. |
| 4 | Owed action after an admission refusal | fence → schedule row | `retry_at` (bounded backoff 5 s doubling to 5 min), refusal counter; nominal time unchanged | A retry beyond the catch-up window becomes `MISSED_CATCHUP_WINDOW` in transaction 3. |
| 5 | `CANCEL_OTHER` obligation | in transaction 3: obligation rows inserted; executor (the schedule pass of the schedule's shard): existing run lock (ordinary cancel use case, chain before run when chained) → schedule-shard fence `FOR SHARE` → obligation row `FOR UPDATE` | run cancel request (journal, steps, outbox, audit `run.cancel_requested` by `system:schedule:<id>`), obligation `DONE`, key `schedule:{id}:cancel:{nominal}:{run}` | Stale fence: rollback, retried by the new owner. Replay of the key is a no-op; a terminal target is recorded `DONE` without change. |
| 6 | Buffered start | as 2, after reading the tracked runs' status without locks | as 2 | A stale "open" read only delays (D1005). |
| 7 | Trigger-now (API) | receipt → schedule row `FOR UPDATE` (no shard fence: an HTTP command does not invent a scheduler lease) → as 2 | as 2 with origin `MANUAL`, nominal time `clock_timestamp()` (µs), run key `manual-{µs}`, plus the API idempotency record | Nothing persists. A trigger-owned schedule before the handover: refused. |
| 8 | Backfill create (API) / backfill step (pass) | schedule row → backfill row; step: fence → schedule row → backfill row → as 2 or 3 | one nominal time per transaction plus the backfill cursor and counters | Crash resumes from the committed cursor; `ALREADY_ACTED` for existing keys. |
| 9 | Start with business id, rerun or reset with `KEEP` | idempotency receipt → business-id row (insert or `FOR UPDATE`) → holder status read → ADR-0039 source `KEY SHARE` (rerun/reset) → admission (incl. 022 permit) | binding (holder, bound at), run, audit, idempotency record | Conflict or reuse refusal: rollback, typed 409; 022 queue full: 429. |
| 10 | Signal-with-Start | receipt → business-id row → existing holder: chain → run `FOR UPDATE`, status rechecked under lock; new: admission (incl. 022 permit) | either the signal effects on the holder, or the new run, its initial reduction and `satisfy_wait` for each waiting step; binding, audits `run.started`/`run.signalled`, idempotency | No waiting step (including a new run 022 queued): full rollback, 409. |
| 11 | Update-with-Start | as 10 with the declared Update evaluated by the bounded evaluator | start (if any), state revision, receipt, journal `state.updated`, audit `run.updated`, outbox | Pre-admission failure: rollback; admitted rejection: run + `REJECTED` receipt commit. |
| 12 | Delayed-start release (pass) | run-shard fence → run row (3R) → 022 semaphore (7) when the version declares top-level `concurrency` | `run.start_released`, then either `concurrency.acquired` and the entry release (tasks, outbox) or `concurrency.queued` with its queue timer (022 FR-022-018) | `apply_if` re-checks under the run lock; second pass is a skip. |
| 13 | Batch create (API) | namespace active-job fence → job insert | job (`SELECTING`), payload content, audit `batch.created`, idempotency | Nothing persists. |
| 14 | Batch selection round (pass) | job-shard fence → job row `FOR UPDATE` | up to 500 items, cursor, selected count, `RUNNING` when exhausted or at `maxItems` | Crash resumes from cursor; unique (job, run) prevents duplicates. |
| 15 | Batch item (pass) | target run (lock wait at most 2 s) → job-shard fence → job row `FOR UPDATE` → item row `FOR UPDATE`; rerun items: business-id row → source `KEY SHARE` → admission (incl. 022 permit) → job → item | the action's own effects (journal, steps, outbox, audit with `batch {job}` in the reason), item outcome, job counters, `next_slot_at` | Lock wait expired or 022 queue full: separate transaction marks the item `DEFERRED`. Test-tree run: `SKIPPED` `TEST_RUN`. Crash: the item is applied or not, never half. `REPIN` items (019 enabled): run (3R) → 019 catalog and deployment fences (4b, 019's repin order) → job → item; 019's repin journals `run.deployment_repinned`. |
| 16 | Batch cancel (API) / drain (pass) | job row `FOR UPDATE` → items | state `CANCELLING`, audit; then bounded rounds mark items `CANCELLED`, final state | In-flight item commits or rolls back first. |
| 17 | Reset, `REQUIRE_TERMINAL` | receipt → business-id row (`KEEP`) → source `KEY SHARE` → `purged_at`/`test_tree` read → admission → 022 semaphore (acquire only; no queue) | new run, re-emitted prefix `1..N` marked `restored` (read and digested before the transaction from the immutable prefix, verified inside), `run.reset` (clears restored permit state), fresh `concurrency.acquired` when declared, settle events, every `REMATERIALIZE` rule of the reset-rule registry in its slot (020 attribute rows, 019 pins, 021 bindings, 022 permit and provenance, 024 input pins and snapshot refs, 025 agent rows, 026 approval side rows), reapply effects, `run_resets` row, audit `run.reset`, idempotency | Ineligible, too long, test-tree: refused before writing. Purged, or a purge committing first (payload FKs fail): rollback, `CONFLICT`/`RUN_PURGED`. Event type without a registry rule: refused `RESET_RULE_MISSING`. No free permit: rollback, `CONCURRENCY_PERMIT_UNAVAILABLE`. Crash: nothing. |
| 18 | Reset, `CANCEL_SOURCE` request / completion | request: receipt → source run `FOR UPDATE` (ordinary cancel) → pending reset insert; completion: reset-shard fence → pending row `FOR UPDATE` → as 17 | request: cancel effects + pending row + audit + idempotency; completion: as 17 + state `CREATED`, or `FAILED` with reason (`BOUNDARY_NO_LONGER_ELIGIBLE`, `CONCURRENCY_PERMIT_UNAVAILABLE`, `RUN_PURGED`) | Pending row survives restarts; eligibility re-checked at completion. |

**Global lock order**: the merged admission slots of ADR-0050 decision 7 — 1 entry rows (026 link,
027 session) and the read-only 029 check → 2 receipt → 3 run-creating aggregates (shard fence, schedule
or pending-reset row, business-id row, 030 `START` message) → 3R existing runs (chain → run, parent
reservation, ADR-0039 source guard, commanded holder) → 4a namespace fences and memberships → 4b catalog
heads and pins → 5 secrets → 6 new run rows → 7 022 semaphores → 8a bookkeeping rows → 8b usage last.
The table above lists each transaction's path through these slots.

**Two aggregate orders only** (D1036): (a) *aggregate → run* for transactions that create runs or
command an existing run for an aggregate (2, 3, 6–11, 17, 18-completion); (b) *run → aggregate
bookkeeping row* for transactions whose subject is one run (5: obligation, 15: job and item,
18-request: pending-reset insert). Order (b) never locks a schedule or business-id row after the run;
order (a) never locks a job, item or obligation row. Transitions driven by the run itself
(completion, failure, timer, wait timeout, deadline, terminal, continuation handoff) lock none of the
schedule, business-id, job, item, obligation or pending-reset rows; T018-012 and T018-076 prove both.

## Authorization matrix

| Operation | Permissions | Roles granted by default |
| --- | --- | --- |
| Read schedules, actions, next times | `schedule:read` | admin, developer, operator, viewer |
| Create, update, delete a schedule | `schedule:write` + `run:start` | admin, developer |
| Pause | `schedule:operate` | admin, developer, operator |
| Resume (the caller becomes `authorizedBy`) | `schedule:operate` + `run:start` | admin, developer, operator |
| Trigger-now, backfill | `schedule:operate` + `run:start` | admin, developer, operator |
| Disclose schedule input values | `schedule:read` + feature-016 audiences (`payload:read-internal`, `payload:reveal-pii` grant) | per level |
| Start with business id / delay | `run:start` | admin, developer, operator |
| Read business-id binding | `run:read` | admin, developer, operator, viewer |
| Signal-with-Start | `run:start` + `run:signal` | admin, operator |
| Update-with-Start | `run:start` + `run:update` | admin, developer, operator |
| Batch preview | `run:batch` + `run:read` | admin, operator |
| Batch create | `run:batch` + the action's permissions (`run:operate`; `saga:override` when the single action needs it; `run:signal`; `run:update`; `run:operate`+`run:start` for rerun) | admin, operator (O1002) |
| Batch read / cancel | `run:read` / `run:batch` | read: admin, developer, operator, viewer; cancel: admin, operator |
| Reset points, preview | `run:operate` + `run:read` | admin, operator |
| Reset | `run:operate` + `run:start`; reapply `SIGNALS` adds `run:signal`, `UPDATES` adds `run:update` | admin, operator |
| Batch `REPIN` (feature 019 enabled) | `run:batch` + `deployment:manage` + `run:operate` | admin, operator (019 grants `deployment:manage` to both) |

Worker, runner and database-runner roles receive none of these. A schedule action and a batch item
recheck the stored authority (`authorizedBy`, job creator) against current membership; revocation
takes effect at the next action or item.

## Finite ceilings

Design values, not measured capacity (G1005). Effective value = min(request, namespace, deployment);
zero refuses new admissions; existing obligations drain.

| Resource | Default | Hard ceiling |
| ---: | ---: | ---: |
| Schedules per namespace (active + paused) | 200 | 10,000 |
| Spec entries (calendars + cron + intervals) / exclusions per schedule | 10 / 10 | 32 / 32 |
| Minimum spacing between nominal times | 60 s | floor 10 s |
| Jitter | 0 | 24 h, and at most the gap to the next nominal time |
| Catch-up window | 366 d | 10 s .. 366 d |
| Actions per recovery in catch-up mode `ALL` | 100 | 1,000 |
| `BUFFER_ALL` buffer | 100 | 1,000 |
| Tracked open runs per schedule (`ALLOW_ALL`) | 100 | 1,000; beyond it new actions are `SKIPPED_OVERLAP` with reason `TRACKING_LIMIT` |
| Backfill actions per request / concurrent backfills per schedule / per namespace | 1,000 / 1 / 4 | 10,000 / 4 / 16 |
| Recent actions returned / next times returned | 10 / 10 | 100 / 100 |
| Retained actions per schedule | last 1,000 or 30 d | 10,000 |
| Schedule input | 64 KiB | 256 KiB (run input limit) |
| Note | 512 characters | 512 |
| Remaining actions | unset | 1,000,000 |
| Schedule actions per pass round per replica | 100 | 1,000 |
| Business id length | 200 characters | 200 |
| Business-id reuse window | 30 d | 366 d |
| Start delay | 0 | 366 d |
| Active batch jobs per namespace / tenant | 2 / 8 | 16 / 64 |
| Items per job (`maxItems`) | 10,000 | 100,000 |
| Explicit run-id list | — | 1,000 |
| Item rate per job | 20/s | 200/s |
| Item retries on quota/rate/lock refusal | 10 within 1 h | 50 within 24 h |
| Batch job and item retention | 30 d | 90 d |
| Preview sample | 20 run ids | 20 |
| Reset prefix (events re-emitted) | 5,000 | 20,000 |
| Reapplied signals + updates per reset | 64 | 256 |
| Pending `CANCEL_SOURCE` resets per namespace | 16 | 64 |
| Reset points per page | 50 | 200 |

Quota resources added to `crates/domain/src/quota.rs`: `schedules`, `schedule_backfills`,
`batch_jobs`, `tenant_batch_jobs`, `pending_resets`; per-operation bounds use the feature-009 policy
store exactly as feature 017 did (operator ceilings, tenant defaults, namespace overrides).

## Failure and recovery matrix (AGENTS.md rule 9)

| Case | Where | Expected | Task |
| --- | --- | --- | --- |
| Crash at each statement of a firing | transaction 2 | no run, no action; the time fires once after restart | T018-020 |
| Two replicas / stale fence | schedule pass | one run; `scheduler_fenced_total{kind="schedule"}` increments | T018-020 |
| Duplicate request | trigger-now, start with business id, with-start, batch create, reset | same key ⇒ same outcome; changed body ⇒ `IDEMPOTENCY_KEY_REUSED` | T018-024, T018-036, T018-042, T018-045, T018-056 |
| Timeout vs signal | Signal-with-Start against a wait whose timeout is due; `CANCEL_OTHER` vs run finishing | first commit wins; loser is a recorded no-op | T018-040, T018-030 |
| Lost broker delivery | tasks of runs started by schedule, with-start, batch rerun, reset | outbox reconciler republishes; passes find due work by index | T018-020, T018-046, T018-056 |
| Failed compensation | `CANCEL_OTHER`, batch cancel, `CANCEL_SOURCE` whose saga compensation fails | run ends `COMPENSATION_FAILED`/`MANUAL_REVIEW`; buffered schedule action waits and may become `MISSED_CATCHUP_WINDOW`; pending reset waits for a terminal source | T018-030, T018-047, T018-057 |
| Restart / recovery | every pass | due work resumes from committed state; no lease survives a restart beyond its expiry | T018-020, T018-046, T018-057 |
| Outage longer than the window | catch-up | `ONE`: one action + `COALESCED`; `ALL`: bounded; older: one summary | T018-029 |
| Tenant isolation | every route and pass | foreign ids are `NOT_FOUND`; probes audited; a batch never selects another namespace's run | T018-070 |
| Quota / ceiling exhaustion | schedules, backfills, jobs, delayed runs, resets | refused before work; existing work drains | T018-022, T018-048, T018-057, T018-071 |
| Authority revoked | schedule action, batch item | schedule paused / job failed, no effect | T018-021, T018-048 |
| Clock skew between replicas | firing, release, rate slots | decisions use the database clock only | T018-020 |
| Run committing after the selection round passes its id | batch selection | horizon wait (`pg_snapshot_xmin`) makes every transaction open at job creation visible first | T018-046 |
| Run-driven transition vs aggregate path | every aggregate table | no aggregate lock in run-driven transactions; no deadlock | T018-012, T018-076 |
| Purge racing reset / rerun | reset, rerun, batch rerun | `RUN_PURGED`, nothing created | T018-077 |
| No free run permit | reset, `CANCEL_SOURCE` completion | refused / `FAILED`; holder count equals open non-restored acquisitions | T018-055, T018-077 |
| Pause before the trigger handover | trigger-owned schedules | refused; legacy pass keeps firing; reconciliation fills gaps | T018-061 |
| Event type without a reset rule in a prefix | reset points, preview, reset | point not listed; explicit request refused `RESET_RULE_MISSING` | T018-052 |
| A `REMATERIALIZE` check fails at reset time (retired version or snapshot, paused resource, busy permit) | reset, `CANCEL_SOURCE` completion | refused / `FAILED` with the owner's reason; nothing created | T018-077 |

## Observability

**Spans** (live, ADR-0017 decision 3; attributes `sagat.tenant.id`, `sagat.namespace.id`, plus
`sagat.schedule.id`, `sagat.batch.id`, `sagat.run.id`, `sagat.outcome`; never a business id):
`schedule.action`, `schedule.backfill.step`, `schedule.obligation`, `run.start_delayed.release`,
`run.signal_with_start`, `run.update_with_start`, `batch.select`, `batch.item`, `run.reset`.
A reset run's journal-derived spans start after the restored prefix and carry a link to the source
run's trace.

**Metrics** (closed labels only):

| Metric | Type | Labels |
| --- | --- | --- |
| `schedule_actions_total` | counter | `kind` ∈ {scheduled, triggered, backfill}, `outcome` ∈ {started, skipped_overlap, buffered, dropped_buffer_full, dropped_deleted, coalesced, missed_catchup, already_acted, authority_revoked, admission_retry, business_id_conflict} |
| `schedule_action_lag_seconds` | histogram | `kind` |
| `scheduler_lag_seconds` | gauge (existing) | `pass` gains `schedules`, `delayed_starts`, `batches`, `resets` |
| `scheduler_fenced_total` | counter (existing) | `kind` gains `schedule`, `delayed_start`, `batch`, `reset` |
| `schedules_paused_by_system_total` | counter | `reason` ∈ {authority_revoked, failure} |
| `cron_fires_total`, `cron_fire_lag_seconds` | existing | kept for origin `TRIGGER` |
| `business_id_admissions_total` | counter | `outcome` ∈ {bound, use_existing, conflict, reuse_rejected} |
| `with_start_total` | counter | `kind` ∈ {signal, update}, `outcome` ∈ {started, existing, refused, queued} |
| `batch_items_total` | counter | `action` (the closed batch action set, `repin` included), `outcome` ∈ {succeeded, skipped, failed, cancelled, deferred} |
| `batch_jobs_active` | gauge (monitor) | none |
| `batch_item_duration_seconds` | histogram | `action` |
| `run_resets_total` | counter | `outcome` ∈ {created, refused, pending, failed, permit_unavailable, purged} |
| `reset_prefix_events` | histogram | none |
| `reset_reapplied_total` | counter | `kind` ∈ {signal, update}, `outcome` ∈ {reapplied, rejected, not_reapplied} |

**Audit actions** (added to `crates/domain/src/audit.rs`): `schedule.created`, `schedule.updated`,
`schedule.deleted`, `schedule.paused`, `schedule.resumed`, `schedule.auto_paused`,
`schedule.triggered`, `schedule.backfill_requested`, `schedule.backfill_cancelled`,
`schedule.input_disclosed`, `schedule.trigger_handover`, `batch.created`, `batch.cancelled`, `batch.finished`, `run.reset`,
`run.reset_requested`. Runs started by a schedule, with-start, rerun and reset reuse `run.started`
and `run.rerun`; batch item effects reuse the single action's audit (`run.cancel_requested`,
`run.signalled`, `run.updated`, `run.paused`, `run.resumed`, `run.marked_for_review`,
`saga.step_retried`, `run.rerun`) with the job id in the bounded reason. Audit reasons never carry
payloads or business ids.

**Alerts**: Prometheus rules in `infra/observability/alerts.yaml`: `SagatSchedulesLate`
(`scheduler_lag_seconds{pass="schedules"} > 120` for 5 min), `SagatBatchesStalled`
(`scheduler_lag_seconds{pass="batches"} > 600`). Engine-found namespace conditions (ADR-0017
decision on alerts): `SCHEDULE_PAUSED_BY_SYSTEM`, `BATCH_STALLED` as additive alert kinds.

## Migration, compatibility and rollback

1. **Expand.** Additive migrations (timestamp assigned at implementation, after the latest on
   `develop`): `schedules`, `schedule_revisions`, `schedule_actions`, `schedule_backfills`,
   `schedule_obligations`, `business_ids`, `batch_jobs`, `batch_items`, `run_resets`; nullable `runs`
   columns `business_id`, `start_delayed_until`, `schedule_id`, `schedule_nominal_at`,
   `schedule_action_kind`, `schedule_revision`, `reset_sequence`; partial indexes by shard; grants to
   `sagat_app`; `storage-classification.json` entries; ALWAYS triggers refusing updates of immutable
   rows; `schedule_trigger_handover` (singleton, `LEGACY`); the instance-capability registry
   (`serving_instances`, `platform_requirements`, `platform_requirement_epoch`, data-model) unless 029 or
   031 shipped it first, in a release before the handover can be used.
   The migration creates one trigger-owned schedule per existing cron trigger with
   `next_action_at = next_fire_at`; the reconciliation pass keeps them equal afterwards.
2. **Dual run and handover.** While the handover state is `LEGACY`, the old cron pass (old and new
   engines) and the new schedule pass may both fire a tick; the shared key (`trigger:{id}`/
   `cron-{epoch}`) starts one run and a replayed key records `ALREADY_ACTED`; trigger-owned schedules
   are read-only; a reconciliation pass creates or removes trigger-owned schedules so an old control
   plane publishing in the window is covered. The audited operator command
   `control-plane schedule-trigger-handover` is an activation of the instance-capability registry: it
   sets `HANDED_OVER` and records the requirement `schedules-v1` for `EXECUTION_ENGINE` only when every
   live engine instance lists it; new engines then stop their cron pass and trigger-owned schedules
   accept pause, resume and trigger. A later engine without `schedules-v1` refuses to run passes
   (registry rule 3); only a binary older than the registry release could still start (G1011, G1607,
   runbook).
3. **Clients.** New request fields are optional; old SDKs never see `200 USE_EXISTING` because they
   never send the policy; new response fields are optional and ignored by old clients.
4. **Rollback.** Disable new schedule, batch, with-start and reset admission with configuration;
   keep readers. The down migration refuses to drop tables that hold schedules, bindings, jobs or
   pending resets, and refuses to drop `runs` columns that are not null for any run (feature 017
   precedent: no silent loss of authority or evidence).

## Instance-capability registry (owned by 018; used by 029 and 031)

One registry for every control-plane and execution-engine process (data-model, "Instance-capability
registry"): registration before serving under the requirement epoch, a 10 s heartbeat on the database
clock, "live" = seen within 30 s, re-check on epoch change, and activation by an audited operator command
that locks the epoch, verifies every live instance of the required kinds and records the requirement in
the same transaction. 018's trigger handover is its first activation; 029 (`access-narrowing-v1`, its
`serving_instances` and requirement tables become these) and 031 (`payload-encoding-v1`, replacing its
`control_plane_members` and its `scheduler_members.capabilities` column) activate through it. A new table
was chosen over extending `scheduler_members` because that table's rows are the shard-lease membership:
the keeper divides 64 shards by the replicas it lists (ADR-0020 decision 2), so control-plane rows there
would change shard shares, and a scheduler member exists only while the engine runs the keeper.

## Delivery sequence

1. **S0 Foundation** — contracts, ADR-0050 review, domain types, migrations, ports (Phase 1–2).
2. **S1 Schedules core (US01)** — calendar adapter, store, firing pass, create/read API.
3. **S2 Schedule operations (US02)** — overlap, catch-up, pause/resume, trigger-now, backfill,
   remaining actions, pause-on-failure, authority.
4. **S3 Business ids and delay (US03)**.
5. **S4 With-start (US04)**.
6. **S5 Batch (US05)**.
7. **S6 Reset (US06)**.
8. **S7 Trigger-owned schedules (US07)** — after S1–S2 evidence.
9. **S8 Studio, SDKs, docs, gates** — each story's Studio and SDK tasks may run in parallel once its
   API contract is merged.

## Verification plan

Planned targets (do not exist yet): `integration_schedules_bulk_operations`,
`fault_schedules_bulk_operations`, `contract_schedules_bulk_operations`,
`e2e_schedules_bulk_operations`, plus new scenario files run by the existing `sdk_conformance`
target. Planned commands:

```sh
cargo test -p sagat-tests --test integration_schedules_bulk_operations --locked
cargo test -p sagat-tests --test fault_schedules_bulk_operations --locked
cargo test -p sagat-tests --test contract_schedules_bulk_operations --locked
cargo test -p sagat-tests --test sdk_conformance --locked
SAGAT_TEST_AS_RUNTIME_ROLE=1 cargo test -p sagat-tests --test integration_schedules_bulk_operations --locked
python3 scripts/check-architecture.py
npm --prefix frontend run test && npm --prefix frontend run build
```

Each checked task names its evidence in `tasks.md`; skipped or failing tests are reported as such.

## Risks

| Risk | Mitigation / evidence |
| --- | --- |
| Many schedules due at one instant overload a round | Per-round ceiling, fairness of ADR-0020 decision 5 across tenants, jitter; benchmark T018-072. |
| Reset re-emission makes a long transaction | Prefix ceiling 20,000; prefix read outside the transaction; benchmark. |
| Business-id contention on one hot id | Row lock serializes only that id; 32-way contention test. |
| Catch-up storm after a long outage | Default mode `ONE`; `ALL` bounded per recovery; overlap still applies. |
| Developers expect Temporal's buffered signals | Docs, error reason `SIGNAL_NOT_ACCEPTED_AT_START`, quickstart pattern (entry waits for the signal). |

## Complexity Tracking

No constitutional exception is proposed. The one additive IR field, `reapplyOnReset` on Update
declarations, exists because retaining every Update's arguments would add a payload write and a
content charge to every Update on the hot path for a recovery feature (D1038); opting in keeps the
cost on the workflows that need reapply. New tables carry independent semantics (schedule history,
bindings, batch snapshots, pending resets); the simpler alternatives were rejected in ADR-0050:
re-deriving schedules from definitions would lose pause/backfill state, buffering signals would
break ADR-0030 decision 17, and reset by reference to the source journal would couple a run's replay
to another run's retention.
