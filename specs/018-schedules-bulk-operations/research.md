# Feature 018 — Baseline, external semantics, decisions, gaps and owner questions

Baseline: 2026-10-26, branch `feature/018-032-parity-specs` from `origin/develop` `70f09435` (0.2.0).
Nothing here is implemented. Repository facts were read in the cited files; external facts in the
cited pages on that date.

## 1. What exists today (verified)

- **Cron triggers (004).** `crates/domain/src/triggers.rs`: triggers belong to the pinned definition;
  `CronSpec` = five fields (≤ 120 chars) + IANA zone (≤ 64); `MAX_TRIGGERS = 16`.
  `crates/infra-ir/src/cron.rs`: `CronCalendar` (`croner`, `chrono-tz`), `next_after` strictly after,
  never-firing specs refused at publication. `crates/application/src/dispatch.rs`: `CronSchedule` port;
  `advance_cron_trigger` CAS on `next_fire_at`, `delay_cron_trigger` (`retry_at`).
  `crates/scheduler/src/events.rs::cron_pass` (event dispatcher, not the sharded scheduler): starts the
  run under scope `trigger:{id}` / key `cron-{epoch}`, then moves the trigger in a **second** statement;
  a failed start retries the tick after 30 s; missed ticks coalesce (the first missed tick fires once);
  input `{event: {source: "cron", type: "cron.fired", …}, data: {scheduledAt, firedAt}}`;
  `startedBy = system:engine`; metrics `cron_fires_total{outcome}`, `cron_fire_lag_seconds`. Table
  `workflow_triggers` (`crates/infra-postgres/migrations/20261012000000_events.up.sql`). 004 research:
  D85 (trigger ids survive republishing), D87 (cron keyed by the tick), G84 (catch-up once; no
  `skip`/`catch_up_all_bounded`, DST as `croner` does). `docs/architecture/execution-semantics.md` names
  the target DST (`skip|once|twice`) and misfire policies.
- **Runs and signals (002, 008, 014).** `crates/application/src/runs.rs`: `RunStore::create` returns the
  open transaction and `finish_start` runs the initial reduction in it; `NewRun.retained` is the
  precedent for a record admitted atomically with a run; `RERUN_AS_NEW` needs `run:operate` +
  `run:start`, an 8–2000 character justification and a terminal source; `signal` wakes only steps
  `WAITING` on a `waitEvent` of that name, else `StateConflict` — never buffered (also the `signalRun`
  contract and ADR-0030 decision 17). `crates/domain/src/execution/plan.rs`: a released `waitEvent` is
  evaluated inline, so a wait reached from the entry is `WAITING` when the creation transaction
  commits. `command_dedupe` stores a 32-byte request hash per `(tenant, scope, key)`
  (`…/20261009000000_definitions.up.sql`). `runs`, `run_events` (`detail jsonb`, `payload_digest`,
  `actor`), `steps` in `…/20261009000100_execution.up.sql`; chain id = initial run id
  (`…/20261102000000_execution_chains.up.sql`). `RunState::replay(run_id, events)` in
  `crates/domain/src/execution/state.rs`; `verify_run` in `crates/infra-postgres/src/execution.rs`.
  `operation_update_receipts` keeps only a request hash of Update arguments
  (`…/20261030000000_declared_operations.up.sql`).
- **Authority, quotas, scheduler.** `crates/domain/src/identity.rs`: `developer` has `run:start`,
  `run:read`, `run:query`, `run:update`, not `run:operate`/`run:signal`; `operator` has both; `system:`
  subjects cannot come from a token. `crates/domain/src/quota.rs` count resources; ADR-0027: event/cron
  starts are retryable durable admissions. ADR-0020: 64 shards, lease fence read `FOR SHARE` in each
  action, "not what makes the result correct"; `SchedulerAuthority`
  (`crates/application/src/primitives.rs`), `scheduler_lag_seconds{pass}`
  (`crates/scheduler/src/monitor.rs`), `scheduler_fenced_total{kind}` (`crates/scheduler/src/passes.rs`),
  SQL `run_shard(uuid)` (`…/20261017000000_scheduler_shards.up.sql`). ADR-0039: rerun takes the source
  `KEY SHARE` after the receipt check and before admission. ADR-0014: `BeginSafely::begin_safely`.
- **Absent.** No business identifier, reuse/conflict policy, start delay, batch job or reset exists in
  code, contracts or specs (`git grep`).

## 2. External semantics consulted

