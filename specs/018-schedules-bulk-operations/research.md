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
- **D1801** (2026-10-30, T018-025) A backfill is a state machine of the schedule: each step is one transaction
  that takes the first nominal time of its range after the cursor and commits the action (or the decision) together
  with the new cursor, the counters and the state of the backfill, under the same lock order as any other action
  (shard fence, schedule row, backfill row). It runs under the role of its requester, checked again at every step
  (a requester who lost `run:start` stops it), and it works while the schedule is paused, because it is the
  operator's explicit request and not one of the schedule's own times. A backfill never drops a time: when the
  overlap rule would drop one (`DROPPED_BUFFER_FULL`) the step waits instead and tries the same time again, so the
  cursor never passes a time that was neither acted on nor found acted on. Its times take no remaining action. The
  range holds at most 1,000 times (hard ceiling 10,000, T018-022) and a schedule runs one backfill at a time.
- **D1802** (2026-11-03, T018-020) The firing acts on the database's time, not on the process's. The pass finds
  what is due by `clock_timestamp()`, but `ScheduleFiring` plans the times it acts on (`occurrences(.., until = now)`)
  and decides whether a retry has waited with the clock it is given; its module doc said the engine feeds that clock
  from the database, and the engine fed `SystemClock`. A test with a process 3 hours ahead showed the cost: the firing
  folded times 2.5 hours in the future into a `COALESCED` summary and moved the schedule beyond them, so those runs
  would never start (and a process behind started nothing). `DatabaseClock<L>` (`infra-postgres/src/clock.rs`) keeps
  the difference between the database's clock and `L` (one `SELECT clock_timestamp()` on a connection taken from the
  pool first, so the wait for a connection is not counted; the answer is placed at the middle of the interval the
  question took) and answers with `L` corrected by it. It is used wherever a schedule's time is computed:
  the engine reads the database before every round (`loops::schedule_round`), and the control plane, whose
  `ScheduleService` and trigger-now compute the first time of a created, updated or resumed schedule and the nominal
  time of a manual action, syncs at start and every 5 s. **A clock that was never measured does not act:** the
  engine's round returns `failed = 1` and does nothing until the first reading succeeds, because acting on the
  process clock is the failure this removes and it cannot be undone; after a first reading a database that cannot be
  asked leaves the last difference, which drifts by milliseconds in a minute. Run creation (`RunService`) still stamps
  its own rows with the process clock, as every other run does. Alternatives refused: passing the database time
  through `DueSchedule` (rewrites every firing test that moves a clock by hand), and a clock trait with an async
  `now` (changes every `Clock` user).
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
- **D1950** (2026-11-04, T018-021) The authority of a schedule's action is checked where the run is admitted, and
  a database step stands on it. (1) `ScheduleAdmission.authority` names the subject the action acts for
  (`authorizedBy` for a scheduled time and a released waiting action, the requester of a backfill, the caller of
  trigger-now). `RunStore::create` locks that membership `FOR SHARE` (through `sagat_lock_memberships`, the only way
  the runtime role can) and checks `run:start` again, after the shard fence and the schedule row and before the quota
  checks, so a revocation that has returned stops the run and one that has not waits for this transaction. The refusal
  is `StateConflict(AUTHORITY_REVOKED)`, committed nothing; the firing records the time `AUTHORITY_REVOKED` and pauses
  the schedule (a scheduled time, a waiting action), stops the backfill, or answers trigger-now `Forbidden` (the caller
  lost the right; the schedule is not paused for it). The replay of a key is answered before this check. Before this,
  the role was read once outside the transaction and a revocation committing in between did not stop the run.
  (2) A run started by `system:schedule:<id>` stood on a subject that no membership has, so a workflow with a managed
  database step could not be admitted at all: the 017 admission found no database authority for the starter and the
  time stayed owed for ever (`fire` answered `NotFound`). The 017 actors of such a run are now the publisher and the
  member `authorizedBy` of the schedule today (`database::authority_of`, also for the root starter of a child and for
  the replay check), and a schedule action whose workflow has such steps requires of that member `database:use` (and
  `database:write` when a step writes) besides `run:start`, taking the catalog lock before the membership as 017
  does. Losing database authority is losing the authority of the schedule (`AUTHORITY_REVOKED`, paused). A run
  started by trigger-now or a backfill stands, for its database steps, on `authorizedBy` as well as on the role of
  its requester; the requester only presses the button. Not done: a run keeps no record of whose authority it
  started under, so a database step claimed after the schedule was resumed by someone else is admitted under the new
  `authorizedBy` (G1950).