| Source (2026-10-26) | Relevant facts | Use in 018 |
| --- | --- | --- |
| https://docs.temporal.io/schedule | Intervals, calendars, cron strings, exclusions, start/end, jitter (bounded by the next action), zone; overlap Skip (default), BufferOne, BufferAll, CancelOther (start after cancellation completes), TerminateOther, AllowAll; catch-up window default 1 year, minimum 10 s, not applied to triggers/backfills; pause-on-failure; notes; trigger while paused; backfill; limited actions; id + timestamp. | Same vocabulary; no terminate (D1004); coalescing mode (D1006); no repeated nominal time (D1008). |
| https://docs.temporal.io/workflow-execution/workflowid-runid | One open execution per id per namespace; reuse Allow Duplicate (default) / Failed Only / Reject, for the retention period (example 30 days); conflict Fail (default) / Use Existing / Terminate Existing. | D1013, O1001. |
| https://docs.temporal.io/sending-messages | Signal-With-Start signals or starts-and-signals; Update-With-Start needs a conflict policy, is not atomic, may start without delivering. | D1016, D1017. |
| https://docs.temporal.io/workflow-execution/timers-delays | Start delay; with-start skips it, plain signals do not; not with schedules/cron. | D1015. |
| https://docs.temporal.io/cli/workflow, https://docs.temporal.io/cli/batch | Reset by event id or `LastContinuedAsNew`; batch reset restricted; `--rps` with a query; batch jobs described, listed, terminated without rollback. | D1019, D1022. |
| https://docs.temporal.io/develop/typescript/workflows/cancellation (via search result text) | Reset terminates the current execution and starts a new one from a point. | No terminate here (D1024). |
| https://www.windmill.dev/docs/core_concepts/scheduling | `croner` syntax with optional seconds; disable; error/recovery handlers; one queued occurrence, others skipped; no backfill. | Confirms `SKIP` default; handlers are 022/026. |

Concepts that exist only for code replay (workflow tasks, sticky queues, signal-handler determinism)
do not apply: a run is a reduction of its journal over a pinned declarative IR.

## 3. Decisions

- **D1000** Schedules are namespace resources (not definition content) that start ordinary runs of a
  workflow, `ACTIVE` version at each action or pinned. Schedules add nothing to the IR (D1030 lists
  the one Update field).
- **D1001** An action that starts a run commits fence check, schedule CAS, decision, authority check,
  run admission (journal, tasks, outbox, audit), action row (unique tenant/schedule/nominal time) and
  schedule state in one transaction, under run key `schedule:{id}`/`nominal-{ms}`. Correctness comes
  from the keys; the fence bounds redundant work (ADR-0020). Database clock only.
- **D1002** Spec = union of ≤ 32 calendars (AND of fields), five-field cron strings (004 grammar) and
  epoch-aligned intervals with phase, minus exclusions, within `startAt`/`endAt`, one zone (UTC
  default), DST gap `SKIP` (default, = 004) | `SHIFT_FORWARD`, ambiguous `ONCE_EARLIER` (default, = 004)
  | `TWICE`. Second resolution, computed in infra-ir behind a port. Refused when it never fires or two
  of its next 1,000 occurrences are closer than the minimum spacing.
- **D1003** Jitter = SHA-256(schedule id, nominal time) mod jitter, bounded by the next gap:
  deterministic across restarts, no random source.
- **D1004** Overlap `SKIP` (default), `BUFFER_ONE`, `BUFFER_ALL` (bounded, overflow
  `DROPPED_BUFFER_FULL`), `CANCEL_OTHER` (ordinary cancellation, new action buffered until terminal),
  `ALLOW_ALL`. No `TERMINATE_OTHER`: there is no terminate, and stopping without draining attempts and
  compensations would contradict constitution III.
- **D1005** A schedule tracks its runs last seen open and reads their status without a run lock; safe
  because terminal status is monotonic (a stale read only delays). No run transition touches a
  schedule row.
- **D1006** Catch-up `catchupWindow` (default 366 d, 10 s..366 d) and `catchupMode` `ONE` (default:
  earliest missed time inside the window, the others one `COALESCED` summary row) | `ALL` (each, up to
  the per-recovery ceiling, the rest coalesced). Older misses: one `MISSED_CATCHUP_WINDOW` summary.
  Summaries keep storage bounded after long outages. `ONE`/366 d = G84.
- **D1007** Paused nominal times are neither acted on nor caught up; resume continues after the resume
  instant; notes ≤ 512 chars, audited. `pauseOnFailure` on `FAILED`, `TIMED_OUT`,
  `COMPENSATION_FAILED`, `PARTIAL` (not `CANCELLED`).
- **D1008** Trigger-now acts at the request's database time (also paused, optional overlap override,
  keyed by the request, no remaining-actions decrement). Backfill `[start, end)` with `end <= now`,
  incremental, default `BUFFER_ALL`, `ALREADY_ACTED` for times that have an action; re-execution of a
  past action is a rerun.
- **D1009** Immutable revisions; an action keeps the revision it was decided under; after an update the
  next nominal time is the first after the update.
- **D1010** `authorizedBy` (last creator/updater/resumer) is rechecked for `run:start` (and 017 database
  authority where needed) at every action; if revoked: `AUTHORITY_REVOKED`, auto-pause, audit (O1000).
- **D1011** Scheduled runs: `startedBy = system:schedule:<id>`, schedule provenance on the run,
  optional `nominalTimeField` (one declared top-level input property), optional business-id prefix
  (`<prefix>-<nominal time>`).
- **D1012** Each cron trigger of an active version is a `TRIGGER`-origin schedule fired by the schedule
  pass with the 004 key, input document, `system:engine`, `ALLOW_ALL`, catch-up `ONE`/366 d, no jitter,
  004 DST. Spec changes only through publication. During a rolling upgrade the shared key makes the
  old cron pass and the new pass start one run per tick; control of these schedules waits for the
  handover gate (D1034).
- **D1013** Business id: optional, 1–200 visible ASCII, unique among open executions of a namespace
  (run, or chain by its initial run). Conflict `FAIL` (default) | `USE_EXISTING`. Reuse
  `ALLOW_DUPLICATE` (default) | `ALLOW_DUPLICATE_FAILED_ONLY` (allowed unless the holder ended
  `SUCCEEDED`) | `REJECT_DUPLICATE`, within a reuse window (default 30 d, O1001). No terminate-existing.
- **D1014** Lock order: the merged slot order of ADR-0050 decision 7 (D1606): receipt (2) → shard
  fence and schedule or pending-reset row → business-id row (3) → chain → existing run → ADR-0039 source
  guard (3R) → namespace fences and memberships (4a) → catalog heads (4b) → secrets (5) → new run rows
  (6) → 022 semaphores (7) → usage (8b). Batch items: run (3R) → fence → job → item (8a). See D1036 for the two allowed aggregate orders;
  transitions driven by the run itself never lock schedule, business-id, job, item, obligation or
  pending-reset rows; binding openness is read from the monotonic holder status.
- **D1015** `startDelayMs` (1 ms..366 d): run created `WAITING` with `startDelayedUntil` and
  `run.start_delayed`, counts as active, holds its business id; a fenced run-shard pass journals
  `run.start_released`. Cancel/deadline apply; signals refused; Updates admitted; not with
  Signal-with-Start or schedules; with run-level concurrency the permit is taken at release (D1608).
- **D1016** Signal-with-Start under the binding lock: signal an open holder exactly as `signalRun`, or
  create a run and deliver to steps waiting after its initial reduction in the same transaction, or
  roll back everything (also when 022 would queue the new run). No buffer (ADR-0030 decision 17).
- **D1017** Update-with-Start: `USE_EXISTING` (default) | `FAIL`, one transaction; pre-admission
  failures create nothing; an admitted business rejection leaves the run started with a `REJECTED`
  receipt (ADR-0031 decision 9).
- **D1018** Batch selection: structured filter (existing filters + `versionId`, `scheduleId`, creation
  range, `businessIdPrefix`) or ≤ 1,000 run ids; 020's query later through 020's `RunSelector` contract
  (D1033). Selection is evaluated per round over keyset pages after a transaction horizon (D1033) and
  is fixed once an item is materialized, ≤ `maxItems`; the filter is not re-evaluated for a
  materialized item, applicability is rechecked under the run lock. Preview: count to `maxItems + 1`,
  ≤ 20 ids.
- **D1019** One run per item transaction via the single-run use case, key `batch:{job}:{run}`; outcome
  and counters commit with the effect; sequential per job at a durable rate (`next_slot_at`); jobs in
  parallel up to a ceiling; creator authority rechecked per item (revoked ⇒ job `FAILED`); quota/rate
  refusals defer with bounded backoff; cancel stops new items and undoes nothing.
- **D1020** Batch needs new `run:batch` (admin, operator; O1002) plus the single action's permissions;
  029's evaluator, when present, hides runs from selection.
- **D1021** Actions `CANCEL`, `SIGNAL`, `UPDATE`, `RERUN`, `RETRY_STEP`, `PAUSE`, `RESUME`,
  `MARK_MANUAL_REVIEW`, and `REPIN` when feature 019 is present (D1607); grows additively under the
  same per-item contract.