- **D1951** (2026-11-04, T018-029) A recovery is planned once and remembered. FR-018-008 limits a catch-up: mode `ONE`
  acts on the earliest time inside the window, mode `ALL` on up to `catchup_actions`, and the rest is one `COALESCED`
  summary. A step of the pass acts on one time and planned the backlog again from the next one, over a slice of
  `catchup_actions + 2` times, so the limit held for one slice and never for a backlog: 20 owed times with a ceiling of
  5 started 20 runs in mode `ALL` and 3 in mode `ONE`, and a hundred thousand owed times would have started them all
  with a thundering herd of admissions, which is what the ceiling exists to prevent. The schedule now carries
  `recovery_until` (the instant the recovery's times end; what falls due later is the next recovery) and
  `recovery_acts` (the actions it may still take), migration `20261230001000_schedule_recovery`, both null outside
  a recovery. The pure `plan_recovery_step` (domain) decides one step from the slice and the remembered recovery:
  `Missed` (a summary of up to `SUMMARY_CHUNK` = 10,000 times older than the window, the recovery untouched), `Act`
  (`settle` on the last allowed action or the last time owed, when the pass counts every later time up to the end of
  the recovery into one `COALESCED` row in the same transaction and forgets the recovery; otherwise it remembers the
  actions left, but only when the backlog is longer than the recovery may act on), `Coalesce` (the actions are
  used up and times remain: one more summary), `Finished` and `Idle`. The slice a step reads is a chunk of 10,000
  when it only summarizes (a year of per-minute times is a few dozen steps, not thousands) and `acts left + 2`
  otherwise. Counting the rest is `ScheduleFiring::tally`: up to 100 calls of the calendar, a million times (G1951).
  A revision, a pause or resume and a delete clear the recovery; trigger-now and a backfill leave it alone. A
  property test runs a whole recovery step by step over random backlogs, windows, modes and ceilings and checks that
  the actions never exceed the mode's allowance, are the earliest times inside the window, and that every due time
  is acted on, coalesced or missed exactly once. Alternatives refused: counting the whole backlog in the first step
  without remembering (unbounded work, and the same herd if the count is cut), and a `COALESCED` marker in the
  `counters` object (it is returned by describe and bounded to 1 KiB).
- **D1952** (2026-11-04, T018-029) A time that cannot start because the workflow has no published version is owed
  with the same backoff as a full quota (5 s doubling to 5 min), not retried on every round: `StateConflict` with
  `runs::NO_PUBLISHED_VERSION` is told apart from the `StateConflict` of "another process acted first", which still
  commits nothing and defers nothing.
- **D1953** (2026-11-04, T018-030) `CANCEL_OTHER` owes each open run an ordinary cancellation, recorded with the
  action that waits and paid by the pass. The `Buffer { cancel_open }` decision of the domain was ignored: the action
  waited and nothing cancelled the run. The decision transaction now inserts one `schedule_obligations` row per run the
  schedule tracks as open, with the key `schedule:{id}:cancel:{µs of the waiting time}:{run}`, and makes the schedule due
  at once (a schedule with a debt is looked at in the next round, not in five seconds). The pass pays the oldest debt
  before anything else, right after the authority check: it calls the cancel use case of the run service
  (`RunService::operate`, `run.cancel_requested` audited by `system:schedule:<id>` with the reason "the overlap rule of
  the schedule is CANCEL_OTHER") under that key, then records the debt `DONE`. **The first commit wins:** the cancel
  of a run that ended by itself (`StateConflict`) or is gone (`NotFound`) leaves nothing to do and the debt is paid;
  `Forbidden` means the subject the schedule acts for may not cancel runs and is the loss of its authority
  (`AUTHORITY_REVOKED`, paused); any other failure leaves the debt and the pass tries again. The waiting action
  starts through the existing release once the runs are over (a run that waits for review is not over). Two
  transactions, not the plan's one: the plan puts the shard fence and the obligation row inside the run's cancel
  transaction, which would change the cancel path of the run store for one caller. The key makes the pair
  idempotent instead (a cancel that committed before the record is replayed as a no-op, so the run is cancelled once),
  and the fence guards the record: a replica that lost its lease may repeat the cancel it was about to repeat anyway,
  but cannot record it, and the new owner pays again. Paying a recorded debt is not a new decision, so it needs no lease
  to be right. **Who may write the rule:** cancelling a run needs `run:operate`, which a developer lacks, so creating,
  updating or resuming a schedule with `CANCEL_OTHER`, asking for it in a backfill and overriding the rule in
  trigger-now need `run:operate` (admin, operator), and the authority check of every action asks for it too (O1950).
- **D1954** (2026-11-04, T018-030) A debt may be paid after the run that owes it was forgotten: the pass forgets runs that
  ended before it looks at debts, and a debt for a run that is terminal is paid by the `StateConflict` of the cancel, not
  by a special case, so the code has one path.
- **D1956** (2026-11-04, T018-016) The input of a schedule is checked for shape and size, not against the `inputSchema` of the
  target version (G1955): nothing on the platform enforces that schema, so a schedule would be stricter than a person
  starting the same workflow.
- **D1957** (2026-11-04, T018-045) Batch jobs: the model and who may do what. A job is a row that never changes except in
  its progress (an `ALWAYS` guard refuses every other `UPDATE`, a finished job staying finished) with one item row per
  selected run and no foreign key to `runs`, so retention of an item never waits for a run. Creating it needs
  `run:batch` and every permission of the single action (`action_permissions`): `run:operate` for cancel, pause, resume and
  retry-step and mark-manual-review (the single-run `Hold`), `run:signal`, `run:update`, and `run:operate` with `run:start` for
  rerun; `REPIN` is `BATCH_ACTION_UNAVAILABLE` while feature 019 is not enabled. The creation is judged under a fence
  (the tenant's first, then the namespace's, the same order everywhere) against the jobs that have not finished, 2 per
  namespace and 8 per tenant by default (hard 16 and 64). The selection bound, the horizon (the identity of the creating transaction, D1958; D1957 first said
  `pg_snapshot_xmax(pg_current_snapshot())`, which was wrong) and the first slot come from the database inside the
  creating transaction, so the executor can wait until every
  transaction that was open then has ended. The key (`Idempotency-Key`, 8 to 160 characters) is recorded with the job: a
  repeat answers the job it made, a changed request is `IDEMPOTENCY_KEY_REUSED`. The payload of a signal or the
  arguments of an update are a payload of the tenant, named by a digest, and neither the job record nor its audit record
  carries the body; the audit reason is the action name. A `businessIdPrefix` selector is refused
  (`BATCH_SELECTOR_UNSUPPORTED`) until business ids exist, because there is no column to match. New reasons of the contract:
  `BATCH_PARAMS_INVALID`, `BATCH_ACTION_UNAVAILABLE`, `BATCH_LIMITS_INVALID`, `BATCH_PAYLOAD_INVALID`,
  `BATCH_PAYLOAD_TOO_LARGE`, `BATCH_SELECTOR_UNSUPPORTED`, carried by the new `AppError::InvalidBatch`.
- **D1958** (2026-11-04, T018-046) The selection waits for the transactions that were open at creation, and the horizon is the
  creator's own identity. D1033 recorded `pg_snapshot_xmax(pg_current_snapshot())` as the horizon and the condition
  `pg_snapshot_xmin(pg_current_snapshot()) >= horizon`. In PostgreSQL the `xmax` of a snapshot is the last completed
  transaction identity plus one, not the next one to be assigned, so a transaction that was open with exactly that identity
  passes the condition while it is still open: a test with one held transaction showed `held 916, horizon 916, xmin 916`.
  The horizon is now `pg_current_xact_id()` of the transaction that creates the job and the condition is `xmin > horizon`:
  the selection starts once every transaction that had an identity when the job was created has ended. A transaction that
  began before the job and takes its identity only after it (a run whose `created_at` was read before the first write)
  can still be missed; it is concurrent with the creation of the job, which commits first (G1957).
  **The executor.** One call of `BatchExecutor::step` does one thing to one job: a page of the selection (500 runs by id
  after the cursor, up to `maxItems`, items numbered after the ones already there), a page of the drain of a cancelled
  job, or one item. An item waits for the durable slot (no faster than the rate; a job that ran late never catches up in a
  burst), re-checks the creator (a creator who lost `run:batch` or a permission of the action fails the job
  `AUTHORITY_REVOKED` and the items that had not ended are cancelled, so the counters equal the outcomes), applies the
  single-run use case under the key `batch:{job}:{run}`, and records the outcome, the counters and the next slot under the
  fence of the shard in one transaction; the record that makes the last item end also completes the job. As with
  `CANCEL_OTHER` (D1953) the effect and the record are two transactions made safe by the key: a crash between them costs a
  repeat that changes nothing, and a replica that lost its lease may repeat the effect but cannot record it. No step holds
  two run locks. What a use case refuses because the run is not in the state the action needs is a skip with its closed
  reason (`ALREADY_TERMINAL`, `NOT_APPLICABLE`, `NOT_WAITING`, `RUN_CONTINUED`, `HIDDEN_FROM_CREATOR` for a run that is not
  there or not visible); a quota, a busy tenant or an unavailable store defers the item with the bounded backoff of the
  domain (5 s doubling to 5 min) and an item refused more than the retries allow or for longer than the window fails
  `RETRIES_EXHAUSTED`; any other error fails the item with a closed code. A job asking for `UPDATE` is refused when it is
  created (`BATCH_ACTION_UNAVAILABLE`, G1959), as is a step named with an instance path (G1956).
- **D1959** (2026-11-04, T018-045) The routes of batch operations follow the contract and, where the code had named something
  its own way, the contract wins. `BATCH_ACTION_PARAMETERS` is the reason for an action that needs or refuses a
  parameter (the service said `BATCH_PARAMS_INVALID` and was renamed); a job that already ended is `409`
  `BATCH_NOT_CANCELLABLE` (the service had left it alone and answered with it; it now refuses before it audits, and a job
  being cancelled that is cancelled again is still one cancellation, one audit record). The use cases reach the HTTP layer
  as one trait object (`BatchUseCase`, implemented by `BatchService`) so the routes take no new generic parameter, like
  the schedule routes. Admission authorizes the namespace of the path (`run:batch` to create or cancel, `run:read` to read,
  both to preview); the service then asks for the permissions of the action; a job is placed by `namespace_of`, which
  needs no permission, so a caller with `run:batch` and no `run:read` can still cancel, and a job through the path of
  another namespace of the same tenant is `404`. The ids of a selector travel as strings and are parsed here, so a
  malformed id is `BATCH_SELECTOR_INVALID` at `/selector/...` and not a generic body error. The routes exist only behind
  `SAGAT_BATCHES_ENABLED` (control plane and engine, off by default), on the database clock shared with the schedules.
- **D1960** (2026-11-04, T018-045..050) What an independent correctness and security read of the batch lane changed. (1) A
  reason is trimmed, 8 to 2,000 characters of plain text with no control character, the rule the single-run `review` and
  `rerun` already applied to the reason of each item: a job whose reason they refuse would be accepted and then fail every
  item. (2) The name of a signal is 1 to 128 characters of `A-Za-z0-9_.:-` starting with a letter or digit, as the
  single-run route takes it. (3) A job does not return the digest of its payload, only `hasPayload`: an unsalted SHA-256
  of a low-entropy payload (`{"approved":true}`) lets anyone who can read the job (`run:read`) confirm a guess of what
  they may not read (the contract draft listed `payloadDigest`; it is dropped). (4) The keys that start with `batch:` and
  `schedule:` are the platform's own and are refused as an `Idempotency-Key` on every route, so a caller cannot decide what
  an item or a `CANCEL_OTHER` cancellation does by sending the key first. (5) The caller is part of what a replay means:
  the same key and body from another person is `IDEMPOTENCY_KEY_REUSED`, not the first person's job. (6) A request that
  waited for the creation fence looks for its key again after it, so two requests with one key make one job and the second
  is answered with it, whatever the ceiling. (7) The selection bound is read before the payload insert gives the creating
  transaction its identity, so no transaction can take an identity between the horizon and the bound (what remains is
  G1957). (8) The cancellation reason is checked and kept in the audit record (G1960 is closed for the reason; the key is
  still not replayed), and the audit record of a run changed by an item names the job (`batch {job}: {reason}`).
  (9) `due_jobs` returns only the jobs that can do something now: a selection whose horizon has not passed and a running job
  whose items are all deferred to a later time are left out, so a hundred of them no longer hide every other job from the
  pass. (10) Immediately before an effect the executor looks once more (`ensure_live`) at the lease, the state of the job
  and the state of the item, so a replica that lost its lease, a job that was cancelled or an item that has ended does not
  touch the run (what is left is G1961). (11) The fencing metric counts lost leases only. (12) The selection runs under a
  10 s statement timeout and is tried again.
- **D1961** (2026-11-05, T018-034/035) A business identifier is claimed inside `PgRunStore::create`, in the slot of
  the run-creating aggregates (after the schedule's row, before any run's, ADR-0050 decision 7), by `INSERT ... ON
  CONFLICT DO NOTHING` and, when the id exists, `SELECT ... FOR UPDATE` of its row: a concurrent claim that has not
  committed makes the insert wait for its outcome, so of 32 starts of a free identifier exactly one binds it, and of 32
  that find a closed holder exactly one takes the binding over (the row lock makes the others read the new, open
  holder). The holder's status is read from `runs` and, when the run is the initial run of a chain, from
  `execution_chains` (open until a final outcome; ended when the last segment did) and never copied, so no run-driven
  transition touches `business_ids`. A holder that is gone reads as free. The result of creating a run is a dedicated
  `Admitted { Created, Replayed, Existing }` instead of a third variant of the generic `Applied` (28 users, most
  unrelated): `Existing` is `USE_EXISTING` returning the open holder, answered `200`.
- **D1962** (2026-11-05, T018-035) The business options join the idempotency fingerprint of a start only when there are
  any (a second length-prefixed group appended), so the fingerprint of a start without them is what it always was and a
  retry made before an upgrade still replays. The same key with another identifier or policy is `IDEMPOTENCY_KEY_REUSED`.
- **D1963** (2026-11-05, T018-034) `business_ids.execution_id` has no foreign key to `runs`: a holder purged by a
  retention policy leaves a binding that reads as free and that `business_ids_purge` removes (it is also what a
  migration to feature 020's erasure needs). The identifier is charged twice (the binding and the copy on the run),
  both as content, by the ALWAYS accounting triggers; `runs.business_id` is immutable after the insert.
- **D1964** (2026-11-05, T018-035) The refusal names the holder (`details.runId`) only to a caller who may read runs
  (`run:read`), as `errors.md` says; the identifier is never in an error, a log line, a metric label or any table but
  the two that keep it (`privacy::`, which searches every table of the schema as text and as hex).
- **D1965** (2026-11-05, T018-035) The identifier of a start is refused with `422 BUSINESS_ID_INVALID` and the place
  (`/businessId`, `/businessIdConflictPolicy`, `/businessIdReusePolicy`) when it is not 1 to 200 visible ASCII
  characters, when a policy comes without an identifier, or when it comes with `inputBlobRef`; a rerun and a schedule's
  action take the identifier from the run they repeat (T018-035, built: D1966) or never carry one
  (`BUSINESS_ID_NOT_APPLICABLE`). A prefix filter is a literal (`LIKE` with `\`, `%`, `_` escaped).
- **D1966** (2026-11-05, T018-035, `FR-018-040`) A rerun (`RERUN_AS_NEW` and a batch `RERUN`) chooses what becomes of the
  identifier of the run it repeats. `KEEP` is the default and claims it for the new run with `FAIL` and `ALLOW_DUPLICATE`:
  the source has ended, so it is a closed holder inside its window and is taken over, while another open execution that
  holds it by now refuses the rerun with `409 BUSINESS_ID_IN_USE` (the holder named only to a caller with `run:read`), never
  a rerun that quietly lacks the identifier. `DROP` starts without one and asks nothing of the binding, so it succeeds when
  the identifier is held. The choice is part of the request a key stands for: `KEEP` extends the fingerprint with the
  identifier the same way a start does (D1962) and `DROP` leaves it as it is, so the same key under the other choice is
  `IDEMPOTENCY_KEY_REUSED` in either order (a marker for `DROP` was tried and its mutant survived: it distinguished nothing). `businessId` (`KEEP`/`DROP`) of the operation request is refused with `422` for any other
  operation. In a batch job a held identifier fails the item with `CONFLICT` (it is for the operator to read and settle, not
  a state the item skips), and the job's `businessId: DROP` is the `dropBusinessId` its parameters already carried. A
  rerun of a private (ad hoc) run never has an identifier.
- **D1967** (2026-11-05, T018-034/035, `FR-018-022`) The lookup of a binding is `GET /namespaces/{namespaceId}/business-ids/
  {businessId}` (`getBusinessIdBinding`, `run:read`), the path the feature contract named, read without a lock by the
  runtime role. It answers `404` in exactly the cases in which a start finds the identifier free: unbound, a holder that is
  gone (purged), and a closed holder past its window (`now >= finished + window`, one `Held::reusable_after` in the domain
  that both the claim and the lookup use); an identifier that cannot be one and a namespace of another tenant are as absent
  as one that never existed. `executionId` is the holder (the run, or the initial run of a chain), `currentRunId` the
  segment that carries the work, `lastStatus` how the holder ended and `reusableAfter` when `REJECT_DUPLICATE` stops
  applying; the last two are absent while it is open. `listRuns` also filters by `scheduleId` (the runs a schedule started,
  whatever the kind of action), the third filter `FR-018-022` names.
- **D1968** (2026-11-05, T018-006/037, `FR-018-023`) A start delay is two journal events and one column. `run.start_delayed
  {until_us}` follows `run.started` in the creation transaction (the reducer accepts it only there, once) and
  `run.start_released {until_us}` releases the entry, once, at that same instant; `RunState` holds `start_delayed_until_us`,
  so a replay of the journal equals the stored projection (`verify_run`). While it is set the planner is idle, which is the
  whole hold: the entry stays `PENDING`, no timer starts, no wait begins and no worker is offered a step; a stop is planned
  before the hold, so a cancellation or a deadline still ends the run with no step having run, and the status is `WAITING`
  until then. `runs.start_delayed_until` mirrors the state (written with the two events, in the same transaction) so the
  release pass finds the due ones by a partial index on `(run_shard(id), start_delayed_until) WHERE ... status = 'WAITING'`;
  a trigger lets it be set once and cleared, never changed. The pass (`start_releases`, after the deadlines) applies
  `release_start` under the run's lock with the condition read again there (still delayed, due by the database clock,
  not stopped, not paused), so a run cancelled since the candidate query is skipped; it is fenced by the run's shard lease
  like every other pass. A signal is refused because nothing waits for it yet (the entry has not begun). Whether a declared Update is admitted to a
  held run, as FR-018-023 says, is not tested yet (G1972). A schedule's action and a rerun never take the options of a start, so they cannot carry a delay;
  the delay joins the idempotency fingerprint only when present (D1962). The run `WAITING` while held is a status that
  already existed, and `startDelayedUntil` says why. The release of 022's run-level permit is the half this does not do
  (T018-037, blocked on 022).
  A schema that predates the delay (a rollback, or a rollout whose migration has not run) still works: the loader of a run
  reads the column through a snapshot of the row, so a missing column is `NULL` and not an error for every pass of the
  scheduler, and the pass that looks for due releases treats an undefined column as having none (the rollback tests keep
  driving the scheduler on the schema they roll back to).
- **D1969** (2026-11-05, T018-040, `FR-018-024`, `FR-018-026`) Signal-with-Start is one request over mechanisms that
  already exist, and adds no table or event. The identifier is claimed with `UseExisting` whatever the caller wrote (an open
  holder is always signalled; the reuse policy still governs a closed one). Then one of three things: the identifier has an
  open holder, which is signalled by the ordinary `signal` use case (`202`, `started: false`; a holder that waits for
  nothing is `409` `CONFLICT` with `details.reason` `NOT_WAITING` and nothing changes); the same key was already served,
  and the answer is the first one, with the nodes it woke read from the receipt the signal left (`run.signal:{run}`); or a
  run is created, and the signal is delivered by the same core as `signalRun` (`deliver_signal`) inside the creation
  transaction, after the initial reduction, so it reaches every step that waits for it then. If none does the transaction is
  dropped and the answer is `409` with `SIGNAL_NOT_ACCEPTED_AT_START`: no run, no binding, no stored signal and the
  identifier is free (tested by taking it with an ordinary start afterwards). A signal is never buffered. The signal's name
  and payload join the idempotency fingerprint, so the same key with another signal is `IDEMPOTENCY_KEY_REUSED`, and the
  right is `run:start` and `run:signal` both (a Developer has the first, an Operator both). A start delay is refused at the
  route (`422` `START_DELAY_WITH_SIGNAL`) because a held run waits for nothing. The answer is `202` `{runId, started,
  deliveredTo}`; the route is admitted as `RunStart` (one token of the namespace, also for a request the engine refuses).
  The signal marker in the fingerprint matters for a key shared by two operations: an ordinary start under key K and then a
  with-start under K with the same body would otherwise replay the start and answer as if a signal had been delivered (the
  receipt of the signal catches a changed signal, not a missing one). The task names `runs/with_start.rs`; the code is in
  `runs.rs` beside `signal`, which it shares `deliver_signal` with.
  *Amended 2026-11-05 after an independent review (security and correctness readers):* an open holder that serves the
  request keeps, in the transaction of the signal, the receipt of the request itself (`run.start:{ns}` + key, naming the
  run the signal reached) and a marker (`….served`, same key and fingerprint) that says a holder, not a new run, was the
  answer. `create` replays the first before it claims anything, so a retry made after the holder has closed and the
  identifier is free is answered by the holder, is not delivered to a new run, and says `started: false` as the first
  answer did; the same key with another body is `IDEMPOTENCY_KEY_REUSED` whoever holds the identifier (`G1973` (1) closed).
  A holder that is a continuation chain is its initial run; the signal goes to the segment that is current
  (`RunStore::current_segment`), and `runId` names the run that was reached. A replay whose signal receipt is missing is an
  internal error, not an empty answer.
- **D1970** (2026-11-05, T018-041, `FR-018-025`, `FR-018-026`) Update-with-Start applies the declared Update in the
  transaction that creates the run, through the code of `updateRun` and not a copy of it. `apply_update` of the PostgreSQL
  store was split: `apply_update_in_tx` does everything between the begin and the commit (authorization, chain and run
  locks, receipt replay, validation, evaluation, state, receipt, journal event, run version, audit, outbox) in a
  transaction the caller owns, and the store method opens one and commits it. `RunTx` offers `apply_declared_update`, which
  an adapter that cannot refuses (a wiring bug, 500); `PgRunTx` hands over its own transaction, and the result bytes are
  read out of that same transaction (`update_result_in_tx`), so the route reads nothing a second time. The service
  creates the run (claim of the identifier, initial reduction, persisted), applies the Update, and commits. A failure
  before admission (an undeclared operation, arguments the declaration does not accept, an update key reused with
  another request, a run that no longer admits updates) returns the error and drops the transaction: no run, no
  binding, no receipt, and the identifier is free. A business rejection (the validator said no, the expected revision is
  stale) is `Ok` with a `REJECTED` resolution and the commit keeps the run, its binding and the rejected receipt. An open
  holder (`USE_EXISTING`, the default) and a retry of the same request go through a transaction of the run itself (`begin`
  with the run lock), where the Update is applied or, for a retry, answered again from its receipt, even if the run has
  closed since; `FAIL` makes an open holder `BUSINESS_ID_IN_USE`. The start's idempotency fingerprint carries the
  operation name, the update key, the canonical arguments and the expected revision, so one key is neither a plain start
  and an Update-with-Start nor two different Updates (`IDEMPOTENCY_KEY_REUSED`). The answer is `200` for an applied
  and for a rejected Update (the contract; `updateRun` answers a rejection 409/422), `Cache-Control: no-store`, and says
  `started`. The service serves it only when it holds an evaluator (`with_declared_updates`): the control plane and the
  test platforms do, the engine's internal run services do not. A held run (`startDelayMs`) is `WAITING`, which an Update
  admits (FR-018-023): G1972 is closed by `delayed run admits the update`.
  *Amended 2026-11-05 after review:* the same receipt for an Update served by an open holder (one transaction with the
  Update, its receipt, journal event, audit and outbox), the same current-segment targeting for a chain holder, the
  journal event dated by the database clock of the run's own transaction (`tx.now()`, as its other events) instead of the
  process clock, and a server that was not wired with an evaluator answers `500`, as this decision says, not `422`.
  The Update is counted in the telemetry of declared operations as `updateRun` is (the `workflow.update` span,
  `workflow_operation_duration_seconds{kind="update"}`, and `workflow_primitive_rejections_total` for a rejection, which is
  an answer to the caller here and a rejection in the metrics). The platform's own update keys (`batch:`, `schedule:`) are
  refused to callers of `updateRun` and `updateWithStart` (`INVALID_UPDATE_KEY`), so a caller cannot make the item of a
  batch job fail by taking its key first.
- **D2012** (2026-11-06, T018-057, review L) One creation, two callers; the audit says which reset it completes. The request of an
  ended source (`reset_ended`) and the pass (`complete_reset`) assemble the creation from one function, `RunService::create_from_prefix`
  (the reapply plan, the acknowledgement, the digest of the prefix, the business identifier, the run quotas and the `ResetStart`), and
  differ only in who is judged (the caller now, or the member who asked with the role they hold now) and in what they do with a
  refusal (an answer to the caller, or `failure_of`): the same defect has one reason in both. The records of a reset carry its id
  in closed words and nothing else of it: `reset {id} requested at {N}...` and `cancelled for the reset {id}` on the source (the request),
  `reset from {source} at {N}... completes reset {id}: {justification}` on the run that was made, and `completes reset {id}: failed
  {REASON}` on the source when the entry failed (the completion); the runbook says the join between them is by the reset id. The
  drift between the two callers that the review saw (the reapply plan before or after the acknowledgement; whether the plan is gated by the
  setting) cannot happen any more because there is one order. Tests: the whole `cancel_source` suite runs the completion through the helper
  and `reset`, `reapply` and `reset_rest` run the request of an ended source through it; the audit assertions of
  `the_run_is_created_once_the_source_has_ended_from_the_prefix_the_request_named` and `every_line_the_pass_logs_names_the_tenant_and_the_reset`.
- **D2011** (2026-11-06, T018-057, review I) The register says what was asked. A request for an open source that finds the source ended
  (D2005 (3)) or that is made for a source that has ended is reset at once, and the entry now keeps the `source_policy` that the request
  said (`CANCEL_SOURCE`, not `REQUIRE_TERMINAL`) and the acknowledgement it gave (`acknowledged_reexecution`): the register is a record of
  requests, and what the platform did with them (nothing to cancel) is the state. Test:
  `the_register_keeps_the_policy_and_the_acknowledgement_that_were_asked_for_a_source_that_ended`.
- **D2010** (2026-11-06, T018-057, review H) The justification is part of the request. The fingerprint of the receipt of a reset
  holds the justification as it is judged, audited and kept (trimmed), in **both** paths: the same `Idempotency-Key` with another
  justification is `409 IDEMPOTENCY_KEY_REUSED`, not the first entry with a second justification that is never audited. Before, neither
  path had it. Test: `the_same_key_with_another_justification_is_not_the_same_request_in_either_path`.
- **D2009** (2026-11-06, T018-057, review E, ADR-0079) Which optional passes the engine runs is a pure function of its configuration,
  `settings::optional_passes`, which returns the names `schedules`, `batches` and `resets` in the order the engine starts them; `run()` starts
  exactly those. The reset pass is in the list only when `SAGAT_RESET_CANCEL_SOURCE_ENABLED` is on, and its settings alone do not turn it
  on. Tests: `the_pass_that_completes_resets_is_absent_unless_the_operator_turned_it_on` (closes G1992 (2)),
  `no_shipped_manifest_turns_the_reset_switch_on`. ADR-0079 records that the per-process flags stand in for the instance-capability
  registry (ADR-0050 decision 9), the order engine first, and that nothing may make the feature default-on before the registry.
- **D2008** (2026-11-06, T018-057, review D) The acknowledgement is asked **before the source is cancelled**, and also for a step in flight.
  A cancellation is not taken back, and an attempt that is leased or running when the request is made usually completes after the
  cancellation: the journal the source ends with then has a step completed after the point, and without the acknowledgement the entry
  would be `FAILED` `REEXECUTION_NOT_ACKNOWLEDGED` after the customer's run was cancelled. So `ReexecutedStep.in_flight` (a step after the
  point, in the full replayed journal, with a status of leased or running) joins `completed_in_source` in `needs_acknowledgement()`, and
  the request (and the preview, `requiresAcknowledgement`) needs `acknowledgeReexecution: true` when either holds: otherwise
  `422 RESET_REEXECUTION_NOT_ACKNOWLEDGED`, with nothing cancelled and no entry. The completion still judges it again on the journal the
  source ended with (a step leased only after the request). The API does not name the step in flight in the preview body (G1994). Tests:
  `domain/tests/reset.rs` (the in-flight bit), `a_step_in_flight_when_the_request_is_made_needs_the_acknowledgement_and_nothing_is_cancelled_without_it`,
  `nothing_in_flight_and_nothing_completed_after_the_point_asks_for_no_acknowledgement`,
  `an_entry_whose_source_ended_with_a_step_done_after_the_point_and_no_acknowledgement_fails`, and over HTTP
  `a_step_in_flight_needs_the_acknowledgement_and_a_request_without_it_cancels_nothing`. The CLI says in its usage and in the preview
  that a step in flight is the reason, and the runbook and D1999 say it.
- **D2007** (2026-11-06, T018-057, review B) Only an answer of the platform ends an entry. `From<StoreError> for AppError` turns an
  unavailable database, a missing reference and a connection limit into `NotFound` and `NotAllowed`, which `failure_of` took for
  `ADMISSION_REFUSED`, a state the trigger makes final, after the source had been cancelled. The completion now classifies the store's
  failure **before** that conversion (`store_failure(pending_completion, error)`, applied where the creation talks to the store): the
  answers of the admission (a quota, a business identifier held, a conflict, an invalid start) are converted as ever, and anything else
  becomes `AppError::Unavailable` for the pass, which leaves the entry pending and backs off (D2006); every other caller is told what it
  always was. A source the requester cannot read at completion time (`reset_candidate` answering `NotFound`) is not `AUTHORITY_REVOKED`: a source
  read at request time cannot disappear, so it is an anomaly that leaves the entry pending with a warning (no reason is invented). Tests: the table
  of scripted answers `only_an_answer_of_the_platform_ends_an_entry_and_anything_else_leaves_it_pending`,
  `a_source_the_requester_cannot_see_is_an_anomaly_that_leaves_the_entry_pending_and_not_a_lost_authority`, and the unit tests of
  `store_failure` and `failure_of`.
- **D2006** (2026-11-06, T018-057, review A) An entry that keeps failing does not hold the head of the scan. The scan was oldest first with
  `LIMIT batch`, so an entry that errored in every round was first in the next, and with `batch` such entries (or `batch = 1`) every
  younger entry on the replica's shards starved. Now: (1) `due_resets(shards, limit, per_tenant, skipping)` takes at most `per_tenant`
  entries of any tenant per round (`row_number() OVER (PARTITION BY tenant_id ORDER BY created_at, id) <= k`, as the schedule pass does;
  `SAGAT_RESET_CANCEL_SOURCE_PER_TENANT`, 1 to 1,000, default 10) and leaves out the ids it is told to; (2) the pass remembers in memory the entries
  that failed and leaves each out of the scan until its time: 2 s after the first failure, doubling to 5 minutes, at most 1,024 entries
  remembered (past that the one whose time comes first is forgotten), forgotten at once when the entry is made, failed on purpose, found idle
  or refused; (3) only the two known conflicts (`RESET_ALREADY_COMPLETED` and `SHARD_LEASE_LOST`) are `Refused` (debug); any other
  `StateConflict` is an error with a warning and the backoff; (4) every line the pass logs names `tenant_id` and `reset_id`. The memory is the
  process's: a restart tries every entry once. Tests: `an_entry_that_keeps_failing_does_not_hold_the_head_of_the_scan_of_the_others`,
  `an_entry_that_keeps_failing_does_not_hold_back_the_younger_entries_of_its_own_tenant`, `the_scan_takes_at_most_its_share_of_each_tenant_and_leaves_out_those_it_is_told_to`,
  `the_pass_leaves_an_entry_that_failed_until_its_time_and_forgets_it_once_it_is_made`, `every_line_the_pass_logs_names_the_tenant_and_the_reset`,
  the unit tests of `Backoff` (the delay doubles and stops at the ceiling; the memory is bounded) and `only_the_two_known_races_are_taken_for_races`.
- **D2005** (2026-11-06, T018-057, `FR-018-038`) What a request does with a source that is not quiet, and with one that ended while it waited.
  (1) A source that is already stopping (`stop` set: it was cancelled, or a saga is undoing it) is not cancelled a second time: the
  entry is written and waits for it (no second `run.stop_requested`, no second audit record of the cancellation). (2) A source in
  `MANUAL_REVIEW` whose compensation failed is **open**, and when it is not stopping yet it is cancelled in the ordinary way like any
  other (the cancellation is journaled and the run stays in manual review until an operator acts); the entry waits. (3) A source
  that ended between the first look at it and the moment the request got its row is not an error and not a pending entry: the
  transaction is dropped and the request is made as a reset of an ended run (the journal is read again, `CREATED`, a run, nothing
  cancelled). (4) The audit trail of a request is two records: `run.cancel_requested` on the source, whose reason says only
  `cancelled for the reset {id}`, and `run.reset` on the source, whose reason is `reset requested at {N}[ reapply=POLICY]: {justification}`
  (the justification is audited by contract and nothing else of the caller's content is); the run made later has the `run.reset` of
  every reset, and an entry that fails has one by the engine with the closed reason. Tests: `cancel_source::a_source_that_is_being_cancelled_already...`,
  `a_source_whose_compensation_is_stuck_is_open_and_the_entry_waits_for_an_operator`, the two orders of the race in
  `fault_schedules_bulk_operations cancel_source`, and the audit assertions of `a_request_cancels_an_open_source_in_the_ordinary_way...`.
- **D2004** (2026-11-06, T018-057, `FR-018-047`, `FR-018-048`) The surfaces. (1) `ResetMade.run` is an `Option`: a request that waits
  has none; the route answers `202` with the entry either way. (2) OpenAPI: the descriptions of `resetRun`, `ResetRequest.sourcePolicy`,
  `Reset.state` and `Reset.failureReason` say it; the three new reasons and `RESET_CANCEL_SOURCE_NOT_AVAILABLE` and `PENDING_RESET_LIMIT`
  are in `contracts/errors.md`, the API's closed table of sentences and the lists of the contract test. Additive: an older client keeps
  working, and `failureReason` is an open string (a client reads a name it does not know as another reason). (3) Studio: the dialog
  is offered for a run that has not ended as "Cancel and reset to a point…", fixes its mode when it opens (a lost answer followed by
  the end of the run caused by our own cancellation must not turn the retry into another request), requires the acknowledgement (the
  Studio cannot know whether a step in flight will complete), does not navigate on `PENDING_SOURCE` or `FAILED`, and the list of resets
  of a run refetches on focus and after a reset, never on a timer. (4) CLI: `--cancel-source` on `sagat workflow reset`, sent only when
  given; the answer says in words that the run is being cancelled and the new run is made later, and a retry that finds the entry
  `FAILED` says why and exits 1; `sagat workflow resets <run>` reads the register in words (a state or a reason it does not know is said
  as written). (5) The shared scenario `reset` (`server.resetCancelSource`: the harness turns the setting on in the control plane of the
  scenario and in the engine): the old step that expected a plain `422` for `CANCEL_SOURCE` is gone (D1987's sibling), a run held in
  flight by an activity that ignores the cancellation shows `PENDING_SOURCE`, the same key the same entry, another request under it
  `IDEMPOTENCY_KEY_REUSED`, the source `CANCELLING`, the register and the read by id; then the attempt ends, the source is `CANCELLED`,
  the engine makes the run (the scenario waits for it through the log of the worker that serves it, so that nothing is in flight when
  the scenario ends) and the entry reads `CREATED`; a source that had ended is reset at once; a policy the server does not know is a
  plain `422` (G1981 (2)). A first design that left the attempt in flight when the scenario ended made the six languages disagree on
  the worker log and the state of the run (each stops its workers at its own moment): a scenario ends with nothing in flight.
- **D2003** (2026-11-06, T018-057, `FR-018-044`, `FR-018-046`) Telemetry. The counter `workflow_reset_completions_total{outcome,reason}`
  (`outcome` `created`/`failed`; `reason` `none` or one of the six closed failure reasons; no tenant, run or reset is a label) is
  incremented by the completer after the transaction that created the run or failed the entry has committed, so a step that was
  refused, found nothing to do, lost its lease or was cut off counts nothing (test: `what_the_pass_completes_is_counted_once_it_is_committed_with_closed_labels`,
  which also checks that the label keys are exactly the two). The monitor of the engine adds the gauge `workflow_resets_pending` (the
  entries that wait) and the lag `scheduler_lag_seconds{pass="resets"}` (since when the source of the oldest pending entry has been
  over, 0 when none; a source that is open is not lag: nothing the engine can do). The lag is the stalled signal: the existing alert
  `SagatSchedulerLate` (`max(scheduler_lag_seconds) > 60` for five minutes) covers it, its summary now names the resets, and no new
  alert rule is added (so no promtool test: a rule of the same expression exists and is tested). The runbook lists the three series.
- **D2002** (2026-11-06, T018-057, `FR-018-046`, ADR-0050 decision 9) The setting. `SAGAT_RESET_CANCEL_SOURCE_ENABLED`, off by default,
  `true`/`false`, `yes`/`no` or `1`/`0` (unset or empty is off; anything else is refused naming the variable: the `flag()` of the
  control plane's configuration, which the engine uses too), is read by **both** processes and means the same: the control
  plane accepts a request for an open source only with it, the engine runs the pass only with it; the engine also reads
  `SAGAT_RESET_CANCEL_SOURCE_BATCH` (1 to 1,000, default 100), `SAGAT_RESET_CANCEL_SOURCE_PER_TENANT` (1 to 1,000, default 10, D2006)
  and `SAGAT_RESET_CANCEL_SOURCE_POLL_MS` (default 1,000). The rollout is the engine first and then the control plane (ADR-0079). It is asked **after**
  the receipt of the request is looked at and after the source is seen to be open, so a retry of a request that was accepted is answered
  from the register whatever the setting is now, and a request for a source that has ended is made at once (D1996). Off, an open source
  is `422 RESET_CANCEL_SOURCE_NOT_AVAILABLE` (named reason, sentence in `error.rs` and `errors.md`, nothing cancelled, nothing written).
  Tests: both `config` tests (default, strict parsing, not one of the other switches), the `--help` of both, the service the control plane
  builds from the real configuration (`the_service_the_control_plane_builds_refuses_the_cancel_of_a_source_unless_the_operator_set_it`:
  unset, `false`, `0`, `no` and empty refuse; `true`, `1` and `yes` serve), and a real engine started with the flag that makes the run of
  a request a real API accepted (`cancel_source_rest::an_engine_started_with_the_flag_creates_the_run_of_the_reset_the_api_accepted`).
  The converse (an engine without the flag makes nothing) has no test that is not a wait for nothing to happen (G1992 (2)). With the flag
  off in the engine and on in the control plane, requests are accepted and the entries wait: G1989.
- **D2001** (2026-11-06, T018-057, `FR-018-045`) The quota. `QuotaResource::PendingResets` (`pending_resets`, a count of the namespace, 16 by
  default, hard 64 as the plan's limits table says), the migration `20270108000000` widens the three CHECKs of the ceilings tables. The
  request judges it where it writes the entry: under the advisory lock of the resource, counting the entries `PENDING_SOURCE` of the
  namespace, in the transaction that holds the row of the source (lock order: source run row, quota lock, entry; the receipt is read
  before the row is taken and written after it, so two equal requests meet at the row and the second finds the receipt of the first).
  *Amended 2026-11-06 (review F):* this order is a recorded deviation from the table of ADR-0050 decision 7 (which has the entry, slot 8a,
  before the quota, 8b, and the receipt in slot 2), aligned in ADR-0079 decision 5 and in one line of that table. There is no cycle: the
  quota key of the pending resets is taken by the request path only, always after the row of its source; the completion takes the fence of the
  shard and the row of the entry, never that key and never a source row; two requests for different sources meet only at the key, and whoever holds
  it waits for no other lock. The race for the last place is a test: `two_requests_for_the_last_place_are_judged_one_after_the_other` holds the key
  from a side connection, lets two requests reach it (`pg_stat_activity`, no sleep) and sees one `202` and one `429`, the second source not cancelled.
  A refusal is `429 QUOTA_EXCEEDED` with `details.reason` `PENDING_RESET_LIMIT` and `details.limit`, **before** anything is written: the
  source is not cancelled. A retry of an accepted request is answered whatever the count is. The count falls when an entry is
  completed (tests: `the_namespace_holds_at_most_the_pending_resets_its_limit_allows`, the REST one with the reason, the domain test
  of the resource, `quota_reasons`).
- **D2000** (2026-11-06, T018-057, `FR-018-038`, `O` for the owner) No entry expires, and none can be withdrawn. The conservative default of the
  brief: an entry waits for as long as its source is open (a run in manual review included) and ends `CREATED` or `FAILED`; it is visible
  in the register, in the gauge `workflow_resets_pending` and in the lag once its source has ended; it counts against the limit of the
  namespace while it waits. What an operator does about a source that does not end is what they do about any run that does not end
  (retry, skip or resolve the step: runbook section 4.2). A call that withdraws an entry, or a time after which it fails, is a product
  decision (G1991).
- **D1999** (2026-11-06, T018-057, `FR-018-036`, `FR-018-038`) The acknowledgement is part of the request and is judged again. The request
  needs `acknowledgeReexecution` when the preview on the journal as it is **now** says a step completed after the point would run again, or
  (amended 2026-11-06, D2008) a step after the point is in flight and may complete before the cancellation takes effect: it is then refused
  before anything is cancelled;
  it is stored (`run_resets.acknowledged_reexecution`, immutable like the rest of the request); and the completion judges it on the
  journal the source **ended** with, where the attempt that was in flight when the run was cancelled has usually completed after the
  point: without the acknowledgement the entry is `FAILED` `REEXECUTION_NOT_ACKNOWLEDGED`. That is the case of "the journal changed
  after the request" that a real run produces once the request has acknowledged what it could see (tests:
  `an_entry_whose_source_ended_with_a_step_done_after_the_point_and_no_acknowledgement_fails`, written by hand in the register, with the
  converse `an_acknowledgement_given_with_the_request_covers_the_steps_the_source_completes_afterwards`; the request that would reach it through
  an attempt in flight is refused first, D2008). The Studio always asks
  for it in this mode, the CLI says so when a retry finds the reason, the runbook says when to give it.
- **D1998** (2026-11-06, T018-057, `FR-018-038`, `contracts/errors.md`) The failure reasons. The contract lists `BOUNDARY_NO_LONGER_ELIGIBLE`,
  `ADMISSION_REFUSED`, `BUSINESS_ID_IN_USE`, `CONCURRENCY_PERMIT_UNAVAILABLE`, `RUN_PURGED` and the reasons of `REMATERIALIZE` rules; the
  last three have no producer yet (features 020, 022 and the others are not approved). Three are added, each recorded in the CHECK of
  the table, `errors.md`, the OpenAPI description, the CLI, the Studio and the runbook: `AUTHORITY_REVOKED` (the member who asked is not a
  member, or lost `run:operate`, `run:start` or what the reapply needs, or can no longer see the run), `REEXECUTION_NOT_ACKNOWLEDGED`
  (D1999) and `REAPPLY_TOO_MANY` (the source received more than 256 candidates in the end). The mapping of what a completion can answer to
  a reason is one function, `failure_of`, with a unit test for each arm: a refusal of the reset that is about the point, the prefix, the
  rules or the kind of the source is `BOUNDARY_NO_LONGER_ELIGIBLE`; a quota, a refusal of the start or a workflow that is gone is
  `ADMISSION_REFUSED`; the business identifier held is `BUSINESS_ID_IN_USE`. **Anything else is not an outcome**: storage, wiring, a lost
  lease or an entry completed first is retried (or refused and left alone), never written as the end of the reset (G1990).
- **D1997** (2026-11-06, T018-057, `FR-018-038`, `FR-018-040`, ADR-0050 decision 7) The completion. The pass of the engine (`ResetPass`, the
  shape of the batch pass: the entries of the shards the replica holds, `scheduler_fenced_total{kind="reset"}` for a lost lease, a round
  bounded by the batch size) takes the entries `PENDING_SOURCE` **whose source has ended** (`runs.finished_at IS NOT NULL`: an entry whose
  source is open is not read at all) and for each one, as the **member who asked, with the role they hold now** (`find_membership`),
  does what a fresh reset does on the final journal: the permissions (`run:operate`, `run:start`, `run:signal`/`run:update` for the reapply),
  the point (`plan_reset`), the digest of the prefix (it must equal the stored one), the acknowledgement (D1999), the reapply plan, the
  business identifier (`KEEP` binds the identifier of the source unless another open run holds it: `BUSINESS_ID_IN_USE`) and the run
  quotas; then it creates the run exactly as `reset()` does (`start_with`: prefix, `run.reset`, reapply deliveries, register), in one
  transaction that takes, in the slot of the aggregates that create runs, the **fence of the shard of the entry** and **the entry
  `FOR UPDATE`** (an entry that another engine completed first, or a lease that moved, ends the transaction before anything is written:
  `StateConflict`, left alone), and, in the slot of the memberships, the **membership of the requester `FOR SHARE`** (a revocation that
  returned stops the run). The entry is `UPDATE`d `CREATED` with `new_run_id` in that transaction, guarded by the prefix digest. The
  run's own receipt is the platform's key `reset:{id}` in the scope `run.start:{namespace}` (the prefix `reset:` is reserved, D1995).
  **The engine does not ask the deployment for the reapply** (it has no such setting, D1990): what the control plane accepted at the
  request stands (test: `a_pending_reset_delivers_again_what_the_source_received_though_the_engine_has_no_such_setting`, with a service
  built as the engine builds it). A failure is written under the same fence, with an audit record by the engine.
- **D1996** (2026-11-06, T018-057, `FR-018-038`, ADR-0050 decision 7, ADR idempotency) The request. `reset()` with `sourcePolicy: CANCEL_SOURCE`:
  (1) the authorizations and the justification as for any reset; (2) the source is read, and the **receipt of the key is looked at before
  anything is judged** (scope `run.reset:{namespace}`, the fingerprint holds the policy, the source, the sequence, the digest of the prefix,
  the reapply, the business policy and the acknowledgement): a key that made a request that waits answers its entry, in whatever state it is
  in now, even after the source ended and the point stopped being eligible, whatever the setting; the same key with any other request, a
  request that does not ask to cancel included, is `IDEMPOTENCY_KEY_REUSED`; (3) a source that has ended is reset at once; (4) the setting
  (D2002); (5) the plan on the journal as it is now (the point, the length, the acknowledgement, the reapply plan, with no content read);
  (6) the quota (D2001); (7) one transaction on the row of the source: the receipt is read again under the row (two equal requests meet
  there), a source that ended meanwhile becomes a reset of an ended run (D2005), then the entry (`PENDING_SOURCE`, `CANCEL_SOURCE`, the actor,
  the justification, the acknowledgement, the digest of the prefix), the audit record and the receipt (the result of the receipt is the
  id of the entry), then the ordinary cancellation. The planning reads go through the pool **before** the transaction is opened: a
  transaction that holds a connection never waits for another from the same pool. Crash matrix: `fault_schedules_bulk_operations
  cancel_source` kills the connection after each of the 19 writing statements of a request (nothing changes in any table; the same
  request sent again cancels the source once, makes one entry and one receipt; a third answers the first).
- **D1995** (2026-11-06, T018-056, `FR-018-044`, `FR-018-043`, rule 10) Observability, audit and the key prefixes, after review.
  (1) The counter `workflow_reset_reapply_total{kind,outcome,reason}` (kind `SIGNAL`/`UPDATE`, outcome and reason the closed
  names of the report, reason `none` when there is none) is incremented by `count_reapply` in `finish_start` *after the
  commit*, so a creation that was dropped (a cut connection, a refusal) counts nothing and a retry answered from the register
  counts nothing; the crash matrix of 91 cuts proves it by the totals. It is in the metrics tables of the runbooks
  (`run-reset.md` §7, `observability.md`). The reapplied Updates do not go through the spans and histograms of the API layer
  (G1986). (2) The audit reason of `run.reset` carries the justification of the caller (that is the contract, "audited with it")
  and, when something is to be delivered again, the closed token ` reapply=<POLICY>` before the justification; the count
  delivered is not in it (the audit record is written with the run, before the delivery). A delivery is joined to its reset by
  the trace id of the request, which both carry (G1985). (3) The hint of `RESET_REAPPLY_NOT_AVAILABLE` says "ask the operator of
  the platform to enable reset reapply" and not the name of an environment variable, which a tenant caller has no use for; the
  runbook keeps the variable. (4) The platform-reserved key prefixes are one list in `api-http` (`RESERVED_KEY_PREFIXES`,
  `is_reserved_key`), asked by every validator of a caller's `Idempotency-Key` (the shared one, `chains`, `versioning`) and of an
  update key, and a test reads the sources and fails for a validator that does not.
- **D1994** (2026-11-06, T018-056) Wiring and the transaction, after review. (1) The control plane builds its service of runs in
  one function, `sagat_control_plane::run_service`, and `reapply::the_service_the_control_plane_builds_refuses_the_reapply_unless_the_operator_set_it`
  builds it from the real configuration: unset, `false`, `0`, `no` and empty refuse; `true`, `1` and `yes` serve. The test
  support turns the reapply ON by default so that the suites can exercise it; the comments at both defaults say so and point at
  that test, so a fail-open production default cannot hide behind them. (2) The Update of D1984 can only be refused cleanly
  where the database check and the reducer agree: both accept `StateUpdated` only in `Running` or `Waiting`, so an Update to a
  run that is paused, cancelling, compensating or in manual review is `RunNotMutable` (409), written nowhere (the check is one
  line for all of them); a test pauses a run and cancels another with an attempt in flight, applies an Update-with-Start to
  each, and checks the refusal, the journal and the replay of the projection (compensating and manual review are not set up
  by a test of their own). (3) An Update addressed to another run than the one of the transaction is refused before
  anything is written (it was refused only by the failure of the read-back of D1984, after the write, and would have been
  absorbed into the wrong state had the sequences coincided).
- **D1993** (2026-11-06, T018-056, `FR-018-045`, `NFR`, security review) The reads of a reapply are bounded in statements,
  whatever the number of candidates and the depth of the chain. (1) The chain of resets that made the source is read **once**
  per request, by one recursive statement at most 64 hops deep (`MAX_RESET_HOPS`; it was one walk of up to 1,000 queries for each
  of 256 candidates, outside the transaction, also for the preview); the owner of each row is found in memory; the receipts are
  read in one statement (`unnest`) and the payloads of the signals in another. A row whose owner is beyond the bound is
  `NOT_RETAINED` and reported. A test builds a chain of 65 resets of 200 Updates and previews and resets at 64 hops (all found,
  in under two seconds; the old walk took 9 seconds) and at 65 (all `NOT_RETAINED`), over a store whose pool counts the
  connections it hands out: the preview took 6 and the reset 16, and the test bounds them at 12 and 32, so a walk for each of
  the 200 candidates (200 more, though each is one cheap statement and no clock would see it) fails it. (2) The preview reads the length, not the
  body (`read: false`): it says `WILL_TRY` for what is kept; arguments that are kept and do not parse are `NOT_RETAINED` only
  at the reset (G1987). (3) `candidates` stops at the 257th and keeps its delivery groups in a map. (4) *Nothing deletes
  content between the read and the commit.* The reset reads the payload of a signal and the arguments of an Update before its
  transaction, and the delivery writes the content again (`INSERT … ON CONFLICT DO NOTHING`); an erasure that ran in between
  would put erased content back. Nothing can erase: `payloads` has a guard (`payloads_append_only`, enabled ALWAYS) against
  `UPDATE` and `DELETE`, the runtime role holds neither privilege nor `TRUNCATE`, no function of the schema deletes from it,
  and the events migration says content is never deleted. A test pins all three, so that feature 020's erasure, or any change
  that lets content go, fails it and has to move the read inside the transaction (or check the digest there) first.
- **D1992** (2026-11-06, T018-056, `FR-018-046`) A report is read as it was written, after review. A version that is not this one
  may have written a kind, an outcome or a reason this one does not know (the report is data of the register, and the register
  outlives the binary). Writing stays typed (`ReapplyEntry`); **reading** is `ReportEntry`, whose kind, outcome and reason are the
  strings the register holds, a field it does not know is left out, and a line that is not a line (no sequence, kind or
  outcome) is left out; reading never fails. `GET /resets`, `GET /resets/{id}` and the replay of a reset answer it. The
  OpenAPI does not close `kind`, `outcome` and `reason` (a strict generated client, Java or C#, would refuse a new value
  otherwise); their meanings stay in the description, and the Studio and the CLI show a name they do not know as written. A
  test stores `"reason": "FUTURE"` and an unknown kind and reads it through the three.
- **D1991** (2026-11-06, T018-056, `FR-018-037`, ADR idempotency) The setting is asked where a run would be created, not before
  the replay. A client that retries a reset that succeeded with `reapply: ALL` after the setting was turned off (or on a replica
  where it is off) is answered with the stored entry and its report, as the `Idempotency-Key` promises; a request that was never
  made is refused where the creation transaction is open (`finish_start`), which is dropped, so no run, entry or receipt exists
  (tests count runs and entries). Where the setting is off nothing is planned or read. The preview asks at once.
- **D1990** (2026-11-06, T018-056, `FR-018-046`, ADR-0032 decision 8) The engine has no setting. The engine builds its
  definitions with `DefinitionCache`, which parses stored versions with `parse_stored`, and `parse_stored` reads a version
  that declares `reapplyOnReset` whatever the capability says (`crates/infra-ir/tests/reapply_on_reset_enabled.rs`); it
  publishes nothing and makes no reset. The variable, its configuration test, its `--help` lines and the call of
  `set_reset_reapply` are removed from the engine, and the harness of the SDK conformance does not pass it. D1986, G1977's
  closure and the runbook are corrected.
- **D1989** (2026-11-06, T018-056, `FR-018-039`, D1982 (3)) **Defect found by review; the rule of "one delivery" is the
  journal's, not the clock's.** All the rows a transaction writes have one `occurred_at`, so two equal signals that a reset
  delivered one after the other in the transaction that created its run looked like one delivery to a reset of that reset, and
  the second was dropped without a word. A row joins the delivery of an earlier row (same name, payload and instant) only when
  its step was **already waiting when that earlier row was written** (its `wait.started` has a smaller sequence than the first
  row of the delivery): a delivery wakes everything that is waiting for the name at that moment, and a step that began to wait
  after the first row was woken by another. When the beginning of the wait cannot be found the rows are two: nothing is dropped
  without being reported. Tests: the unit tests of the rule (consecutive waits are two signals, parallel waits one, a missing
  beginning is two) and `a_reset_of_a_reset_keeps_two_equal_signals_that_the_first_reset_delivered_in_one_transaction`
  (red first: the second signal was dropped).
- **D1988** (2026-11-06, T018-056, T018-059, T018-069, `FR-018-039`, `FR-018-047`) The Studio and the CLI show the reapply.
  (1) The Studio dialog offers, once the points are listed, the four policies in words (nothing, the signals, the updates whose
  arguments were kept, both), nothing being the default and not sent. Choosing a policy reads the plan again with it (the
  acknowledgement of the steps that run again is about the steps and stays); the plan lists what would be delivered in the order
  of the journal, `WILL_TRY` or `NOT_REAPPLIED` with its reason in words. A reset that delivered something does not take the
  person away at once: the dialog says what came of each, with a button to open the new run (the report is read nowhere else
  at that moment); a reset that delivered nothing goes to the new run as before. The register under the source lists the
  report of each reset (`Delivered again (N)`). (2) The CLI takes `--reapply none|signals|updates|all` (the API's names in
  lower case are accepted) on `reset-preview` and `reset`, sends it only when given (an older server never sees the field), and
  prints each entry as `signal NAME (event N): outcome: reason in words`. Both say a name and a place and never what was sent:
  the API does not return a payload, and a control character in a name is not passed to the terminal. A reason neither knows
  is shown as the platform wrote it, so a later version can add one.
- **D1987** (2026-11-06, T018-056, `FR-018-039`, `FR-018-048`) The shared scenario `reset` follows the new truth. The server of
  the scenario runs with `SAGAT_RESET_REAPPLY_ENABLED` (`server.resetReapply`; the harness switches the capability on in its own
  process, which is the scenario's control plane, and tells the engine by the flag). The two steps that expected a `422` for
  `SIGNALS` now see it served: `preview-signals` lists nothing (the run received nothing) and `reapply-refused` becomes
  `reapply-unknown-policy`, which keeps the refusal that is still one (`SOMETIMES`, a plain `422`, G1981 (2)). A new workflow
  `signalled` (`a`, a wait for `payment.confirmed`, `b`) is signalled; the preview lists the signal `WILL_TRY`; the reset
  delivers it again and its report says `REAPPLIED`; the same key answers the same report; the reset to the start finds nothing
  waiting for the signal and reports `REJECTED` `NOT_WAITING`.
- **D1986** (2026-11-06, T018-056, `FR-018-039`, `FR-018-046`, G1977; *corrected 2026-11-06 after review, D1990, D1991*) The
  reapply is a deployment setting of the control plane, `SAGAT_RESET_REAPPLY_ENABLED` (off by default; `true`/`false`, a
  mistyped value is refused naming the variable), listed in its `--help`. (1) It sets `RunService::with_reset_reapply` (through
  `run_service`, D1994) and, when on, the process capability `sagat_infra_ir::set_reset_reapply(true)`, so a workflow may
  declare `reapplyOnReset` (D1974) where the reset that applies it again is served. The capability is only ever switched on
  by a binary (a process that starts with the flag off leaves it as it is), because it is a global of the process. The
  engine has no setting: it reads the versions it was given with `parse_stored`, which does not look at the capability
  (ADR-0032 decision 8), and a reset is made by the control plane in its own transaction (D1990). (2) With the flag off a
  *preview* with `reapply` other than `NONE` is `422` with `details.reason` `RESET_REAPPLY_NOT_AVAILABLE` at once (it was a plain
  `422`: this closes G1981 (3) for the reapply; `CANCEL_SOURCE` stays a plain `422`), and so is a *reset that would create a
  run*; a retry of a reset that succeeded is answered from the register whatever the setting is now (D1991). With it on they
  are served. (3) Workflows published before follow ADR-0032 decision 8: a version published while the capability was on is
  read whatever the flag says now (`parse_stored`), keeps retaining the arguments of its Updates, and its retained rows are
  reapplied as soon as the reset is served; a version published before the capability existed has no `reapplyOnReset`, its
  Updates say `retained: false` (or, if older than D1975, nothing) and are not candidates (`LEGACY_EVENT` for the rows that
  say nothing). Turning the flag off does not erase anything. Replicas of the control plane can disagree during a rollout
  (G1988).
- **D1985** (2026-11-06, T018-056, `FR-018-039`) The report and the preview have one closed shape, `ReapplyEntry` of the OpenAPI
  (additive: `ResetPlan.reapply` and `Reset.reapplyReport` were arrays of free objects that were always empty):
  `{sequence, kind: SIGNAL|UPDATE, name|operation, outcome, reason?}`; a signal has a `name`, an Update an `operation`; `sequence`
  is the row of the source journal; `outcome` is `WILL_TRY` (preview only), `REAPPLIED`, `REJECTED` or `NOT_REAPPLIED`; `reason` is
  one of `LEGACY_EVENT`, `NOT_RETAINED`, `PREVIOUS_SIGNAL_NOT_DELIVERED` (not delivered), `NOT_WAITING`, `UPDATE_REJECTED`,
  `REVISION_CONFLICT`, `UPDATE_NOT_ACCEPTED` (rejected). It carries no payload, no argument and no digest of either: they are
  content of the tenant (`FR-018-042`), and a test reads the stored report for the content of every signal and the arguments
  of every Update. It is stored as `jsonb` in `run_resets.reapply_report` by the same statement that writes the entry, in the
  transaction that creates the run, so a replay of the request answers the same list. The register bounds it at 256 entries and
  65,536 bytes; the longest entry the closed names allow (a 128-character name, the longest reason, a sequence of eight digits)
  is 253 bytes as `jsonb` writes it, so 256 of them are 64,768: the bound in bytes cannot be reached before the one in entries
  (a unit test computes it). The preview lists the same candidates from the same function, so for the same source and point it
  cannot list others than the report; what only the delivery can know (a step that waits, a validator) it leaves as `WILL_TRY`
  (and G1987). The names are read as they are stored (D1992).
- **D1984** (2026-11-06, T018-056, `FR-018-039`) **Defect found by the first test of the reapply:** an Update applied in a
  transaction that goes on afterwards left the in-memory state of that transaction behind its journal. `apply_declared_update`
  writes the `state.updated` row, the receipt and the run's version straight to the database (it was built for Update-with-Start,
  which commits right after), so the next write of the same transaction numbered its events from a sequence the Update had taken
  (`Internal`, every reset that interleaved an Update and a signal). `PgRunTx::apply_declared_update` now reads back the row the
  Update wrote and applies it to the state and to the sequence it persists from (`absorb_written_update`), the way a replay would
  take it; an Update that finds the sequence elsewhere is an error, not a guess. Update-with-Start, whose transaction ends there,
  is unchanged (`update_with_start` suites pass).
- **D1983** (2026-11-06, T018-056, `FR-018-039`, `FR-018-044`, `FR-018-050`) The delivery. (1) In the creation transaction, after
  `run.reset` and before the entry of the register is written (it carries the report), in the order of the source journal, each
  candidate on its own: a signal through the same `deliver_signal` as `signalRun` and Signal-with-Start, recorded `via: signal`
  (so a reset of the new run may deliver it again), an Update through `apply_declared_update` as Update-with-Start does, with the
  caller as the actor, so each is audited (`run.signalled`, `run.updated`), journaled and announced as any other. The reset
  itself is one `run.reset` audit record. (2) The key of a candidate is `reset:{reset id}:{sequence of the row}` (the reset id is
  chosen before the delivery and is the id of the entry): stable for the reset, distinct for every candidate, and the prefix
  `reset:` is reserved as `batch:` and `schedule:` are, for the `Idempotency-Key` of every call and for the update key of
  `updateRun`/`updateWithStart` (`400`, `INVALID_UPDATE_KEY`), so a caller cannot take a key the platform will use. A retry of the
  request is not a second delivery because the creation is answered from the receipt of the first (a crash test cuts the
  transaction after each of its writing statements, 91 of them in the test). (3) A signal nobody waits for (`NOT_WAITING`) is `REJECTED` and ends the signals.
  So does any signal that cannot be delivered at all (`LEGACY_EVENT`, `NOT_RETAINED`), and this is known before the transaction,
  so the preview says it: every later signal is `PREVIOUS_SIGNAL_NOT_DELIVERED`, because delivering it would reach the wait the
  earlier one was meant for (two signals of one name). A signal that already has a reason of its own keeps it. Updates are not
  affected by that stop. (4) An Update is applied only to the state it was applied to in the source: the request carries
  `expectedStateRevision` = the `before_revision` of the row. A rejected one therefore makes the Updates that followed it
  `REVISION_CONFLICT`, which is what they are: they were made for a state that the new run does not have. The validator declining
  is `UPDATE_REJECTED`; a declaration that no longer accepts the arguments or the state they make (`InvalidArguments`,
  `EvaluationFailed`, `ValueTooLarge`, `StateSchemaChanged`, `UnsupportedDisclosure`, the `422` of `updateRun`) is
  `UPDATE_NOT_ACCEPTED`. (5) Anything else, storage, wiring, a receipt that contradicts itself, a quota, aborts the whole
  creation: nothing of the run exists and the caller is told it failed (the conservative choice: a reset that cannot say what it
  did is not made). (6) The caller needs `run:signal` for `SIGNALS` and `ALL` and `run:update` for `UPDATES` and `ALL`, asked
  before the source is read (the role matrix gives both to every role that has `run:operate`, G1982).
- **D1982** (2026-11-06, T018-056, `FR-018-039`, G1978) What a reset delivers again, computed from the source journal after the
  point `N`, in journal order, by one function used by the preview and the reset (`crates/application/src/runs/reapply.rs`).
  (1) A **signal** is a `wait.satisfied` of a root step with `via: signal`, named by the `eventType` of that node's wait in the
  pinned definition; its payload is the one the row names (the output of the step it woke). `via: event` and
  `via: signal_with_start` are not candidates (G1978 (2): a reset must not deliver again what the caller asked for in one
  request that also started the run, nor what an event did); a row with no `via` is `NOT_REAPPLIED` `LEGACY_EVENT`; a payload
  that is not named or no longer kept is `NOT_RETAINED`. Several waits one delivery woke are one signal, otherwise the second would
  find nobody waiting and end the signals for nothing; *the rule is D1989* (it was "the same name, payload and instant", which
  merged two real signals a reset had delivered in one transaction). A wait below the root is not a candidate (G1983). (2) An **Update** is a `state.updated` with `retained: true`; its operation is the
  row's, its arguments those of the receipt of the run that applied it, which for a row a reset restored (`sequence <= reset_sequence`)
  is found by walking `rerun_of` through the resets that made the run (G1978 (1); the chain is read once and is bounded at 64 hops, D1993): a reset of a reset
  delivers the Updates of the first source. `retained: false` is not a candidate; no marker is `LEGACY_EVENT`; arguments that
  are gone or do not parse are `NOT_RETAINED`. (3) **At most 256 candidates**, the skipped ones included (the report lists
  them): more is `422` `RESET_REAPPLY_TOO_MANY` for the preview and for the reset, and nothing is dropped. (4) A source with a row
  this version cannot read is refused `RESET_RULE_MISSING` (with the type) when something is to be delivered again: the rows
  after the point may hold what is to be delivered and the first unreadable one hides the rest. (5) `sourcePolicy: CANCEL_SOURCE`,
  an open source and a scoped prefix are as before.
- **D1981** (2026-11-06, T018-062, T018-063, `FR-018-048`) The client surface of feature 018 in the SDKs and its nine
  conformance scenarios. (1) The surface is the table of `contracts/sdk-conformance.md` and `sdks/SPEC.md` §3.1: three groups
  on the client (`schedules`, `batch`, `businessIds`) and, on the client, `start` with the new options, `signalWithStart`,
  `updateWithStart` and the five reset calls. (2) `start` answers whether it created the run (`started`, from the `202`
  against the `200` of `USE_EXISTING`) and a `Run` carries `businessId` and `startDelayedUntil`; a call that uses none of
  the new options sends what it sent before. (3) A business identifier is sent as given and never checked by the SDK
  (the server's `BUSINESS_ID_INVALID` is what every language reads); only the lookup refuses locally what cannot be a path
  segment. (4) A start delay is a duration converted once, before the first attempt, to integer milliseconds by truncation
  toward zero; the SDK refuses only what is not a number and leaves the range to the server. (5) The scenarios assert each
  `details.reason` in the scenario of the operation that raises it, so `errors-018.json` of the draft is not written; the
  concurrency-permit reset is not a scenario because the permit is not built. (6) What a scenario cannot compare is made
  comparable by the runner and not by hiding fields: instants that depend on the day are `<time>`, the ones a fixed spec
  determines (January 2099) are epoch milliseconds, and the runs of a batch are named by the step that started them because
  the order the server lists them in is not a promise. (7) The engine's schedule and batch passes are off until told to run,
  so a scenario that waits on them says `"server": {"schedules": true, "batches": true}` and the harness sets
  `SAGAT_SCHEDULES_ENABLED` and `SAGAT_BATCHES_ENABLED`; without it the scenario would only time out. (8) Found by the
  scenarios and fixed in the server: the actions a backfill makes never carried `backfillId` (nothing wrote the column;
  commit `5cba87e0`). (9) Found by the C# port: a business identifier of exactly `.` or `..` cannot be looked up by path
  (every URL resolver, also for `%2E`, takes a whole dot segment as a step, so `GET …/business-ids/..` reaches
  `…/` and not the identifier), so `businessIds.get` refuses those two locally in every SDK; the identifiers that merely
  contain dots stay valid. An identifier like that can still be used in a start and found through `listRuns?businessId=`.
- **D1979** (2026-11-06, T018-054, T018-052, `FR-018-036`, `FR-018-044`) The five reset routes and two amendments.
  (1) `listResetPoints`, `previewReset`, `resetRun`, `listRunResets` and `getRunReset` (`x-implemented: '018'`) are
  served from `crates/api-http/src/routes/resets.rs` straight from `RunService` (no new use case trait), behind the
  admission of the run's namespace: `ResetInspect` (`run:operate` and `run:read`) for the points and the preview,
  `RunReset` (the permissions of a rerun) for the reset, `run:read` for the register. The points page by a cursor
  that is the sequence of the last one served, `limit` outside 1 to 200 is clamped like every list, the reset needs the
  `Idempotency-Key` and answers `202` with the entry of the register. `CANCEL_SOURCE` and a `reapply` other than
  `NONE` are `422` in words (T018-056 and T018-057 are not built). (2) **Defect in D1976 (4), found by the
  contract test of the preview:** `reset_plan` listed as restored every step that existed in the replayed state of
  the prefix, and at the start of a run the entry step exists (pending) before it has run, so a reset to the start
  listed it as kept and left it out of the steps that run again: a source whose first step had completed needed no
  `acknowledgeReexecution`. A step is restored only when it has a result (a terminal status); the others are the
  new run's to do, and if the source ran them after the point they are in `reexecuted` with `completedInSource`. The
  domain test accepted `all(node == "a")` for the start and hid it; it now asserts an empty `restored`, the three steps
  in `reexecuted` and the acknowledgement. (3) Two sentences of `contracts/errors.md` are amended, and the API's
  closed table with them: `RESET_NOT_SUPPORTED_FOR_SOURCE` also covers a run of a database execution, and
  `RESET_SOURCE_NOT_TERMINAL` no longer sends the caller to `CANCEL_SOURCE`, which this version does not offer.
- **D1978** (2026-11-05, T018-054, `FR-018-036`, `FR-018-037`) The register of resets is the table `run_resets`
  (migration `20270107000000`, additive, the shape of `data-model.md`), and the API names a reset by its id.
  (1) The entry is written by `RunTx::register_reset` in the transaction that creates the run, after the prefix is
  restored and `run.reset` recorded, so a run made by a reset always has its entry and a refused reset leaves none;
  it is `CREATED` with `new_run_id`, `completed_at = created_at`, `source_policy = REQUIRE_TERMINAL`, and keeps the
  point, the digest of the prefix (the one `run.reset` carries), `reapply`, the business policy, the subject that
  asked and the justification (content of the tenant, charged by the `ALWAYS` quota triggers). `PENDING_SOURCE` and
  `FAILED` exist in the CHECKs for `CANCEL_SOURCE` (T018-057) and nothing writes them. (2) `RunService::reset` answers
  `ResetMade { reset: ResetRecord, run: RunRecord }`; a retry of the same key is answered with the entry of the
  first request (`reset_made` finds it by the new run). `resets` (newest first by the time-ordered id, at most 200)
  and `reset_record` need `run:read` and a source the caller can see; another tenant, another source run and an id
  nobody made are all `NotFound`, and the store filters by tenant and source on its own. (3) A trigger fixes what a
  reset is (run, point, digest, policies, actor, justification, time) and a completed entry stays as it completed;
  `DELETE` is refused for a pending one and the runtime role has no `DELETE` at all (no purge function yet: the
  entry lives as long as the runs it names, and no code deletes runs). `reapply_report` is `[]`: nothing is
  reapplied until T018-056. (4) The foreign keys to `runs` have no cascade, like `rerun_of`.
- **D1977** (2026-11-05, T018-053, T018-054, `FR-018-037`, `FR-018-038`) The store and the service of the reset are
  `crates/application/src/runs/reset.rs` (`reset_points`, `reset_preview`, `reset`), `crates/infra-postgres/src/execution/resets.rs`
  (`read_source`, `restore`) and the `persist` of `crates/infra-postgres/src/execution.rs`. (1) The new run is created by
  `start_with` as a rerun of the source (`rerun_of`), with `runs.reset_sequence = N` (migration `20270106000000`, additive,
  immutable once set, a `down` that refuses while any run carries it; the check is the data model's with `N >= 1` added); the restored prefix is written in the creating
  transaction, after the run row and before the settle that the domain's `record_reset` does, so no reader sees the run
  with a half prefix. (2) A restored event is a **verbatim copy** of its source row: `event_type`, `detail`, `actor`,
  `occurred_at`, `trace_id` and `payload_digest` are the source's (`Effects.restored` carries the rows, `persist` binds
  them instead of the transaction's own actor and clock), so a reader of the new run's journal sees who did each thing and
  when, and the first row adds `detail.restored = true` and `reset_of` (source run and sequence) to say where the history
  came from. (3) A restored row has **none of the side effects of the first time**: `persist` takes the row, collects its
  sagas and goes on (`continue`), so no `task.ready` is announced for a step that completed before the point and no signal
  is delivered again. The `task.ready` of the first step the new run releases is the only announcement. (The telemetry
  that skips the restored prefix is T018-058 and is not built: a metric derived from the journal would count it.)
  (4) The projections the events would have written are rebuilt from the same prefix: the outputs of the steps it
  completed are given to the new run's steps (same digest, same content), and the declared state is restored to the last
  revision in the prefix (`restore_declared_state`), so the run goes on from the point with the state it had there. (5)
  The service refuses before it writes, with typed reasons (`AppError::ResetRefused { reason, event_type }`, which the API
  answers as `422 VALIDATION_FAILED` with `details.reason` (and `details.eventType`), except `RESET_SOURCE_NOT_TERMINAL`,
  `409 CONFLICT`, as `contracts/errors.md` says):
  `RESET_NOT_SUPPORTED_FOR_SOURCE` (a child run, a run of a database execution, a continuation chain),
  `RESET_SOURCE_NOT_TERMINAL`, `RESET_POINT_NOT_ELIGIBLE` (not one of `reset_points`, which includes a prefix with a
  scoped event, `ResetBlock::ScopedEvents`), `RESET_PREFIX_TOO_LONG` (more than `RESET_PREFIX_EVENTS`, 5,000 events),
  `RESET_RULE_MISSING` (an event type this version has no rule for) and `RESET_REEXECUTION_NOT_ACKNOWLEDGED` (a step that
  completed after the point declares an effect and the caller did not acknowledge it). A missing or malformed
  justification and a `reapply` other than `NONE` are `NotAllowed`, not a reset reason. The request's fingerprint holds the
  marker `run.reset` and the sequence, so the same key and point is the same reset (the first run is returned) and the key
  of another point is not a retry. (6) v1 restores the **root** events only, `REQUIRE_TERMINAL` sources and
  `reapply = NONE`; G1980 lists the rest.
- **D1976** (2026-11-05, T018-006, T018-052, `FR-018-035`, `FR-018-036`) The domain half of the reset is
  `crates/domain/src/execution/reset.rs`. (1) The registry of D1605 is one `registry!` table of the 66 event types of the
  journal (`run.reset` included): an exhaustive `match` (`rule_of`), so a new event type without a rule does not compile,
  and the same rows as a table by the name the journal stores (`RULES`, `rule_by_name`), the way a type written by a
  version this one does not know is looked up (`RESET_RULE_MISSING` is the store's answer, T018-054). The test
  `every_event_type_of_the_journal_has_exactly_one_rule` reads the names out of the source of `RunEvent::name()` and fails
  when a name has no row or a row has no name; `the_rules_are_the_ones_of_the_contract` is typed from the contract, not
  from the table. Of the 66, 52 are `COPY`, 12 `EXCLUDE` and 2 `REFUSE` (`reconciliation.requested` until
  `reconciliation.concluded`, `run.start_delayed` until `run.start_released`); none is `REMATERIALIZE` yet, because the
  side tables it re-creates belong to features that are not merged. (2) `run.reset` is `RunEvent::RunReset { source_run_id,
  source_sequence, prefix_digest, reapply }` with the closed detail of the contract (`source_run_id`, `source_sequence`,
  `prefix_sha256`, `reapply` as `NONE|SIGNALS|UPDATES|ALL`), and the reducer accepts it only after the restored prefix of a
  run that has started, is open and is not stopping, with a prefix of at least one event. (3) `reset_points` implements D1022
  over the addressed journal: the candidates are the root events `run.started`, `step.completed`, `child.finished`,
  `collection.finished` and `loop.finished`; one is blocked by an `EXCLUDE` event or an open `REFUSE` window in its prefix,
  by a state that is not quiet (the run not plainly `RUNNING`, paused or stopping; a step open; a collection or loop that
  did not succeed), and by a later compensation, skip, resolution or reconciliation of an instance of its prefix. At
  `run.started` the entry step has not been released yet, which is the clean start and not work in flight. (4) `reset_plan`
  lists the steps restored (flagging approvals) and the declared steps the source ran after the point, each with the effects
  it declares (`compensateWith`, `reconcileWith`, `irreversible`, `highRisk`) and whether it completed; any that did means the
  caller must acknowledge the re-execution.
- **D1975** (2026-11-05, T018-043, `FR-018-039`) The journal says what satisfied a wait and whether an Update kept
  its arguments, and the arguments are kept on the receipt. (1) `wait.satisfied` carries `detail.via`
  (`RunEvent::WaitSatisfied { node, via: Option<WaitVia> }`): `signal` for `signalRun` and a chain signal, `event` for
  the dispatch of a published event (the one caller of the scoped event wait commands), `signal_with_start` for
  Signal-with-Start whether it created the run or found an open holder (the route decides, as the contract's table
  says); an older row has none and a name this version does not know decodes as none, so a later version cannot make
  an older reader lose a row. (2) The contract of the draft put the retained arguments in the `payload_digest` of the
  `state.updated` row, but that column of the row already holds the new state (the foreign key that keeps the state
  referenced), so the arguments cannot go there. They are referenced from the receipt of the Update: the new nullable
  column `operation_update_receipts.arguments_digest` (migration `20270105000000`, a foreign key to `payloads`, a SHA-256,
  only on an `APPLIED` receipt, whose `down` refuses while any is set). The receipt is written in the same transaction as
  the row, goes with the run (`ON DELETE CASCADE`) and is found by `(resolved_run_id, run_version)`, the sequence of the
  row. (3) `state.updated` carries `detail.retained` (`RunEvent::StateUpdated { .., arguments_retained: Option<bool> }`):
  `true` when the declaration opts in and the arguments were kept, `false` when it does not, none in a row written before.
  That is what lets a reset report `NOT_RETAINED` (an Update that did not opt in) apart from `LEGACY_EVENT` (written before
  the journal said so, or by a binary that did not know), which the receipt alone could not. (4) The evaluator decides:
  `DeclaredUpdateDecision::Applied { .., arguments }` has the canonical arguments only when the declaration opts in, so an
  Update that does not pays no payload write; the write is a `payloads` row, charged to `tenant_content_bytes` by the
  trigger that charges every payload, deduplicated by digest (the same arguments applied twice are one row). The size is
  within the ceiling by construction: the arguments of an Update are bounded by `MAX_OPERATION_VALUE_BYTES`, as a retained
  value is.
- **D1974** (2026-11-05, T018-078, `FR-018-039`, `FR-018-046`) `reapplyOnReset` on an Update declaration is an optional
  boolean of the additive IR (ADR-0032), merged into `$defs/update` of `workflow.schema.json` from
  `contracts/workflow-ir.fragment.schema.json`. The Update declaration lives in the IR codec
  (`DeclaredUpdateDefinition`, `crates/infra-ir/src/declared_state.rs`), not in the domain crate the task named: the
  domain holds no declared state, so `crates/domain/src/workflow.rs` is untouched (the task text is amended, as D300
  did). The codec reads the field (absent is `false`; a value that is not a boolean is a schema error) and the digest of
  the declaration ignores a default that is written, so a document made before the field keeps the digest it had
  (pinned in `crates/infra-ir/tests/reapply_on_reset.rs`) and `false` is the same declaration as saying nothing. The
  capability is the process's: `sagat_infra_ir::set_reset_reapply`, off by default. While it is off, a new publication
  (`IrCodec::parse`, and `declared_state_publication`, which produces what a publication stores) is refused with
  `UnsupportedDeclaration { field: "reapplyOnReset" }`, which the API answers as `422 UNSUPPORTED_NODE`; `parse_stored`,
  `parse_stored_declared_state` and `parse_declared_state` read a version that was published while it was on, so closing
  the capability never erases what was published under it (ADR-0032 decision 8). It is a switch of the process and not a
  field of `JsonIrCodec` because that type is a unit struct built at about four hundred places. Six SDKs carry
  `reapplyOnReset` on their `UpdateDefinition`, written only when true; the scenario `state-operation-builders` has the
  workflow `retained-update` and its golden holds the document, which the six builders write alike.
- **D1973** (2026-11-05, T018-033, `FR-018-042`) The input of a schedule is disclosed by `POST …/schedules/{scheduleId}/input-disclosures`
  (`discloseScheduleInput`), through the policy of feature 016 and not a copy of it: `ServiceEvaluator` of
  `payload_disclosure.rs` takes the permission that reads the owner of the payload (`schedule:read` here, `run:read` for a
  run) and the labels of the answer from a `DisclosureSubject`, and everything else, the audiences, the reveal grant, the
  projection, the structural fallback, is the same code. The store (`ScheduleInputStore`, `PgScheduleStore` in
  `payload_disclosure.rs`) reads the head revision of the schedule and the version its action would use now (the one it pins,
  or the workflow's `active_version_id`) in one transaction, rechecks the role and the reveal grant there, calls the
  evaluator under the open transaction and appends the audit before it commits. The task's contract named
  `PayloadDisclosureNode` as the response; the answer is `ScheduleInputDisclosure`, the envelope of `PayloadDisclosure` with
  `scheduleId` and `revision` in place of the run's identifiers (the caller and the result variants are the same), because a
  bare tree could not say which levels the caller saw or why a value is withheld. Audit: `schedule.input_disclosed` (collapsing
  identical reads within five minutes, as the action was defined) for what an internal reader or an inspector got, and
  `payload.pii_revealed` on the schedule for a reveal of personal data, which must never collapse with another; a refused reveal
  is `payload.reveal_denied`. A reader who is shown only public values leaves no record, as for a run. The request is at
  most 1 KiB and every response, a refusal for size included, is `private, no-store`. A deleted schedule, one of another
  namespace and one of another tenant are the same `404`.
- **D1972** (2026-11-05, T018-047, `FR-018-030`, `FR-018-031`) The `UPDATE` action of a batch applies the declared Update
  to each run through the use case Update-with-Start uses for an existing run (`update_in_own_transaction`: begin on the
  run under its lock, `apply_declared_update`, commit), as the creator with the role they hold now. The key of the item,
  `batch:{job}:{run}` (80 printable ASCII characters), is the update key, so a repeat of an item answers from the receipt it
  left and the Update is applied once; another job is another key and applies again. An applied Update is `SUCCEEDED`; a
  rule that says no (the validator) is `FAILED` with `VALIDATION_FAILED`, as `updateRun` answers it (a stale revision,
  `CONFLICT`, cannot happen: a batch carries no expected revision); an operation the run's version does not declare is
  `SKIPPED` `OPERATION_NOT_DECLARED`; a run that ended `ALREADY_TERMINAL`; one that handed off `RUN_CONTINUED`; the other
  refusals fail the item with the `Error.code` `updateRun` would answer (`declared_update_code`). The name the job gives
  must be a name an operation can have (`BATCH_ACTION_PARAMETERS` at `/update/name`): found by a test that created a job
  with the name `two words`, which would have failed every item as `INTERNAL`. The engine's batch run service holds the
  evaluator (`with_declared_updates`). `REPIN` stays refused without feature 019.
  *Amended 2026-11-05 after review:* the audit record of an Update a batch job made says so, `;batch=<job id>` after the
  outcome (an identifier, never free text; `check_audit` still refuses a free-text reason), so the up to 100,000 changes of
  a job can be attributed to it. A run that cannot take an Update now is `SKIPPED ALREADY_TERMINAL` only if it has ended,
  `NOT_APPLICABLE` while it is winding down, compensating or waiting for a person; a failure of the platform (a pool or lock
  timeout) defers the item as the other actions do, and a creator who lost the right ends the job (`AUTHORITY_REVOKED`)
  instead of failing item by item; a run that vanished is `NOT_FOUND`, not an undeclared operation.
- **D1971** (2026-11-05, T018-044, `FR-018-043`) The remediation of a reason of this feature is a row of the closed table
  `ErrorCode::remediation` (`crates/api-http/src/error.rs`) whose sentence is the one in the feature's own
  `contracts/errors.md`; the task also names `docs/quickstarts/errors.md`, which documents the problems the definition
  codec returns (two tests keep it equal to the help of the CLI) and has no place for the reason of an API error, so it is
  not touched. `contract_schedules_bulk_operations errors::` keeps the three apart from drifting: each reason an
  operation raises (29 today) has exactly the table's sentence, a row of its own and a mention in the `Error.details`
  description of the OpenAPI; each reason the table lists that nothing raises is on a named list (what it waits for), has
  no row of its own, and the test fails when a row of the table is on neither list. The ceilings of schedules, backfills
  and batch jobs now say which they are: a 429 at one of them carries `details.reason` (`SCHEDULE_LIMIT`,
  `BACKFILL_CONCURRENCY`, `BATCH_LIMIT`) next to `details.limit`, chosen from the label the refusing store passes (a
  resource's `limit_name`, or `schedule_backfills_per_schedule`); the other limits are refused as they always were.
- **D1955** (2026-11-04, T018-028) The actions of a schedule are purged by a bounded function, visiting schedules in
  turn. Nothing deleted them: an hourly schedule writes 8,700 rows a year and one every minute 525,000. FR-018-013
  keeps "at least the last 1,000 or 30 days": an action goes when it is older than 30 days **and** beyond the
  newest 1,000 of its schedule, and a `BUFFERED` action never goes (the guard of the table refuses it as well).
  The runtime role has no `DELETE` on the table, so retention is `schedule_purge_actions(schedules, rows)`
  (`SECURITY DEFINER`, fixed `search_path`, migration `20261230002000_schedule_action_purge`), called by the
  housekeeping of the scheduler a few schedules at a time and at most 5,000 rows per call. A rule of "oldest rows
  first" would starve: a schedule with fewer than 1,000 actions keeps all of them however old, and its old rows
  would be the first candidates of every call, for ever. The function therefore visits schedules in the order of
  `schedules.actions_purged_at` (never visited first) and marks one visited only when its purgeable rows are
  gone, so a schedule cut short by the row budget is the first to be looked at again and one with nothing to purge
  is looked at once per cycle. The cut is the time of the thousandth newest action, found through the index that
  already serves the recent-actions read.

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
| G1015 | `listSchedules` filters by `state`, `origin`, `workflowId` and the start of a `name` (done 2026-11-05, without a new index: a namespace holds at most as many schedules as its `schedules` quota, which bounds the scan over the head revisions, and the order and the cursor are those of the unfiltered listing, so a page ends where the next begins whichever part filters it). The response `action` still carries `inputDigest` but not `inputBytes`. | An index per filter column (`schedules_by_workflow`, `schedules_by_state`) is a migration for a scan the quota already bounds; it is worth adding if a namespace's ceiling is raised far above the default. | Open (`inputBytes` only) |
| G1018 | The crash-at-each-write matrix of T018-020 kills the transaction that starts a run (28 writes) and the one that records a skipped time (3 writes). The other decision-only steps (buffer, coalesce, drop, reconcile, auto-pause, the backfill step) have the same shape (fence, compare-and-swap, action row, new state in one transaction) but are not killed at each write; no test boots the control plane with schedules enabled and a skewed clock. | Extend the matrix with the `Step` enum of `tests/fault/schedules-bulk-operations/firing.rs` when a task touches those steps; add a control-plane wiring test with T018-018's remaining work. | Open |
| G1607 | A binary older than the instance-capability registry release does not register, so no activation can see it; activations therefore require the registry release deployed fleet-wide first (operator precondition, checked by nothing but the runbook), and rollback below a recorded requirement is unsupported. | Inherent to introducing the registry; 029's prerequisite release (T029-003) is the vehicle. | Accepted |
| G1605 | Draft lock orders that deviated from ADR-0050 decision 7 on 2026-10-27 (026 T2 membership and rate buckets after the link row; 030 endpoint head before memberships; 021 asset heads after secrets and the variable fence as an "authority slot"). Resolved the same day: 021 and 026 now follow decision 7 exactly (021 D1624, 026 D1337 revised); 030 follows it with one listed exception, a caller transaction locking its own `START` row after its run (030 D1660), recorded in decision 7. | Owners 021, 026, 030. | Resolved |
| G1606 | `REMATERIALIZE` checks of other features (retired version, paused resource, retired snapshot, busy permit) are evaluated at reset time; a preview reports them as provisional and a reset can still fail on them. | Inherent to reset at a later time. | Accepted |
| G1950 | A run keeps no record of whose authority it started under. The 017 admission of a database step claimed after the schedule was resumed by someone else (or after `authorizedBy` changed through an update) stands on the new `authorizedBy`, not on the member the run started under (D1950). | Fails closed when that member lacks database authority (the step is refused, the run is not corrupted), and open when the new one has it and the old one did not. Closing it needs a column on `runs` (an additive migration with the erasure-aware immutability trigger of the provenance columns) or a different rule for the claim. | Open |
| G1951 | The count of a recovery's summary stops after 100 calls of the calendar (a million times: a year of a schedule every 30 s, the densest the spacing floor and the longest window allow). Past it the step records a partial `COALESCED` summary and the recovery is remembered with no actions left, so the next step goes on summarizing instead of acting again; the work per step is bounded, the number of steps is not. | A schedule that dense with a window that long and an outage that long is not a case anyone sized for; the steps are cheap (one small transaction each) and the pass takes `steps_per_schedule` of them per round. Revisit with the benchmark (T018-072). | Open |
| G1953 | A waiting action keeps waiting however long the runs that hold it back stay open (a run in `MANUAL_REVIEW` after a failed compensation can stay open for days) and starts when they end, even if it is older than the catch-up window; `data-model.md` has it become `MISSED_CATCHUP_WINDOW` instead. | Starting a stale time is the safe direction (the work is not lost) and the action is visible as `BUFFERED`; the transition needs an update of an action row under the append-only guard. | Open |
| G1954 | Only the actions are purged. The revisions of a schedule (append-only: a trigger refuses every `UPDATE` and `DELETE`, and they carry the content the tenant is charged for), the obligations that are `DONE`, the finished backfills and the tombstone of a deleted schedule are kept for ever; `data-model.md` has the tombstone go 30 days after the last purge of its actions. | They are small next to the actions (a row per edit, per debt, per backfill), but a tenant that edits a big schedule often keeps charging its content quota with revisions nobody reads. Needs a guard that lets a purge function through, and a rule for the revision an action still names. | Open |
| G1955 | The input of a schedule is not validated against the `inputSchema` of the target version at write or at action (`SCHEDULE_INPUT_INVALID` means "not an object" or "too large"), and `nominalTimeField` is not checked to be a declared string property (`NOMINAL_TIME_FIELD_INVALID` is never answered). | The IR declares `inputSchema` but the platform enforces it nowhere (run start, signal, worker), so enforcing it for schedules alone would refuse what the same workflow accepts from a person. Close it with the run start: a validator port (`infra-schema::validate` exists) used by both, and the contract `errors.md` already has the two reasons. | Open |
| G1956 | A batch job names root steps only (`RETRY_STEP`, `MARK_MANUAL_REVIEW`): a step of an iteration of a collection or a loop needs its scope path resolved per run, which the single-run `review_instance` takes as an exact instance and the job has no way to express across runs. A request with an `instancePath` is `BATCH_PARAMS_INVALID`. | Resolving a path per run (the iteration that waits is not the same in every run) is a design of its own. | Open |
| G1957 | A transaction that began before a batch job was created and takes its identity only after it can commit a run whose `created_at` is before the job's bound after the selection read, and the run is not selected (D1958). | It is concurrent with the creation of the job, which commits first, so the run is not "before" the job in any order a caller can observe; the window is the milliseconds between reading the clock and the first write of a run creation. Closing it needs the selection to look at the oldest transaction that has not ended, not only at identities. | Open |
| G1958 | The wait of an item for the lock of its run is not bounded to two seconds (`LOCK_WAIT` is never produced): the run service opens a run with `Lock::Wait`. A long transaction on one run delays the job's pass at that item. | Run transactions are short; a bounded wait needs a lock mode in the run store that gives up and an error to carry it. | Open |
| G1959 | The `UPDATE` action of a batch is refused at creation: it needs the declared-operations evaluation of a run (its receipt, `reapplyOnReset`, the per-run update key), which no use case offers to a caller that is not the HTTP route. | A use case of the application layer for a declared Update, shared by the route and the batch. | **Closed 2026-11-05 (D1972):** the use case exists (`update_in_own_transaction`, the one Update-with-Start uses) and the `UPDATE` action is built. |
| G1960 | `cancelBatchOperation` requires an `Idempotency-Key` and a `reason` (8 to 2,000 characters) but neither is kept: asking twice is asking once by the nature of the operation (the second answer is the job being cancelled), and the reason is checked and not stored, because the audit record names the action and never free text (D1957). | A cancellation reason in the audit trail needs a closed vocabulary or a redaction rule; the key would need a replay store (as G1014 for schedule writes). | Open |
| G1961 | **The effect on a run and the record of its item are two transactions, which ADR-0050 decision 2 and FR-018-030 say they should not be.** `ensure_live` narrows the window to the duration of the effect, it does not close it: a replica that stalls after the look, a lease that lapses, or a cancel that commits in that window still lets one more effect through, and its record is then refused; a crash between the effect and its record followed by a cancel marks the item `CANCELLED` although the run was changed. Every effect is keyed, so a repeat changes nothing and the counters always equal the item rows, but an item can say less than what happened to its run. | Closing it means the single-run use cases take a hook that locks the fence, the job and the item inside the run transaction (ADR-0050 slot order: fence, run, job, item) and records the outcome there; that touches `operate`, `signal`, `review` and `rerun`, and ADR-0050 would then be followed instead of amended. The owner decides (O1951). | Open |
| G1962 | The selection compares `runs.created_at`, stamped by the application clock of the host that created the run, with the bound read from the database clock. A host whose clock is ahead of the database's by δ stamps runs that are older than the job with a time after its bound, and the job never selects them; a host that is behind has runs created after the job selected. Not the window of G1957 (one host, between reading its clock and its first write). | Stamping `created_at` from the database in the run insert, or selecting by a column the database orders (commit order), is a change to run creation, which is not this feature's. Hosts of one deployment are expected to be within milliseconds. | Open |
| G1963 | The wait for the transactions that were open when a job was created has no deadline, and it is database-wide: one forgotten transaction or prepared transaction keeps every `SELECTING` job there and the active-job place each holds until it ends or the job is cancelled (a cancel does not wait for the horizon). `SELECTION_FAILED` exists and is produced by nothing. | A deadline that fails the job is a product choice (how long is too long, and who is told). | Open |
| G1964 | `businessId: DROP` of `RERUN` is accepted and stored but the adapter does not read it: business ids are not bound to runs yet, so a rerun has none to keep or to drop. | It becomes a promise the code must keep when business ids land (US03). | Open |
| G1965 | The control plane and the engine each have `SAGAT_BATCHES_ENABLED`. With it on in the control plane and off in the engine, jobs are accepted and never run, they stay `SELECTING` and hold their active-job places, and a cancellation waits for an engine to drain it. Turning the flag off in the control plane stops the routes, not the jobs already running; the engine's flag is the switch. | One flag per process kind is how schedules are gated too; the instance-capability registry of ADR-0050 decision 9 would refuse the mismatch and is not built. | Open |
| G1966 | The Studio's backfill dialog has no preview of how many times the range holds: no route counts them without starting the backfill, so it states the bound (1,000 times, an end that is not in the future) and shows the platform's refusal in words. | A preview is a read of the schedule's spec over a range, which the platform alone computes (the Studio computes no time); it needs a route (`previewScheduleBackfill`) with the contract, the reference and the types. | Open |
| G1967 | The reuse window of an identifier is the default (30 days) for every namespace: `ReuseWindow::default()` is frozen into each binding and no setting changes it. FR-018-020 makes it namespace-configurable up to 366 days. | A namespace setting is a table or a column of its own (and a Studio form); the binding already freezes the window it was made under, so adding the setting changes nothing that exists. | Open |
| G1968 | `business_ids_purge(p_rows)` removes the bindings whose holder closed and whose window passed, or whose holder is gone. Housekeeping now calls it (2026-11-05, `MaintenanceStore::purge_business_ids`, up to 5,000 rows a round, at most 20 rounds), in the pass that purges the actions of schedules. It has no setting of its own: a binding holds nothing once its window has passed, so there is nothing to tune. | A binding that holds nothing is only a row, so the purge is an economy, not a rule; it runs with the other purges. | Closed |
| G1969 | The Studio does not use the lookup of a binding (D1967): the run list filters by the whole identifier and by its prefix, which finds the same run. The lookup (holder, open, last status, reusable after) and the rerun's `KEEP`/`DROP` (D1966) are built. | A screen that shows `reusableAfter` or the chain behind an identifier is a design choice for the owner, not a gap in the contract. | Open (Studio only) |
| G1970 | The plain `continue_as_new` of the chain store (the prepared handoff, used by tests; production uses `continue_evaluated`) copies the identifier to the successor too, and no test of it checks that. | Its callers are the unit tests of the continuation and the prepared-handoff integration test; the production path is under `a_chain_holds_the_identifier_until_it_ends_and_every_segment_carries_it` and a mutation that drops the copy fails it. | Open |
| G1971 | The privacy check covers logs at every level, the fields of every span (created and closed, 2026-11-05), metrics, error texts and every table; what is not searched is the OTLP wire conversion of a span, which carries the same fields. `BusinessId`'s `Debug` hides the text, so a span field would show `BusinessId(..)`. | A real OTLP export canary (SC-018-014) belongs with the observability gates of T018-071. | Open (export only) |
| G1972 | A declared Update on a delayed run (FR-018-023: "declared Updates are admitted") has no test: the fixtures of the declared operations create their runs with `NewRun` and a retained state, not through a start with options. Nothing in the reducer reads the hold when it applies an update, so the clause is believed, not proved. | A start with a delay and a declared initial state needs its own fixture; it belongs with Update-with-Start (T018-041), which needs the same one. | **Closed 2026-11-05 (D1970):** `update_with_start::a_delayed_run_admits_the_update` starts a held run with a declared state, applies an Update to it, and shows the run still held, `WAITING`, replay equal to the journal. |
| G1973 | Three cases of Signal-with-Start (D1969) are not proved. (1) *Closed 2026-11-05 (D1969, amended):* a request served by an open holder now keeps the receipt of the request with the signal, so a retry after the holder closed is answered by the holder. (It was: the same key started a new run and signaled it.) (2) A new run that 022's run-level permit would queue instead of start: 022 is not approved, so the rollback of that case (FR-018-024) has no test and no code. (3) The race of a wait's timeout against the delivery at creation: the wait has a timer, but a run is created and signalled in one transaction, so the timer cannot fire between them; no test pins that with a clock at the boundary. | (1) is a retry after the first request succeeded and the world moved on; the same shape exists for `signalRun` and the answer is truthful about what happens now. (2) waits for the owner's approval of 022. (3) needs a wait whose timeout is the creation instant. | Open |
| G1975 | The table of reasons lists two for operations that are built and that the API words otherwise: `NOMINAL_TIME_FIELD_INVALID` (a `nominalTimeField` that is not a top-level string property of the input schema is `SCHEDULE_SPEC_INVALID` with the field's pointer) and `SCHEDULE_DELETED` (a deleted schedule is `NOT_FOUND`, as one that never existed, which is what the contract's own preamble asks of a resource the caller cannot see). The rest that nothing raises waits on a feature (029, 031 `BINARY_CAPABILITY_MISSING`; US06 `PENDING_RESET_LIMIT` and the `RESET_*` reasons; 020 `RUN_PURGED`; 022 `CONCURRENCY_*`). | Changing either would give a caller more than the preamble allows or break a documented answer; the table should say what the API does. The contract test lists them as not raised, with the reason, so they are not forgotten. | Open (amend the table when T018-057/US06 touch it) |
| G1976 | Three smaller points of the review of the with-start and batch `UPDATE` slices are left. (1) The route of Signal-with-Start has no operation telemetry of its own (the signal route does not either), and an Update a batch job applies is counted in no metric because the batch runs in the engine, where the API's telemetry does not: a job of 100,000 Updates is invisible in `workflow_operation_duration_seconds`. (2) The contract says the payload of a signal is an object; `signalWithStart` accepts any JSON, as `signalRun` does, and trims the name before it joins the fingerprint (a batch job refuses the names `valid_signal_name` refuses). (3) The holder of a business identifier is not compared with the request's `workflowId`: a request naming workflow A whose identifier is held by a run of workflow B in the same namespace acts on B, as `startRun` with `USE_EXISTING` already does. | (1) belongs to T018-071 (observability gates), which already lists the batch metrics; the rest are choices the owner may want to make once, for the whole family of signal and start routes, not per route. | Open |
| G1995 | Two suspicions of the review, checked. (1) *`due_resets` selects `runs.finished_at IS NOT NULL` while the completion asks `status.is_terminal()`*: they are the same set, because the CHECK `runs_finished_consistent` ties `finished_at` to the seven terminal statuses (`SUCCEEDED`, `FAILED`, `CANCELLED`, `TIMED_OUT`, `COMPENSATED`, `PARTIAL`, `COMPENSATION_FAILED`), exactly `RunStatus::is_terminal()`; not a gap. (2) *The foreign keys `run_resets -> runs` (the source and the run made) have no `ON DELETE`*: a retention job that deletes a run is blocked by **any** entry that names it, and the guard trigger refuses to delete an entry that waits at all. Nothing deletes runs today (no `DELETE FROM runs` in the code), so nothing is blocked; the work that does (the retention of ADR-0054, and T018-051 that purges the register) must delete or archive the entries first, and a source that never ends keeps its run for as long as the entry waits. | Neither is a defect of this task; (2) is a constraint the retention work must read. | Open (T018-051, ADR-0054 retention) |
| G1994 | The preview does not say which step is in flight. The domain knows (`ReexecutedStep.in_flight`, D2008) but the body of `previewReset` has only `completedInSource`; a client that sees `requiresAcknowledgement: true` and no step with `completedInSource: true` can only conclude that a step is in flight (the CLI and the Studio say so in words). Naming the step is an additive field of the contract, of the Studio types and of the goldens of six SDKs. | A field in the contract is a decision for the whole family of reset routes, and the acknowledgement does not need it. | Open |
| G1993 | The request of `CANCEL_SOURCE` takes its locks in an order that differs from the table of ADR-0050 decision 7 (D2001, ADR-0079 decision 5), and no slot-order barrier harness runs over it: T018-012, which builds that harness for the admission paths, stays open. What is tested instead: the crash matrix of the request (19 writes), the race of two requests for the last place, two equal requests at the row of the source, and a requester revoked while the run is admitted. | The harness is a task of its own (T018-012) and runs over every path of the drafts; one path does not justify building it. | Open (T018-012) |
| G1992 | Three things about the tests of `CANCEL_SOURCE` are weaker than the clause. (1) *The point becomes ineligible after the request* cannot be produced by a run the platform makes: the prefix is immutable, and the clause "compensated, skipped, resolved or reconciled later" cannot be reached by a journal the reducer accepts (G1979 (2)). The re-check is proved by entries written by hand with a wrong digest and a sequence that is not a point (they fail `BOUNDARY_NO_LONGER_ELIGIBLE`), and the change of the journal that a real run does produce, a step that completes after the point, is proved end to end (`REEXECUTION_NOT_ACKNOWLEDGED`, D1999). (2) *Closed in part 2026-11-06 (D2009):* that an engine **without** the flag makes nothing is now a test of a pure function (`optional_passes` names the reset pass only for the flag, and the settings of the pass alone do not turn it on), and the engine starts exactly the passes it names; what stays unproved is the observation itself (nothing happens), which a test can only wait for, and the positive wiring is tested with a real engine. (3) The shared scenario in the six SDKs proves the client surface and, with a real engine, that the run is made; the cases of the quota, of the setting off and of a stuck source are not in it (a scenario cannot change the server between steps). | (1) needs a feature that appends compensation to the journal of a point already passed; (2) is the nature of the observation; (3) the scenario file is one server. | Open |
| G1991 | An entry that waits cannot be withdrawn, and does not expire (D2000). Everything that follows from that: (1) a source that never ends (a run in manual review nobody looks at) holds one of the namespace's places (16 by default) for as long as it lives; (2) the person who asked has no call that gives the place up, and **an administrator has none either**; (3) **revoking the requester does not free the place**: the engine fails the entry (`AUTHORITY_REVOKED`) only when the source has ended, so the one lever on an entry that waits is to end the source (retry, skip or resolve the step), or to remove the member and wait for it to end; (4) the limit can be raised, but only up to the hard ceiling of 64 (`quota_ceilings`), so a namespace in which sources are stuck can be filled for good; (5) the customer's run was cancelled by the request and stays cancelled whatever becomes of the entry. | A call that withdraws an entry, or an expiry (and with what outcome, and whose audit), is a product decision; the brief's conservative default is neither. The runbook (section 4.2) says the lever there is. The owner decides. | Open (owner) |
| G1990 | A completion that fails for a reason that is not an outcome (storage, a wiring that is missing, an error the mapping does not know, a conflict nobody foresaw) leaves the entry pending and is tried again, with a warning in the log naming the tenant and the reset, for as long as it fails. *Head of the line (review A, D2006):* the scan is oldest first, so such an entry would be first in every round and starve the younger ones; three things bound it: at most `SAGAT_RESET_CANCEL_SOURCE_PER_TENANT` entries (10) of any one tenant are taken per round, the pass leaves an entry that failed out of the scan until its time (2 s, doubling to 5 minutes, at most 1,024 remembered), and only the two known conflicts are taken for races. What stays: the memory is the process's, so a restart tries every entry once; the entry has no durable attempt counter and is never given up (`FAILED` with which reason is a decision, and the batch items have a bounded retry; the schedule backfill step has the same gap, G1952); a tenant with more than `per_tenant` failing entries still occupies its whole share every round, which cannot take the share of another. | An attempt counter and a `next_attempt_at` are a column of the register and a decision about when an entry is given up; the engine's log, `workflow_resets_pending` and `scheduler_lag_seconds{pass="resets"}` show it. | Open |
| G1989 | The setting is read by each process (ADR-0079). **With it on in the control plane and off in every engine the request still cancels the customer's run and answers `202`, and nothing replaces the run until an engine's flag is turned on**: nothing is lost (the entry waits in the register, counted against the limit), `scheduler_lag_seconds{pass="resets"}` grows once the sources have ended, and `SagatSchedulerLate` fires, but the cancellation cannot be taken back in the meantime. The converse (on in an engine and off in the control plane) makes the entries that exist and refuses new requests. The rollout order (engine first, control plane first to turn off) is in ADR-0079 and in the runbooks of schedules, batch operations and resets (section 4.2); `no_shipped_manifest_turns_the_reset_switch_on` keeps the repository's own manifests off. | The instance-capability registry of ADR-0050 decision 9 would refuse the mismatch and is not built (T018-080; the same as G1965 and G1988). | Open |
| G1988 | The setting is read by each replica of the control plane. During a rollout, or when an operator sets it on some replicas only, a request can be served by one and refused `RESET_REAPPLY_NOT_AVAILABLE` by another; a retry of a reset that succeeded is answered everywhere (D1991). | The instance-capability registry of ADR-0050 decision 9 would refuse the mismatch and is not built (the same as G1965). | Open |
| G1987 | The preview and the report can differ in two ways that are not the setting: the preview reads the length of what is kept, not the body, so arguments that are kept and do not parse are `WILL_TRY` in the preview and `NOT_RETAINED` in the report; and the preview cannot know whether a step will be waiting or a validator will agree. | Reading the bodies of up to 256 payloads to say the first is the cost the security review asked to avoid; the second is the nature of a preview (D1985). | Open |
| G1986 | The Updates a reset delivers again do not go through `OperationTelemetry` of the API layer (the `workflow.update` span, `workflow_operation_duration_seconds{kind="update"}` and `workflow_primitive_rejections_total`): they are applied inside the creation transaction by the application layer, which has no span or histogram facility for them. What exists is `workflow_reset_reapply_total{kind,outcome,reason}` (D1995), and the journal events, which the telemetry projector turns into spans like any other. | A span and a histogram per delivery would be created inside a transaction that may be dropped; the counter is emitted after the commit. | Open |
| G1985 | What a reset audits. The audit reason of `run.reset` carries the justification of the caller: that is the contract ("audited with it"), and not a leak. When something is to be delivered again it also carries the closed token ` reapply=<POLICY>` (D1995). The deliveries are audited by what they are (`run.signalled`, reason `signal '<name>'`; `run.updated`, reason `operation=<name>;outcome=<…>`) and share the **trace id** of the request with the reset, which is the exact join between them; they do not name the reset or the policy, and the reset does not say how many were delivered (the audit record of the run is written with the run, before the delivery). | The reason of a delivery is the one every other path of `deliver_signal` and of the Update writes; the trace id is already there for the join. | Open |
| G1984 | `UPDATE_REJECTED` (the validator declines) and `UPDATE_NOT_ACCEPTED` (the declaration does not accept the arguments) are proved end to end only with data put there by the test: arguments the validator declines, and arguments the schema does not accept, are kept in the place of those of an Update. The pinned version accepted what the source retained, and a validator that reads only the arguments and the state cannot decline at the same revision what it accepted, so no run the platform makes can produce either of them at a reapply. | Both need a definition that depends on something other than its arguments and its state; the declared-operation language has none. The wiring (a refusal is a rejection of that entry, the next candidate still goes) is the same code whatever made the refusal. | Open |
| G1983 | Two limits of the candidates. (1) A wait below the root of the source (an iteration of a collection or a loop) is not a candidate, and is not reported: `deliver_signal` reaches the root steps only, as `signalRun` does. (2) *Closed 2026-11-06 (D1989):* waits woken by one delivery are told by the journal (the step was already waiting), not by the clock. | (1) follows G1980 (1): the scoped side tables are not restored, so nothing a scope held can be delivered to. | Open (1) |
| G1982 | The extra checks of `run:signal` (for `SIGNALS`, `ALL`) and `run:update` (for `UPDATES`, `ALL`) before a reset cannot be reached by a test through the role matrix: the roles that hold `run:operate` and `run:start` (`operator`, `admin`) hold both. A mutant that removes them survives. A fitness function (`a_role_that_may_reset_a_run_may_deliver_again_what_the_reset_reapplies`, `crates/domain/tests/schedule_permissions.rs`) fails for a role that grants the first two and not the others, and for a role that is not listed, so that the gap cannot widen unseen. Also: an Update rechecks the authority of the actor in the transaction (`run:update` is read from the membership there), and a signal does not (the same as `signalRun` and the with-start requests: the permission is the one of the principal of the request). | They are defence for a role that grants the first two and not the others (a custom role); a reset delivers as the caller. | Open |
| G1981 | Several refusals reach the client as a plain `422` with no `details.reason`, though the contract promises one or the code could give one: (1) a negative `startDelayMs` (the body type is unsigned, so the JSON is refused before the range is checked, where `0` and `31622400001` are `START_DELAY_INVALID`); (2) an overlap policy or a reapply policy the server does not know (`WHENEVER`, `SOMETIMES`); (3) *Closed 2026-11-06 (D1996, D2002):* `sourcePolicy` `CANCEL_SOURCE` is served, and on a deployment that does not serve it for an open source it has its reason, `RESET_CANCEL_SOURCE_NOT_AVAILABLE` (a `sourcePolicy` the server does not know is still a plain `422`, pinned by the scenario `reset`); (4) a `reason` under 8 characters on `resetRun`. The scenarios pin each refusal as it is (a negative delay is left out of them, since pinning a defect is not a test of the contract). | A reason is what lets a client tell two 422s apart without parsing a sentence; an enum the deserializer rejects never reaches the code that names reasons. Closure: map the serde rejection of these fields to the reason of its field (`START_DELAY_INVALID` with `details.field`, `RESET_REAPPLY_NOT_AVAILABLE`, `RESET_SOURCE_POLICY_NOT_AVAILABLE`, `RESET_REASON_INVALID`) in `crates/api-http`, with a contract test, and then bless the goldens of `start-delay` and `reset` after reading the diff; breaking nothing, since only `details` gains a field. | open |
| G1980 | Four limits of the first reset store (D1977) are not settled. (1) A prefix that holds a scoped event (an event of a child scope, a parallel branch, a collection item) is blocked (`ResetBlock::ScopedEvents`, answered `RESET_POINT_NOT_ELIGIBLE`): restoring it needs the side tables of those scopes (`REMATERIALIZE` in D1605) and none exists yet. Points after the first scoped event of a run therefore cannot be reset to, and `reset_points` says so for them. (2) The ceiling `RESET_PREFIX_EVENTS` is a constant of the application, not a setting of the deployment or the namespace: a longer prefix is refused, not truncated. (3) Only `REQUIRE_TERMINAL` sources and `reapply = NONE` exist: `CANCEL_SOURCE` (T018-057) and `SIGNALS`/`UPDATES`/`ALL` (T018-056) are not built. (4) *Closed 2026-11-06 (D1978, D1979):* the `run_resets` register, its reads and the five routes exist. (5) A run of a
database execution is refused as a source (`origin` is set), and no test shows it: a trigger ties `origin` to an internal
workflow, so a test cannot make such a run with SQL, and the ad-hoc database runner is not in this suite's fixtures. | (1) needs the scoped side tables, which belong to the features that own them; refusing is the honest answer and the refusal names the event type. (2) A setting is a product choice (how large a history may be copied in one transaction) that the owner decides with the namespace limits of 022. (3) and (4) are the next slices of US06 and each has its own task. | Open (T018-054, T018-056, T018-057) |
| G1979 | Three things about the reset points of D1976 are not settled. (1) By D1022 a loop that succeeds is not a point: it ends at its last checkpoint (`loop.iteration_checkpointed` that completes it) and `loop.finished` is journaled only for a loop that failed or was cancelled; `timer.fired`, `wait.satisfied` and the decision of an approval complete a root node too and are not in the list. The literal list is implemented, which is the conservative reading: those completions are not offered, and the next completion after them is. (2) The clause "a prefix instance compensated, skipped, resolved or reconciled later" cannot be reached by a journal the reducer accepts: a compensation exists only inside a saga, whose node is parked while its body runs, so every point before a compensated instance is already refused for a waiting step. The clause is implemented and its predicate is unit-tested, but no end-to-end journal exercises its wiring. (3) The corpus of `tests/reset.rs` has no approval run and no loop run, for (1): their completions are not candidates; approval decisions are covered where real approval flows exist (T018-055). | (1) widening the points is a product choice (more places to reset to) and the owner decides it; (2) is a defence for event shapes a later feature may add. | Open (owner, for 1) |
| G1978 | Two consequences of D1975 for the reset are not settled. (1) The retained arguments are on the receipt of the run that applied the Update. A reset re-emits the prefix of the source with its `retained: true` markers under a new run, so a reset of that reset run finds `retained: true` rows whose arguments sit on the receipts of the source: the reset must follow `reset_of` to the source's receipts, or copy the references. (2) A Signal-with-Start whose signal reaches an open holder is recorded `signal_with_start`, as the contract's table says, so a reset with `SIGNALS` (which re-delivers `via: signal`) does not deliver it again, although it is a signal to an open run; delivering it would make the reset re-signal what the caller asked for in one request that also may have started a run. | Both belong to the reset (T018-053, T018-054, T018-056): the rows are written now so the reset can choose; changing what `via` says later would need a rewrite of history. The reset reports a row it cannot reapply (`NOT_REAPPLIED`) with a reason. | **Closed 2026-11-06 (D1982):** (1) the arguments are found by following the resets that made the run; (2) a `signal_with_start` row is not delivered again. |
| G1977 | The `reapplyOnReset` capability (D1974) has no setting of a deployment: `set_reset_reapply` is called by no binary, so a workflow that opts in cannot be published anywhere yet. And the CLI's `sagat check`, which has no deployment, accepts a document that the server will refuse. | Wiring it before the reset exists would let workflows opt in whose arguments nothing retains or reads; the setting arrives with the reset flag of US06 (T018-056, T018-057), where the retention (T018-043) and the reapply are proved together. A client cannot know the capabilities of a server (ADR-0032, consequences), and the refusal names the field. | **Closed 2026-11-06 (D1986, D1990):** `SAGAT_RESET_REAPPLY_ENABLED` in the control plane (the engine has none: it reads stored versions with `parse_stored`); `sagat check` still has no deployment and accepts what a server without the flag refuses. |
| G1974 | Two cases of Update-with-Start (D1970) are not proved or are weaker than the clause. (1) With an open holder the Update runs in a transaction after the claim of the identifier has ended, not under it: if the holder closes in between, the answer is `409 RUN_NOT_MUTABLE`, not a new run. (2) A holder that is a continuation chain is served in the segment that is current when the claim ends (`current_segment`); if that run hands off to its successor between that read and the effect, the answer is `409 RUN_CONTINUED` or `NOT_WAITING`. And a new run that 022's permit would queue is not rolled back, for the reason of G1973 (2). | (1) and (2) are races the caller sees as a typed 409 that a retry resolves, and they are the answers `updateRun` gives to the same race; closing them needs the Update inside the claim's transaction for an existing holder too, which the binding lock order (ADR-0050) allows and a later slice may do. | Open |
| G1952 | A backfill step that the admission refuses (a full quota, no published version) waits and is tried again on every round of the pass, with no backoff, unlike a scheduled time (D1952). | A round is one poll of the database per owned shard and the step is one rolled-back transaction; harmless at the default poll but wasteful. Fix with a `retry_at` on the backfill (a migration). | Open |

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
- **O1951 — Must the effect on a run and the record of its batch item be one transaction?** (a) Keep the two keyed
  transactions with the look before the effect (D1960, G1961): a stale replica or a cancel can let one more keyed effect
  through, and an item can say less than happened to its run; or (b) pass a hook into the four single-run use cases so
  the fence, the job and the item are locked and the outcome recorded inside the run transaction, as ADR-0050 decision 2
  says. **Recommended: (b) before the feature is released; (a) is what is built** and is safe against a repeat, not
  against one extra effect after a cancel.
- **O1950 — Who may write a `CANCEL_OTHER` schedule.** (a) whoever holds `run:operate` (admin, operator; of the roles
  that write schedules only admin), because the schedule cancels runs for them; (b) whoever holds `schedule:write`
  (admin, developer), the schedule cancelling as the platform. **Recommended and implemented: (a)** — a developer who
  may not cancel a run by hand would otherwise cancel any run of a workflow by writing a schedule (D1953).