- **D1022** Reset points: `run.started` and completions of root nodes (`step.completed`,
  `child.finished`, `collection.finished`, `loop.finished`) with no attempt in flight, no waiting step,
  no active scope, no compensating saga, no unknown outcome/manual review, and no later compensation,
  skip, resolution or reconciliation of a prefix instance. Sources: root runs outside chains.
- **D1023** The source events `1..N` are re-emitted under the new run identity (digests, actors, times,
  trace ids kept), then `run.reset` and the settle reduction; replay equals projection without the
  source; telemetry starts after N with a link to the source trace. Re-executed steps use the new run's
  effect keys (reusing the source's would let two runs claim one effect, including compensation).
- **D1024** `REQUIRE_TERMINAL` (default) | `CANCEL_SOURCE` (ordinary cancel + pending reset completed by
  a fenced pass when the source is terminal and the point still eligible). The source never reopens;
  provenance `rerun_of` (ADR-0039 guard) + `reset_sequence`.
- **D1025** Reapply `NONE` (default) | `SIGNALS` | `UPDATES` | `ALL`: signals recorded `via: signal` and
  Updates with retained arguments after N, journal order, ≤ 64, inside the creation transaction; a
  signal is delivered only to a step waiting then, the first undeliverable one ends signal reapply;
  Updates are re-validated. Receipts keep only a hash, so only Updates whose declaration opts in with
  `reapplyOnReset: true` retain their arguments (D1038); others are `NOT_REAPPLIED` (`NOT_RETAINED`),
  older records `LEGACY_EVENT`.
- **D1026** Rerun and reset keep the source's business id when free (`KEEP`, default; reuse policy not
  applied to an explicit, justified operator action), else 409; `DROP` starts without one.
- **D1027** Schedule input, batch payloads and retained Update arguments are content; ordinary reads
  return digest and size; schedule input values only through an audited read with feature-016
  audiences. A business id is caller metadata like `correlationId`, classified as customer `content`:
  never a metric label, span attribute, log field, audit reason or outbox field, and erasable by
  feature 020 (D1037).
- **D1028** No new `Error.code`; `details.reason` values in `contracts/errors.md`.
- **D1029** Permissions `schedule:read` (admin, developer, operator, viewer), `schedule:write` (admin,
  developer; + `run:start`), `schedule:operate` (admin, developer, operator; + `run:start` for
  trigger/backfill), `run:batch` (admin, operator).
- **D1030** REST additive; worker protocol unchanged; the workflow IR gains only the optional
  `reapplyOnReset` on Update declarations (ADR-0032 additive, capability-gated); AsyncAPI run-finished
  data gains optional `scheduleId`, `nominalTime` only (generated values keep `event_outbox.payload`
  protected, ADR-0027).
- **D1031** The rules that cannot be reversed without migrating data (D1001, D1004, D1005, D1013,
  D1014, D1016, D1023, D1024, D1032, D1035, D1036, D1605, D1606) are proposed as ADR-0050.
- **D1032** Every run this feature creates goes through the run-admission path of feature 022 (its T10:
  acquire a run permit or enqueue) when the version declares top-level `concurrency`. Schedule actions,
  business-id starts, Update-with-Start and batch reruns may be enqueued (a queued run is open for
  overlap and binding purposes; a full queue is an admission refusal: owed time, deferred item or
  synchronous 429). Signal-with-Start whose new run would be queued is refused
  `SIGNAL_NOT_ACCEPTED_AT_START` (no step waits). A reset (and a `CANCEL_SOURCE` completion) MUST
  acquire the permit in its transaction and cannot queue: no free permit ⇒ 409
  `CONCURRENCY_PERMIT_UNAVAILABLE` (reset completion: `FAILED`). The `run.reset` reducer clears the
  run-permit state of the restored prefix; restored `concurrency.*` events are history and are not
  counted by ADR-0056 decision 1 because their sequence is ≤ `reset_sequence`; the fresh
  `concurrency.acquired` follows `run.reset`.
- **D1033** Batch selection: the job records `selection_horizon = pg_snapshot_xmax(pg_current_snapshot())`
  at creation and starts selecting only when `pg_snapshot_xmin(pg_current_snapshot()) >= horizon`, so
  every transaction in progress at creation has finished; then keyset rounds by run id over runs with
  `created_at <= selection_bound_at`. Runs created by later transactions with skewed process clocks may
  or may not fall inside the bound; the result is "evaluated per round, fixed once materialized". With
  020, the selector is 020's `RunSelector` (`contracts/list-query.md`, keyset-paged, evaluated per round,
  bounded count preview, no total ceiling for unattended selection), so `maxItems` up to 100,000 stands.
- **D1034** Trigger-owned schedule control waits for a durable handover: a singleton state `LEGACY` →
  `HANDED_OVER`, changed by an audited operator command that is an activation of the **instance-capability
  registry** (coordinator decision; data-model): every control-plane and engine process registers before
  serving (under the requirement epoch), heartbeats every 10 s on the database clock, counts as live
  when seen within 30 s, re-checks requirements when the epoch moves, and refuses to serve when a
  requirement for its kind names a capability it lacks; an activation locks the epoch, verifies every
  live instance of the required kinds and records the requirement in one transaction. The handover
  requires `schedules-v1` from every live engine. 029 (`access-narrowing-v1`) and 031
  (`payload-encoding-v1`) use the same registry instead of their own `serving_instances` /
  `control_plane_members` variants; a new table is preferred to `scheduler_members`, whose rows drive
  the shard shares of ADR-0020. In `LEGACY` the old cron pass runs,
  trigger-owned schedules are read-only (pause/resume/trigger refused `SCHEDULE_TRIGGER_HANDOVER_PENDING`)
  and a reconciliation pass creates or removes trigger-owned schedules to match `workflow_triggers`
  (covering an old control plane publishing during the upgrade); in `HANDED_OVER` the cron pass of new
  engines stops. The per-engine `SAGAT_CRON_PASS` switch is dropped.
- **D1035** Action identity has two origins: `SPEC` (scheduled and backfill actions share the spec's
  nominal times, so a backfill finds `ALREADY_ACTED`) and `MANUAL` (trigger-now, nominal time =
  `clock_timestamp()` in microseconds). The action key is (tenant, schedule, origin, nominal time); the
  run key is `nominal-{ms}` or `manual-{µs}`. A trigger-now never collides with a scheduled time.
- **D1036** Two aggregate lock orders only: (a) aggregate row → run, for transactions that create runs or
  command an existing run on behalf of an aggregate (schedule action, business-id paths, reset
  completion); (b) run → aggregate bookkeeping row, for transactions whose subject is one run (batch
  item → job → item, `CANCEL_OTHER` obligation → obligation row, `CANCEL_SOURCE` request → pending-reset
  insert). Order (b) never locks a schedule or business-id row after the run; order (a) never locks a
  job, item or obligation row. Transitions driven by the run itself (completion, failure, timer, wait
  timeout, deadline, terminal, continuation handoff) lock none of these rows.
- **D1037** Feature 020 interplay: a reset re-creates `run_search_attributes` rows by folding the
  re-emitted `search_attributes.upserted` events of the prefix (same sequences); every re-emitted journal
  row carries `detail.restored = true` and the re-emitted `run.started` carries `reset_of`, so 020
  analytics and any journal reader can skip restored attempts; a purged source (`purged_at` set, or a
  missing prefix row) refuses reset and rerun with `CONFLICT`/`RUN_PURGED` (020's code and reason,
  `specs/020-visibility-analytics-archival/contracts/errors.md`), checked after the ADR-0039 guard, and a purge committing first makes the re-emitted rows' payload foreign keys fail
  so the reset rolls back with the same answer. `runs.business_id` and `business_ids` are erasable: the
  immutability trigger allows exactly one change, to `NULL`, in the statement that sets
  `archive_state = 'ERASED'`, and erasure deletes the run's binding rows.
- **D1038** Update-argument retention is opt-in per declared Update (`reapplyOnReset: true`, additive IR,
  default false): only those Updates pay the content charge on the hot path. Rejected alternative:
  retaining every Update's arguments, a new hot-path cost and retention class for a recovery feature.
- **D1039** Reset, rerun-by-batch and batch actions refuse 022 test-tree runs (`test_tree = true`):
  reset/preview `RESET_SOURCE_TEST_RUN`, batch item `SKIPPED` `TEST_RUN`. (The earlier refusal of a start
  delay with run-level concurrency is replaced by D1608.)
- **D1040** (2026-10-29) The Rust SDK is an official SDK since constitution 1.1.0 (feature 034, ADR-0070),
  after this draft was written for five SDKs. FR-018-048 and the SDK scenarios of
  `contracts/sdk-conformance.md` therefore cover **six** SDKs: task T018-081 adds the Rust client in
  `sdks/rust/` and T018-068 runs all six. The published task ids are not renumbered; the text of
  FR-018-048 and SC-018-013 says "five" and is read as six.
- **D1041** (2026-10-29, T018-003) `CANCEL_OTHER` keeps **one** action waiting, whatever the buffer limit:
  the open runs are being cancelled, so a second waiting action would only queue behind the first; a
  further due time while one waits is `DROPPED_BUFFER_FULL`. FR-018-006 does not say; one is the smallest
  reading that keeps the "buffered until they are terminal" promise.
- **D1042** (2026-10-29, T018-003) With `remainingActions`, every waiting action **holds one remaining
  action** from the moment it is buffered. A due time with no unreserved remaining action is `Exhausted`
  (no row, no next action, the schedule acts as paused), so a buffered action can always start later.
  FR-018-012 says the count is decremented by `SCHEDULED` actions that start a run; the decrement still
  happens at the start, the reservation only stops over-buffering.
- **D1043** (2026-10-29, T018-003, found by a property test) An action that is waiting goes before any new
  time under every policy but `ALLOW_ALL`. `SKIP` never buffers, but an update can change a schedule from
  `BUFFER_ALL` to `SKIP` with actions still waiting; a new time is then `SKIPPED_OVERLAP`, not a start
  ahead of them. The pass releases waiting actions (`release`) before it decides a new time.
- **D1044** (2026-10-29, T018-003) The domain has no hash. `jitter_seed(schedule_id, nominal)` returns the
  24 bytes to hash (id, then the nominal time in whole milliseconds, big-endian) and
  `jitter_offset_ms(digest, jitter, gap)` reads the first eight bytes of the SHA-256 digest modulo the
  bound; the layer that owns `sha2` (application) hashes. This keeps D1003 (SHA-256, no random source)
  without a new domain dependency (`crates/domain/AGENTS.md`).
- **D1045** (2026-10-29, T018-005) A refused batch item (quota, rate, lock wait, full 022 queue) waits 5 s,
  doubling to 5 min, and fails after 10 refusals or 1 hour since its first (hard 50 within 24 hours), the same
  bounded backoff as a refused schedule action (plan, transaction 4). FR-018-031 says "bounded backoff" and
  the plan gives the ceilings but not the curve. The next rate slot is `max(slot, now) + interval`, so a late
  job never catches up in a burst.
- **D1800** (2026-10-30, T018-024) Trigger-now starts its run as the schedule (`startedBy` is
  `system:schedule:<id>`, FR-018-016) under the **role of the caller**, who must hold `schedule:operate` and
  `run:start` themselves, so triggering never lends the authority of `authorizedBy` (which may be a person
  with more rights, or none left) and a caller who cannot start runs cannot make the schedule start one. Its
  nominal time is the application clock to the microsecond (the same clock the firing uses), not a separate
  database read: the identity of the action is `(schedule, MANUAL, µs)`, a collision of two requests in the
  same microsecond is a `STATE_CONFLICT` for the later one, and the stored times are what a replay returns.
  The `Idempotency-Key` is recorded in `command_dedupe` (scope `schedule-trigger:<id>`) in the transaction
  that records the action, so a request and its key commit together or not at all. Numbered above the
  reserved blocks because 018's own (D1000..D1039) is full (G1016).
- **D1605** Reset-rule registry (coordinator C1): every journal event type has one rule (`COPY`,
  `REMATERIALIZE`, `EXCLUDE`, `REFUSE` — a window until a named closing event) in
  `contracts/journal-and-events.md`; existing types are owned by 018 for this purpose; drafts 019, 020,
  021, 022, 024, 025, 026 and 030 declared theirs (each row names where); a prefix containing a type
  without a rule is refused `RESET_RULE_MISSING`. `REMATERIALIZE` work runs inside the reset transaction
  in ADR-0050's slots; owners' refusal reasons (`RESOURCE_UNAVAILABLE`, `SNAPSHOT_RETIRED`,
  `CONCURRENCY_PERMIT_UNAVAILABLE`, ...) surface as the reset's failure reason.
- **D1606** Merged admission lock order (coordinator C4), ADR-0050 decision 7. The coordinator's 8-slot
  proposal was refined against the drafts and the implemented code: (1) existing runs a transaction acts
  on (parent reservation, ADR-0039 source guard, a commanded holder) form slot **3R**, ahead of the
  017 namespace catalog lock and the memberships, because 017's implemented order and ADR-0039 (a
  reproduced deadlock) put them first; (2) the memberships and the 017/021/024 namespace fences
  therefore move from the proposal's slot 1 to slot **4a**, leaving slot 1 to entry rows (026 link,
  027 session) and the read-only 029 check, which keeps 029's rule "fences right after memberships";
  (3) 024 asset heads stay in slot 4b before secrets (C4 and 024), so 021's rank table must move asset
  heads ahead of secrets; (4) slot 8 splits into 8a bookkeeping rows of run-subject transactions and 8b
  usage last. Slot numbers 4–8 keep the proposal's meaning, so 019, 021, 022, 024 and 025 citations
  stay valid.
- **D1607** `REPIN` batch action (coordinator C14): available only when feature 019 is enabled (else
  `VALIDATION_FAILED`/`BATCH_ACTION_UNAVAILABLE`); parameters `deployment` and a 019 `VersionRef`
  target; needs `run:batch`, `deployment:manage` and `run:operate`; each item calls 019's single-run
  repin (FR-019-009) in its transaction; a terminal run is `SKIPPED` `ALREADY_TERMINAL`, a run without a
  pin for that deployment `SKIPPED` `NOT_APPLICABLE`.
- **D1608** Start delay and run permits (coordinator C19): aligned with 022 FR-022-018 — a run with
  `startDelayMs` holds no permit during the delay; the delayed-start release transaction acquires the
  run permit or enqueues the run (slot 7, under the run-shard fence). The refusal
  `START_DELAY_WITH_CONCURRENCY` is withdrawn.
- **D1609** Registries of other drafts are cited, not re-specified: with 020 present, the run-list
  filters `businessId`/`businessIdPrefix`/`scheduleId` are 020's `BusinessId`/`ScheduleId` system
  attributes (`list-query.md`) and batch selection uses 020's `RunSelector` (identity re-resolved each
  round, `QUERY_UNSUPPORTED_OPERATOR` when off); encoded-field positions of schedule input, with-start
  inputs and the business-id prefix are 031's registry (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`;
  Update arguments cannot carry encoded values); retained Update arguments and 018 payload references
  are entries of 020's content-store registry (`specs/020-visibility-analytics-archival/data-model.md`);
  schedule and delay instants are classified by 028's time-class catalog
  (`specs/028-developer-tooling/contracts/time-classes.md`: `workflow_due`/`workflow_record`, batch
  selection bounds `infrastructure`).

## 4. Gaps

| ID | Gap | Reason / closure | Status |
| --- | --- | --- | --- |
| G1000 | Definition cron triggers keep G84 semantics; no policy fields in the trigger IR. | API schedules offer them; an IR extension needs its own publication/diff rules. | Accepted |
| G1001 | No reset point while a branch waits or inside an active scope; no reset of chain segments or child runs. | Per-kind re-arming rules need their own evidence. | Accepted |
| G1002 | No batch reset or deletion. | Reset needs a per-run choice; deletion is a retention decision (G42/G69). | Accepted |
| G1003 | No terminate variants; no "cancel then start" conflict policy. | No terminate exists; a start waiting for a drain is unbounded. | Accepted |
| G1004 | Nominal time reaches a run only through `nominalTimeField`. | A run-metadata expression namespace is an IR change. | Accepted |
| G1005 | Firing, batch rate and reset size ceilings are design values. | T018-072 benchmark on a declared environment. | Open |
| G1006 | `REJECT_DUPLICATE` lasts only for the reuse window. | Finite ceilings (ADR-0027); O1001. | Open (owner) |
| G1007 | An auto-paused schedule is shown in audit, state and a namespace alert, not sent to a person. | Notification channels are 026. | Accepted |
| G1008 | No shared holiday/exclusion calendar resource. | New scope. | Accepted |
| G1009 | Start delay with run-level concurrency was refused. | Closed by D1608: 022 FR-022-018 defines acquire-or-enqueue at release. | Closed |
| G1010 | 020's `RunSelector` section was missing. | Closed: `specs/020-visibility-analytics-archival/contracts/list-query.md` § `RunSelector` (co-owned with 018) now defines it. | Closed |
| G1011 | After the handover, rolling back to an engine without schedules re-enables the legacy cron pass, which ignores pause (the shared key still prevents duplicates); a registry-aware engine without `schedules-v1` refuses to run instead, so this applies only below the registry release (G1607). | Rollback runbook states it. | Accepted |
| G1012 | Shared files and numbers with the other open branches (checked 2026-10-29): the 014 Vacate branch (`feature/014-capacity-vacate`, PR #59) appends three audit actions to `crates/domain/src/audit.rs` and two lines to `crates/domain/src/execution/mod.rs`, as this feature does (a trivial textual conflict at the end of `AuditAction` and of `mod.rs`); migration `20261226000000` is taken by 014 (`capacity_transfer_receipts`) and reserved by 036, so this feature's migration takes a later timestamp chosen after a fresh grep of every branch (T018-008). | Resolve the conflicts on merge, keeping both sets; T018-008 re-greps `origin/*` and the sibling worktrees. | Open |
| G1013 | The `dst` policy of a spec (gap `SKIP`/`SHIFT_FORWARD`, ambiguous `ONCE_EARLIER`/`TWICE`) applies to its structured calendars. Cron strings keep the daylight-saving behaviour feature 004 gave them (the cron library's own rules), whatever the policy says, so moving a trigger to a schedule never changes when it fires. FR-018-002 reads as if `dst` covered every entry; the cron library does not expose its handling, and re-implementing the five-field grammar to apply the policy would risk the 004 behaviour. | Tested by `a_cron_string_keeps_the_feature_004_daylight_saving_behaviour`; the Studio form and the docs say the policy is for calendars. | Open (owner) |
| G1014 | `createSchedule`, `updateSchedule`, `deleteSchedule`, `pauseSchedule` and `resumeSchedule` require an `Idempotency-Key` (8 to 160 printable characters, like every operation that changes something) but do not replay by it: a request sent again after a lost answer is not answered with the original result. It is refused by what the first one did (409 for a name in use or a stale `expectedRevision`, 404 for a schedule that is gone; pause and resume are idempotent by their nature). Nothing is applied twice. | The spec asks for replay by key only for trigger-now and the with-start operations (FR-018-024, FR-018-026); a replay store for schedule writes needs its own table and retention (ADR-0050 decision 4). | Accepted |
| G1016 | D1040..D1045 (the Rust SDK, `CANCEL_OTHER`, the waiting reservation, the waiting order, the jitter seed, the item backoff) were written on this branch on 2026-10-29 in the numbers the parity branch gives feature 019 (its D1040..D1079). 018's own block (D1000..D1039) is full, and the check of that day looked at files and migrations but not at the decision blocks. Both sets are published, so neither is renumbered here. | The owner decides which side moves (suggested: 018's six become D1800..D1805 when PR #53 and PR #63 meet, with the references in code and tasks updated in the same change); new 018 decisions use D1800 and up meanwhile. | Open (owner) |
| G1017 | A waiting trigger-now counts as one of the remaining actions a scheduled time must leave (the reservation of D1042 counts every waiting action), though it never uses one up; with `remainingActions` set and a trigger-now waiting, a scheduled time can find the schedule exhausted a time early. | Rare (a limit, an open run, a buffering rule and a manual action at once); the fix is to count waiting scheduled actions only, which needs the waiting count by kind in `Standing`. | Open |
| G1015 | `listSchedules` takes `cursor` and `limit` only. The draft fragment also listed the filters `origin`, `state`, `workflowId` and `name`; they are not built, so the canonical contract does not name them, and the response `action` carries `inputDigest` but not `inputBytes`. | A filter needs an index per column (`schedules_by_workflow`, `schedules_by_state`) and a keyset that follows the filter; the store lists by id only. | Open |
| G1607 | A binary older than the instance-capability registry release does not register, so no activation can see it; activations therefore require the registry release deployed fleet-wide first (operator precondition, checked by nothing but the runbook), and rollback below a recorded requirement is unsupported. | Inherent to introducing the registry; 029's prerequisite release (T029-003) is the vehicle. | Accepted |
| G1605 | Draft lock orders that deviated from ADR-0050 decision 7 on 2026-10-27 (026 T2 membership and rate buckets after the link row; 030 endpoint head before memberships; 021 asset heads after secrets and the variable fence as an "authority slot"). Resolved the same day: 021 and 026 now follow decision 7 exactly (021 D1624, 026 D1337 revised); 030 follows it with one listed exception, a caller transaction locking its own `START` row after its run (030 D1660), recorded in decision 7. | Owners 021, 026, 030. | Resolved |
| G1606 | `REMATERIALIZE` checks of other features (retired version, paused resource, retired snapshot, busy permit) are evaluated at reset time; a preview reports them as provisional and a reset can still fail on them. | Inherent to reset at a later time. | Accepted |

## 5. Owner questions (the plan implements the recommended defaults)

**Answered 2026-10-29** by the owner, who approved this feature and accepted the recommended default of
each question (`docs/owner-decisions.md`): **O1000 (a)** pause with an audited system note; **O1001** 30 days,
namespace-configurable up to 366; **O1002 (a)** `run:batch` for admin and operator only. The three
paragraphs below are kept as the question that was put.

- **O1000 — Schedule authority after its authorizing subject loses access.** (a) Pause with an audited
  system note at the next action; (b) keep firing under a namespace service identity.
  **Recommended: (a)** — fails closed (constitution V); (b) needs a service-principal model that does
  not exist.
- **O1001 — How long a closed business id is remembered.** 7, 30 or 366 days, or per namespace up to a
  ceiling. **Recommended: 30 days default, namespace-configurable up to 366** (Temporal's example
  retention; finite ceilings).
- **O1002 — Who holds `run:batch`.** (a) admin and operator; (b) also developer for actions a developer
  may take one by one. **Recommended: (a)** — a batch multiplies an action's blast radius by up to
  100,000.
