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
- **D2009** (2026-11-06, T018-057, review E, ADR-0081) Which optional passes the engine runs is a pure function of its configuration,
  `settings::optional_passes`, which returns the names `schedules`, `batches` and `resets` in the order the engine starts them; `run()` starts
  exactly those. The reset pass is in the list only when `SAGAT_RESET_CANCEL_SOURCE_ENABLED` is on, and its settings alone do not turn it
  on. Tests: `the_pass_that_completes_resets_is_absent_unless_the_operator_turned_it_on` (closes G1992 (2)),
  `no_shipped_manifest_turns_the_reset_switch_on`. ADR-0081 records that the per-process flags stand in for the instance-capability
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
  and `SAGAT_RESET_CANCEL_SOURCE_POLL_MS` (default 1,000). The rollout is the engine first and then the control plane (ADR-0081). It is asked **after**
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
  before the quota, 8b, and the receipt in slot 2), aligned in ADR-0081 decision 5 and in one line of that table. There is no cycle: the
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
  *Amended 2026-11-09 (OD-2026-11-08-1, built as D2059 to D2068):* **the second half of that last sentence is decided and built.** The owner
  delegated it ("Faça isso por mim") and it was taken the conservative way: an entry still `PENDING_SOURCE` 24 hours after its request (the
  engine's setting `SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS` may make it 1 hour to 7 days) ends `FAILED` with the closed reason
  `SOURCE_DID_NOT_END`, no run is made and its place is freed. **What stays as decided on 2026-11-06:** an entry whose source is open (manual
  review included) is not completed, the source is never touched by the expiry, and **no call withdraws an entry**: the operator's levers are
  to end the source or to let the entry expire. The withdrawal remains a later decision (G1991, G2018). Whether the whole feature may be on
  by default stays the owner's (G1989); the flag stays off.
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

- **D2013** (2026-11-08, T018-080, `FR-018-052`) The activation is audited in a table of its own, not in `audit_records`.
  `audit_records.tenant_id` is `NOT NULL` and references a tenant; an activation concerns the deployment and no tenant, so a
  row there would need a made-up tenant. `platform_requirement_history` (action `ACTIVATED`/`REMOVED`, capability, kinds,
  actor, epoch, time) is written by the two operator functions in the transaction that records or removes the requirement, is
  append-only (`ENABLE ALWAYS` triggers, truncate refused) and is not readable by the runtime role. Every token in it is from a
  closed set or an operator subject of at most 255 characters; no free text of a caller. The `down` migration refuses while it
  holds a row, so rolling back never destroys the evidence of an activation (tests: `instances_migration`).
- **D2014** (2026-11-08, T018-080; reworded 2026-11-08 after review) Only the operator can record or remove a requirement, so the
  functions that do are not executable by the runtime role (`platform_requirement_lock`, `_activate`, `_remove`; `REVOKE ... FROM
  PUBLIC`, no grant to `sagat_app`). The command runs with the schema-owning role, as `provision-tenant` does (G260 precedent), and the
  runtime-privilege audit lists the three `EXECUTE` needs in `ALLOWED` with the reason (test:
  `the_runtime_role_reads_the_registry_and_calls_three_functions_only`). **What this closes, and what it does not:** a compromised
  API or engine process (it holds the runtime role) cannot *record a requirement*, which would make the fleet refuse to serve. It
  can still, with the functions it may call, register rows of its own up to the limit, hold the registration lock open and fail a
  victim's heartbeat by holding the victim's row lock; it cannot refresh, delete or rewrite another process's row (D2024). That
  residual is G2000: the database cannot close what the role legitimately does, and the claim of this decision is the first
  sentence only.
- **D2015** (2026-11-08, T018-080) The guards. The two tables are guarded by triggers that are `ENABLE ALWAYS`: a row of
  `platform_requirements` is inserted or deleted (never updated) and the epoch is moved (by exactly one, never inserted or
  deleted) only while a transaction-local flag that the two operator functions set is on; and a constraint trigger, deferred to
  the commit, refuses a transaction that changed a requirement without the epoch having moved in it (`changed_xact`, the
  `xid8` of the transaction that last moved it, compared with `pg_current_xact_id()`, which is the top-level transaction
  whatever savepoints it used). The flag is not a secret: the owner can set it. The point is that a hand edit cannot forget the
  epoch (processes would never learn of the change) and that nobody does it by accident (tests:
  `requirements_and_the_epoch_change_only_through_the_activation_functions`, the `session_replication_role = replica` case in
  `the_epoch_is_one_row_and_the_history_and_the_epoch_cannot_be_rewritten_or_emptied`).
- **D2016** (2026-11-08, T018-080) Registration. A refusal (`CAPABILITY_MISSING`, `FULL`) writes no row, so a process that is refused and asks again every second, or a crash loop
  of an old build, does not fill the table. The count of the table is made under an advisory lock taken after the epoch lock, so the limit
  of 1,000 is exact for new ids under concurrent registrations (the check that the id is new is made before the lock: a second
  registration of the same id racing itself is not a case, ids are random per start) (test: `registrations_counted_at_the_same_time_never_exceed_the_limit`).
  **Deviation from the data model:** "a full table refuses registration" is refused only when 1,000 rows are **live**; at the
  limit the rows no longer live (silent for 30 seconds) give way, oldest first, to the registration. A rule of a day kept
  would let a process that restarts every ten seconds (a crash loop) fill the table in under three hours and lock every healthy
  restart of the fleet out (test: `a_table_full_of_silent_rows_makes_room_and_a_crash_loop_cannot_lock_the_fleet_out`). A row
  that exists registers again at the limit (the heartbeat found it gone and the process registers anew).
- **D2017** (2026-11-08, T018-080) The epoch check is carried by the heartbeat, not by a query of its own. The brief left the
  choice between one small query per second and the heartbeat. A process compares the epoch that its heartbeat returns (one
  statement) with the one its gate was decided under, and reads the requirements (one statement, one snapshot, with the epoch)
  only when they differ: at most one extra statement per process per 10 s, and none when nothing changed. The bound on what a
  process may do after an activation is therefore one heartbeat period (10 s, jittered up to 12 s) plus the call; and since an
  activation is refused while a live process of the kinds lacks the capability, only a process that was not live (silent for
  30 s) can be in that window. A shorter loop would only move the exposure of a process that has lost the database anyway. The
  API's admission and the streams read one atomic (`ServingGate::is_open`); a stream ends with a `reconnect` event.
- **D2018** (2026-11-08, T018-080; amended 2026-11-08 after review) The fail mode, stated. (1) The registry cannot be asked at the
  start, or refuses the process: the process does not serve (its gate is closed: every call but liveness is `503`, readiness fails,
  no pass runs, no connector is read, the worker API is `UNAVAILABLE`) and **does not exit**; it asks again every second until the
  registry lets it serve (`REGISTRY_UNREACHABLE`, `BINARY_CAPABILITY_MISSING` in the log and in the counter). Exiting would have
  broken the contract of both runbooks and of `an_instance_pointed_at_an_unmigrated_database_is_alive_but_not_ready_until_migrated`.
  A table of 1,000 live rows is **not** a refusal (D2025): the check of capabilities has passed, so the process serves, loudly, and
  is not counted. (2) A heartbeat fails while it serves: logged with the instance id, tried again at the next beat, never fatal;
  after 30 s of silence the line is an error, and **the failure decides** (D2026): a plain connection failure
  (`StoreError::DatabaseUnavailable`) leaves the process serving, because a registry that cannot be reached is a database that
  cannot be reached and stopping the fleet would make an outage longer; any other failure closes the gate, because the process is
  then invisible to activations and never learns of an epoch change while the database works. The next beat that succeeds reads
  the requirements and decides again. **Precondition for activating 029's or 031's capability:** the fleet runs builds with this
  rule (G2003). (3) Two processes never share an instance id (v4, per start); a restart is a new row and the old one stops
  counting after 30 s (test: `a_restarted_process_is_a_new_row_and_the_old_one_stops_counting_after_thirty_seconds`).
- **D2019** (2026-11-08, T018-080) The adapter's operator API. `PgInstanceStore::activate(Activation, step)` and `remove(capability,
  actor, step)` run in one transaction: the epoch is locked `FOR UPDATE`, the live instances are verified, the requirement is
  recorded and the epoch moved, then the feature's own step runs on the same connection and the transaction commits; an error
  of the step rolls the requirement back with it. Outcomes: `Recorded{epoch}`; `AlreadyRecorded` for the same kinds (nothing is
  done and the step does not run: it ran in the transaction that recorded it); `KindsDiffer` is refused (a capability is recorded for
  one set of kinds; change it by removing it first); `InstancesLackCapability{missing, examples}` names how many and up to
  sixteen ids (ids are not content). A removal locks the epoch, answers `NotRecorded` without running the step when there is
  nothing to remove, runs the feature's verification, then removes. There is no command-line subcommand in this task: the
  first is `control-plane schedule-trigger-handover` (T018-061), which passes its handover write as the step.
- **D2020** (2026-11-08, T018-080; reworded 2026-11-08 after review, D2023) A process lists a capability when **the process** will
  honour it: the binary's list, narrowed by the passes its configuration turns on (`apps/execution-engine/src/capabilities.rs`).
  The control plane lists none until 029 or 031 gives it one (`apps/control-plane/src/registry.rs`); the engine lists `schedules-v1`
  when `optional_passes(config)` contains the schedule pass. The closed list is in `sagat_application::instances::Capability`, in the
  CHECKs of the schema and in rule 5 of the data model, and a test fails when they disagree
  (`the_capabilities_and_kinds_in_code_are_the_closed_values_of_the_schema_and_of_the_data_model`). A value written by a newer build
  (a requirement for a capability this build does not know) is read as the string it is, is lacked, and refuses the process with its
  name: never a failure to read (unit test in `crates/application/src/instances.rs`).
- **D2021** (2026-11-08, T018-080) What "stops serving" means. Control plane: a middleware on the whole `/v1` router and on the
  upload router answers every call but the two probes with `503 UNAVAILABLE` / `BINARY_CAPABILITY_MISSING` (`Retry-After: 10`)
  before authentication; `/v1/health` is never refused, and `/v1/ready` is answered by its own handler, which probes the database
  first (so that a failing probe is logged with its cause, as before) and then consults the gate, so the load balancer drains the
  replica; a gate that is closed because the process has not registered yet names no capability (the plain 503); a stream ends
  with a `reconnect`. The worker gRPC admission refuses every call `UNAVAILABLE` while the gate is closed, before authentication (G1998: closed for new calls and for the claims of an open task stream). Engine: every loop runs its round through `loops::while_serving` and the supervisor of the connectors winds its consumers down (D2027) (a closed gate is a round that found nothing, so the
  liveness of the loops continues), the keeper gives its shards back and leaves the replicas that count, `/readyz` answers
  `503` with the reason in the error envelope and `/livez` stays `200` (a restart would only be refused). A fitness test over
  the engine's wiring fails when a loop is added that bypasses the gate (`apps/execution-engine/tests/registry.rs`).
- **D2022** (2026-11-08, T018-080) Observability. `serving_instances_live{kind}` (a gauge per kind of the closed set, `0` when none
  is live, the same value from every process because the beat reads the fleet: read it with `max`) and
  `instance_registry_refusals_total{reason}` (`BINARY_CAPABILITY_MISSING`, `REGISTRY_FULL`, `REGISTRY_UNREACHABLE`; counted when
  a process is refused or cannot be counted, and once when a running process learns of a requirement it lacks or fails closed),
  and, per process, `instance_registry_gate_open` (0/1) and `instance_registry_last_beat_age_seconds` (D2029). No instance id is a label; the
  id is in the log line. Both are in the metrics tables of the runbooks of the control plane, the engine and observability.

- **D2023** (2026-11-08, T018-080, review A, closes G1997) The capability follows the configuration. `capabilities(config)` lists
  `SchedulesV1` iff the schedule pass is in `optional_passes(config)`; the flags are read from the environment once, so the list is
  true for the life of the process. An activation that requires `schedules-v1` from every live engine therefore refuses an engine
  that runs no schedule pass, naming its id, and an engine restarted without the pass after the handover is refused at registration
  and closes its whole gate (runbook section 2). Without this the handover (T018-061) would stop the legacy cron pass in an engine
  that runs no schedule pass and nothing would fire (tests: `an_engine_lists_schedules_v1_only_when_it_runs_the_schedule_pass_and_an_activation_refuses_one_that_does_not`,
  `apps/execution-engine/tests/registry.rs`).
- **D2024** (2026-11-08, T018-080, review B) A row is owned by a token. A process makes 32 random bytes at every start and sends
  them with every call; the registry stores their SHA-256 in `serving_instances.token_hash`, which the runtime role cannot read
  (column privileges); `instance_heartbeat`, `instance_deregister` and the upsert branch of `instance_register` touch a row only for
  the matching hash, otherwise the row is not found (the upsert raises `restrict_violation`, the row untouched). A holder of the
  runtime role that knows another instance's id (it can read every id) can no longer refresh a dead row, delete a live one or
  rewrite its capabilities so that an activation verifies falsely (tests: `instances_hostile.rs`, both roles; red first). The residual
  is G2000.
- **D2025** (2026-11-08, T018-080, review B) At start, a full table is served. `FULL` is reached only after the check of capabilities,
  so serving on it is as safe as `beat` already was when the row had gone; the process is not counted, which is logged as an error and
  counted (`REGISTRY_FULL`), and it asks again at every beat (test: `an_engine_that_finds_the_table_full_still_serves_and_says_it_is_not_counted`, red first).
  Without it a holder of the runtime role that fills the table closes the gate of every process that starts.
- **D2026** (2026-11-08, T018-080, review C) After 30 s of failed heartbeats the gate closes unless the failure is a lost connection
  (D2018 (2)). The adapter maps connection failures (I/O, TLS, pool timeout or closure, SQLSTATE class 08, shutdown 57P01 to 57P03) to
  `StoreError::DatabaseUnavailable`, everything else keeps the mapping of the adapter; the control plane decides with
  `closes_the_gate` (tests: `a_connection_failure_beyond_the_window_leaves_the_gate_open_and_any_other_failure_closes_it`,
  `failing_closed_forgets_the_epoch_so_that_the_next_beat_that_succeeds_decides_again`).
- **D2027** (2026-11-08, T018-080, review D) A closed engine reads no connector and the fitness test is structural. The supervisor of
  the connectors takes the gate (`Supervisor::with_serving_gate`): while it is closed every consumer is wound down and none is started,
  so no event is admitted from a broker (`Ingest::judge` -> `runtime.admit`); when it opens they start again at the next refresh. The
  first version exempted the supervisor in the fitness test with a wrong reason. The test now walks the calls of `tasks.spawn_loop(` with
  balanced parentheses, requires the exact set of loops, and requires of each the gate in the way that applies (`while_serving`, the
  keeper's `gate.is_open()`, the supervisor built `with_serving_gate`); one behavioural test starts a closed engine with a due schedule and a
  connector and asserts neither acts and that both do once the engine is let (`a_closed_engine_fires_no_schedule_and_reads_no_connector_and_the_same_engine_does_both_once_it_is_let`, red first).
- **D2028** (2026-11-08, T018-080, review E) The heartbeat is supervised and readiness knows the last answer. In the engine it is a
  critical task (`instance heartbeat`); in the control plane `serve_until_the_heartbeat_ends` stops the server and fails the process if
  it ends while the process is not stopping. The gate keeps the time of the last answer of the registry; readiness (`/v1/ready`,
  `/readyz`) fails when it is older than 30 s, whatever the gate says (the gate rule stays D2026). Tests: `a_process_whose_heartbeat_ended_is_stopped_and_fails_while_one_whose_server_ended_is_not`,
  `a_registry_that_has_been_silent_for_too_long_fails_readiness_and_leaves_the_calls_to_the_gate`,
  `the_registry_fails_readiness_when_the_gate_is_closed_and_when_it_has_been_silent_too_long`.
- **D2029** (2026-11-08, T018-080, review F and G) The counters and gauges are proved by recorders of the test's own (the unit tests
  of `instances::tests`), not by the global page that parallel tests share; the process tests no longer read it. Two per-process gauges
  are added, `instance_registry_gate_open` and `instance_registry_last_beat_age_seconds`, set at every try, and two alerts,
  `SagatInstanceGateClosed` (five minutes) and `SagatInstanceRegistrySilent` (thirty seconds for a minute), each with a runbook section
  and a promtool test (`infra/observability/tests/instance_registry_test.yaml`, run with the pinned image as CI does).
- **D2030** (2026-11-08, T018-080, review H and K) The probe of the engine and the control plane say why with one function
  (`sagat_api_http::not_serving`): the reason `BINARY_CAPABILITY_MISSING` only when a capability is missing. The remediation sentence
  that an unauthenticated caller reads names no replica and no version ("The service cannot serve this call now: try again..."). The
  history also stores `session_user` (`database_user`). The operator's transaction sets `lock_timeout` (2 s), `statement_timeout` (3 s)
  and bounds the feature's step (3 s) with a timeout of its own (`OperatorBounds`); a slower step is rolled back, because every process that
  registers waits for the epoch meanwhile and has a statement timeout of 5 s (tests: `the_operators_transaction_runs_under_its_own_lock_and_statement_timeouts`,
  `a_feature_step_that_takes_too_long_is_cut_off_and_nothing_is_recorded`, `a_statement_of_the_step_that_outlasts_the_statement_timeout_is_cancelled`).
- **D2031** (2026-11-08, T018-080, review L) The registry has a connection of its own (a pool of one, `registry_pool_config`), so that
  saturation of the pool of the requests cannot silence a healthy process; the row is deleted after the server has drained (the
  heartbeat loop no longer leaves), so that an activation in the grace window still counts a replica that is finishing its work; the
  worker task stream checks the gate before every claim (`Admission::not_serving`), so a stream open when the gate closes leases nothing
  more (test: `a_task_stream_that_is_open_when_the_gate_closes_leases_nothing_after_it`); every operation of the OpenAPI contract is walked
  against a closed gate and must be refused but liveness, so a route merged outside the gated layer fails
  (`every_operation_of_the_contract_that_this_server_mounts_is_refused_by_a_closed_gate_but_liveness`).
- **D2032** (2026-11-08, T018-080, review I and J) The decisions that are hard to reverse are in ADR-0082 (proposed, awaiting the owner).
  A feature's activation step *reads, it does not lock* (the step runs while the epoch is held `FOR UPDATE`; a transaction that reads
  it `FOR SHARE` waits for the whole step): stated in the module documentation of the adapter, in the migration header and in the
  runbook; the epoch is the first slot of the lock-order inventory (`Slot::RegistryEpoch` in `tests/support/lock_order.rs`, with a test
  that a plan which returns to it is refused). The migration is `20270113000000`: `20270110000000` is taken on `origin/develop` by
  `step_credential_roles` (feature 035); this migration was unpublished and was renumbered.

- **D2033** (2026-11-08, T018-060) No data migration makes the schedules of the cron triggers that exist: the reconciliation pass does, and so does
  publication from the first publication of this release on. Deviation from `plan.md` ("The migration creates one trigger-owned schedule per existing cron
  trigger"): a migration cannot plan a next time without the calendar (`croner` and the time-zone database live in `sagat-infra-ir`, and a migration that
  walked every cron trigger would hold locks on every tenant's rows), and the pass is needed in any case for the older control plane. The cost is a window of
  seconds, after an upgrade, in which a trigger has no schedule or a schedule has no next time: the legacy pass fires meanwhile, so nothing is lost.
- **D2034** (2026-11-08, T018-060) Trigger-owned schedules are **exempt** from the `schedules` quota: neither counted by it (`ensure_room_for_schedule` and the usage
  of `QuotaStore::usage` count `origin = 'API'`) nor refused by it. A publication must never fail because the tenant filled the quota with its own schedules, and the
  number of cron triggers is already bounded by the workflow's own limits. Their name and spec bytes are still charged to the tenant's content (about 600 bytes
  each). The per-tenant bound behind it is the `workflows` quota of each namespace times the 16 triggers a workflow may have (`MAX_TRIGGERS` is per workflow, not per tenant).
  *Corrected 2026-11-08 (R):* "a publication is never refused for it" is true of the schedule **quota** only. The schedule and its revision are charged to the tenant's
  content bytes, so a tenant at its content ceiling is refused at the commit of the publication (`QUOTA_EXCEEDED`), as it is for the version itself; nothing else of
  the schedules refuses it (a taken name, a trigger this build cannot read, are left to the reconciliation). Tests: `trigger_schedules::trigger_owned_schedules_neither_count_against_nor_are_refused_by_the_schedules_quota`.
- **D2035** (2026-11-08, T018-060) The name of a trigger-owned schedule is `trigger-<32 hex digits of the trigger id>`; if a tenant took that name first the schedule
  is made as `…-1`, `…-2`, `…-3` (four names in all), and a tombstone that comes back whose name was taken meanwhile is renamed with a random suffix. A taken name
  never blocks a publication (`INSERT … ON CONFLICT DO NOTHING` aborts nothing); a trigger whose four names are all taken is left to the reconciliation, which logs
  it every round with the tenant and the trigger (G2007). A tenant can only hurt itself this way.
- **D2036** (2026-11-08, T018-061, closes G2002) The command is `control-plane schedule-trigger-handover --operator NAME [--expect-live N] [--dry-run]`, with the schema-owning
  role. `--expect-live N` makes the command refuse unless **exactly** `N` execution engines are live (seen within 30 s): fewer means an engine has not registered or is
  down, more means one nobody counted. Without it the command refuses when **none** is live (an activation with none passes vacuously), and `--expect-live 0` is the
  explicit way to say so. `--operator` is required except with `--dry-run`, must be a valid subject (not in the reserved `system:` namespace) and is the actor of the
  registry's history; the database `session_user` is recorded with it (handover row, `changed_db_role`; history, `database_user`). The count is repeated inside the
  activation's transaction, under the lock of the epoch, so an engine that registers between the check and the lock is refused with nothing recorded. A
  `permission denied` becomes "run it with the schema-owning role"; a refusal for engines names how many lack `schedules-v1`, up to sixteen instance ids, and says an engine
  lists it only when it runs the schedule pass (`SAGAT_SCHEDULES_ENABLED`). A second run says it is already handed over and exits 0; a dry run verifies the same things
  and the privilege and changes nothing. Tests: `trigger_handover::*`.
- **D2037** (2026-11-08, T018-061) The handover state is one row read from the database. The legacy cron pass of an engine believes `LEGACY` for **two seconds** and a
  `HANDED_OVER` for good (the guard of the table refuses any move back, so there is nothing to invalidate); a state it cannot read, or one this build does not know
  (`HandoverState::Other`, read as written), leaves the legacy pass **running** (a tick fired twice starts once; a tick not fired is lost) and keeps the control of
  trigger-owned schedules refused. The services read the row on each pause, resume, trigger-now and backfill of a trigger-owned schedule only: one tiny read at human
  rate. Between the commit and the engines' next read both passes may fire a tick: one run starts. `SAGAT_CRON_PASS` does not exist (nothing to drop).
- **D2038** (2026-11-08, T018-060) The reconciliation pass divides its work by the shard of the **trigger's id** (a schedule that does not exist yet has no id), asks
  one query per round for all the replica's shards (the join of triggers and schedules, five kinds of drift: `MISSING`, `REVIVED`, `ORPHAN`, `RESPEC`, `NO_NEXT_TIME`),
  takes at most 20 entries of a tenant and 200 in all, interleaved by tenant, and writes each change in a transaction of its own under the lease of that shard, with a
  compare-and-swap on the state it read and a lock wait of at most one second (a held row fails its entry, not the round). An entry that fails or is refused is left
  out of the next rounds for 2 s doubling to 5 min (1,024 remembered). It runs every five seconds, in the engines that run the schedule pass, and in `LEGACY` and after
  the handover alike. A new schedule's next time is the trigger's own `next_fire_at` while `LEGACY` and the legacy pass has planned one (so that the two passes keep one
  tick between them), otherwise the first after the database's now. Metrics: `trigger_schedules_reconciled_total{action=created|removed|repaired}`,
  `trigger_schedule_drift`, `trigger_schedule_reconcile_failures_total`. Tests: `trigger_reconcile::*`, `fault upgrade::the_reconciliation_cut_off_at_any_write…`.
- **D2039** (2026-11-08, T018-060) The firing of a trigger-owned schedule is the legacy pass's: scope `trigger:{id}`, key `cron-{epoch seconds}`, fingerprint the epoch
  (so the request hashes agree), the `cron.fired` document (`application::schedule_triggers::cron_document`, equal byte for byte to what `cron_pass` builds, a test),
  `startedBy` `system:engine`, audit `run.started` by `system:engine` with the reason the legacy pass writes, the journal's actor `system:event-dispatcher`, a trace of its
  own, and the version the **trigger** names (`workflow_triggers.version_id`, the version in force), whatever the schedule's own `ACTIVE` target says. The engine is no
  member: the membership check of the admission is skipped for it (database steps stand on the publisher, as they do for the legacy pass). Its ticks are never tracked and never
  wait (`ALLOW_ALL`). When the schedule pass starts the tick the run also carries the schedule's provenance columns (schedule id, nominal time, `SCHEDULED`); one the
  legacy pass started has none. When the other pass came first the action is recorded `ALREADY_ACTED` with the run it found (found by `tick_run`, a read of the key,
  before the start; the race between the read and the start ends in the refusal "already acted" and the next round records it). A **backfill** of such a schedule keeps
  the key, and a time the legacy pass started is counted `ALREADY_ACTED` and never started twice (red first: the backfill refused it for ever); *trigger-now* keeps the
  manual identity of the schedule and the `cron.fired` document with `scheduledAt` and `firedAt` both now; a schedule whose trigger an older control plane dropped is
  removed by the step that finds it and starts nothing.
- **D2040** (2026-11-08, T018-029) Catch-up of a trigger-owned schedule is `ONE` over 366 days. For an outage inside the window it is the legacy pass's behaviour: one run,
  at the earliest missed time, key `cron-{epoch}` of that time, the rest one `COALESCED` summary. For an outage longer than 366 days the legacy pass fires the oldest
  missed tick whatever its age; the schedule pass records `MISSED_CATCHUP_WINDOW` for the times older than the window and fires the earliest inside it (FR-018-017 fixes
  the window). *Consequence stated 2026-11-08 (R):* with **both** passes running (`LEGACY`) an outage longer than 366 days starts **two** runs, one per pass, under two different keys
  (`cron-<oldest tick>` from the legacy pass, `cron-<earliest tick inside the window>` from the schedule pass): the one-run-per-tick guarantee of SC-018-005 holds for
  outages inside the window, and for a longer one only after the handover. It is not tested because the case needs an outage of more than a year. Tests: `catchup::trigger_owned::*`.
- **D2041** (2026-11-08, T018-061) Resuming a trigger-owned schedule does not make the resumer its authority: `authorizedBy` stays `system:engine` (the resumer is the
  note's author and the audit's actor). A pause after the handover survives a republication that keeps the trigger.
- **D2042** (2026-11-08, T018-061) Migration `20270114000000_trigger_handover`: a guard (`ENABLE ALWAYS`, a transaction-local flag as D2015) that lets
  `schedule_trigger_handover` change only through `schedule_trigger_handover_complete(actor)` and never from `HANDED_OVER`; that definer function (owner only; it refuses
  unless `schedules-v1` is recorded for the engines, moves the state, records the operator and `session_user` on the database clock); `changed_db_role`; and
  `schedule_trigger_handover_census()`, the owner's read of the live engines (the runtime role reads the registry by column and `count(*)` needs the table: the privilege audit
  lists both functions in `ALLOWED`). There is **no per-tenant audit record**: `audit_records` is tenant-scoped and the handover has no tenant (D2013's reason); the evidence is
  the handover row and the registry's `platform_requirement_history` row of the same transaction. `AuditAction::ScheduleTriggerHandover` was removed (D2051).
  *Accountability, stated 2026-11-08 (T):* the evidence is self-asserted. `changed_by` is what the operator typed (D2052 restricts the characters), and `session_user` is
  the owner's login, the same for every operator who shares it: a shared owner login gives no individual accountability. Use one database login per operator (`schedules.md` 6.2).
- **D2043** (2026-11-08, T018-061) A requirement of `schedules-v1` that is recorded while the state is `LEGACY` (recorded by something other than this command) is
  not completed by guessing: the command refuses, says so and changes nothing (G2008).
- **D2044** (2026-11-08, T018-060) The reconciliation of a trigger whose zone or expression this build cannot evaluate (a stored value another build wrote: unknown to
  the time-zone database of this one) is an entry that fails and backs off with the tenant and the trigger in the log, never a failed round and never an error to a caller.
  Publication of such a trigger makes its schedule without a next time.
- **D2045** (2026-11-08, T018-060, surfaces; amended 2026-11-08 by D2053) No REST operation is added. The Studio lists the schedules of cron triggers with a marker, says on the page that their controls wait
  for the handover and shows the platform's refusal in words. *Amended:* the schedule resource carries an additive optional `controllable` for a `TRIGGER` schedule (D2053), so the Studio disables the three
  buttons with the reason before the handover and `sagat schedule get` says whether it happened.

- **D2046** (2026-11-08, T018-061, review A) **The handover requires convergence.** `schedule_trigger_handover_complete` takes `LOCK TABLE workflow_triggers IN SHARE ROW EXCLUSIVE MODE` (no publication commits
  between the count and the commit of the state; a publication waits, bounded by the operator's lock timeout of 2 s) and counts, over **all shards**, the cron triggers in force that have no alive
  `TRIGGER` schedule equal to them (`schedule_trigger_convergence(limit)`: no alive schedule, a head spec that is not the trigger's expression and zone, or an active schedule with no next time), and refuses
  `HANDOVER_UNCONVERGED` unless it is zero. The command repeats the count in its first check and in the step of the activation (`schedule_trigger_handover_census(true)`), refuses naming the count and up to sixteen trigger ids,
  and `--dry-run` reports the same. The gauge `trigger_schedule_drift` is not a precheck (it excludes the entries that are backing off); `trigger_schedule_unconverged` is (D2056). Tests: `trigger_followup::the_handover_refuses_a_trigger_with_no_next_time…`,
  `a_dry_run_reports_the_triggers_that_have_not_converged`.
- **D2047** (2026-11-08, T018-060, review G) **The reconciliation is indexed, bounded and paced.** Partial indexes `workflow_triggers (run_shard(id)) WHERE kind = 'cron'`, `schedules (run_shard(trigger_id)) WHERE trigger_id IS NOT NULL` and
  `workflow_triggers (last_fire_at) WHERE kind = 'cron' AND last_fire_at IS NOT NULL` are in the migration; the drift query and the convergence count run with `statement_timeout` 4 s (a slow scan is a failed round); a productive round is followed
  by a pause that keeps the writes to 100 a second (`TriggerRound::pause_after`, at least 200 ms), never by the next round at once. With 20 entries a tenant a round the same pace is the bound for one tenant. **Convergence time**, measured by
  `convergence_time_of_the_reconciliation_with_ten_thousand_and_a_hundred_thousand_triggers` (ignored, run by hand; one developer machine, Docker Desktop PostgreSQL 16, `RUST_TEST_THREADS=1`, other sessions running): 10,000 triggers with no schedule: the drift query 14 ms, the convergence count 5 ms, 500 schedules made in 1.79 s unpaced (279 writes/s, 20 a round); 100,000 triggers: the drift query 122 ms, the convergence count 34 ms.
  **Extrapolation, not measured, and why:** the write measurement at 100,000 did not run (the test's second replica held no shard) and was not repeated. A single tenant is worked at 20 entries a round with a pause of at least 200 ms plus the round's own time
  (about 0.1 s of queries at 100,000 and about 0.07 s of writes): roughly 0.4 s per 20 entries, so **10,000 triggers of one tenant converge in about 3 minutes and 100,000 in about 33 minutes**; across many tenants a round takes up to 200 entries at the 100 writes a second
  budget (100,000 in about 17 minutes plus queries). The handover refuses until then (D2046). **[Corrected 2026-11-09 (G2048, D2118): the 100 a second is a ceiling, not the pace. Measured by the engine's own loop with two tenants, the drain is 30 to 77 writes a second because the pause is added to the time of the round: 100,000 triggers take 21 to 27 minutes and 10,000 take 2 to 6. The "3 minutes", "33 minutes" and "17 minutes" above are the estimates the benchmark replaced.]**
- **D2048** (2026-11-08, T018-061, review M) **The requirement of `schedules-v1` cannot be removed while the state is `HANDED_OVER`:** a `BEFORE DELETE` guard (`ENABLE ALWAYS`) on `platform_requirements` raises. An engine without the schedule pass could otherwise register
  and nothing would fire the triggers. Test: `trigger_followup::the_requirement_cannot_be_removed_after_the_handover`.
- **D2049** (2026-11-08, T018-060, review L) **The names of trigger schedules are reserved:** an API schedule whose name matches `trigger-` and 32 lowercase hex digits and optionally `-` and digits is refused `422 SCHEDULE_NAME_RESERVED` (`field` `/name`; OpenAPI, `errors.md`,
  `error.rs`), so a tenant cannot take the four names of a trigger in advance (G2007). A trigger whose schedule could not be made is logged at warn with tenant and trigger and counted `trigger_schedules_unmade_total{reason="names_taken"}`.
- **D2050** (2026-11-08, T018-060, reviews C and Q) **Lock order: the trigger's row, then the schedule's.** A publication writes the trigger and then takes the schedule's row (`FOR UPDATE`, also for a schedule it keeps untouched); the reconciliation takes `workflow_triggers … FOR SHARE`
  before the schedule row in `Respec`, `SetNext` and the removal. A deadlock (`40P01`) can no longer abort a publication, and the removal of an orphan cannot tombstone a schedule whose trigger a publication in flight is bringing back (the publication
  waits for the row and sees the tombstone, revives it). Fitness: `Slot::TriggerRow` before `Aggregate` and `Slot::HandoverRow` after `RegistryEpoch` in `tests/support/lock_order.rs`; tests `a_change_of_the_reconciliation_and_a_publication…never_deadlock`, `a_publication_serialises_with_the_removal…`.
- **D2051** (2026-11-08, T018-061, review I) **A requirement recorded by hand while the state is `LEGACY` is completed, not refused:** the command repeats the census, the convergence check and `schedule_trigger_handover_complete` in one transaction under the epoch's lock
  (`PgInstanceStore::run_locked`), and still refuses when the checks fail. `AuditAction::ScheduleTriggerHandover` (unused) was removed. Closes G2008.
- **D2052** (2026-11-08, T018-061, review B) **The irreversible run is asked for.** `--apply` is required to hand over; without it (and without `--dry-run`) the command behaves as `--dry-run` and says so. A value that starts with `-` is never taken as the value of an option
  (`--operator --dry-run` recorded `--dry-run` as the operator and ran the handover), an operator name must be printable (no control, bidirectional or zero-width character, no space at its ends), and the info line logs the OS user and host.
- **D2053** (2026-11-08, T018-061, reviews N and J) **The control of a trigger-owned schedule opens `changed_at` + 5 s after the commit** (the 2 s the engines may believe `LEGACY` and 3 s of margin), by the database's clock (`trigger_control_open`): until then pause, resume, run now and
  backfill are `SCHEDULE_TRIGGER_HANDOVER_PENDING`, so a pause never lands where a legacy pass may still fire. The schedule resource (and its description) carries an additive optional `controllable: boolean` for `origin` `TRIGGER` only: `false` until then, `true` after; the 409 stays the authority.
  The Studio disables the three buttons with the reason; `sagat schedule get` says whether the handover happened. Closes G2004.
- **D2054** (2026-11-08, T018-060, review E) **An entry carries the stored cron text and zone, unparsed.** Parsing happens in `reconcile`, so a row this build cannot parse (six fields, an unknown zone) fails its own entry, which backs off, never the query for every tenant; a row that cannot be
  decoded at all is skipped, logged by trigger id and counted `trigger_schedule_rows_undecodable_total`. Test: `trigger_followup::a_cron_row_this_build_cannot_parse_fails_its_own_entry_and_not_the_round`. Amends D2044.
- **D2055** (2026-11-08, T018-060, review D) **The engine's authority needs no membership in any path.** The admission skips the membership recheck for every start of a schedule that has a trigger and whose authority is `system:engine` (the tick, the release of a waiting action, a run-now released later); the first build skipped it
  only for the scheduled tick, so a buffered run-now of a trigger-owned schedule recorded `AUTHORITY_REVOKED` and paused the cron trigger. A backfill and a run-now of a person keep the person's check. Test: `trigger_followup::a_buffered_run_now_of_a_trigger_owned_schedule…`.
- **D2056** (2026-11-08, T018-060, reviews H and K) **A reconciliation that never converges is visible.** Gauges `trigger_schedule_backing_off`, `trigger_schedule_last_round_timestamp_seconds` and `trigger_schedule_unconverged` (all shards, every sixth round, bounded); counters `trigger_schedule_rows_undecodable_total`,
  `trigger_schedules_unmade_total{reason}` and `cron_legacy_fires_after_handover_total{source="legacy_pass"}`; alerts `SagatTriggerScheduleReconcileFailing`, `SagatTriggerScheduleReconcileStale`, `SagatTriggersUnconverged` with `infra/observability/tests/trigger_schedules_test.yaml`
  (run with `promtool test rules`, as CI does). **Tripwire for G2005:** a cron trigger whose `last_fire_at` is later than `changed_at` + 10 s was fired by a legacy pass: logged by trigger id and counted. The CI step must list the new test file at the next touch of the workflow (not touched here).
- **D2057** (2026-11-08, T018-060, review S) The journal's actor of a run the schedule pass started for a tick is `system:event-dispatcher`, as the legacy pass writes it, so that the run is the same whoever started it; the provenance (schedule id, nominal time, kind) is in the run's columns, which only the schedule pass sets.
- **D2058** (2026-11-08, reviews S and T) The decisions that are hard to reverse are in ADR-0083 (proposed, awaiting the owner): no data migration, the quota exemption, the engine's authority without a membership, the one-way switch with its convergence requirement and its control window.
- **D2059** (2026-11-09, T018-057, `FR-018-038`, OD-2026-11-08-1) **A reset that waits expires.** An entry that is still `PENDING_SOURCE` a whole
  window after its request, while its source is still open, ends `FAILED` with the closed reason `SOURCE_DID_NOT_END` and `completed_at` set; no run
  is made; the source is left as the request left it (cancelled, still ending, or in manual review); no call withdraws an entry. It is a step of the
  pass that already completes the entries (`ResetPass`, same fence, same transaction discipline): **after** the entries that are due, so that a
  source that has ended is always the completion's, and through `ResetCompleter::expire` (`Completed::Expired`). **The flag governs the completion and the
  expiry together** (amended 2026-11-09, D2080): the expiry is a step of the reset pass, so an engine with `SAGAT_RESET_CANCEL_SOURCE_ENABLED` off runs
  neither. The window is **counted from the request** (`run_resets.created_at`, the database's `clock_timestamp()` of the transaction of the source, D2079; it
  is stored: a restart of the engine does not start it again, and several engines agree) and compared on the
  clock the completer is given, which an engine makes the database's (`DatabaseClock`, synced before every round as the other passes do), never
  the process's own; the boundary is inclusive (an entry requested exactly one window ago has expired; one microsecond short of it waits). On the
  first rounds after an engine with this version starts, **every entry already older than the window expires**, a few at a time (D2061): an
  operator who turns the flag on with old entries in the register should read them first. Tests: `cancel_source_expiry` (integration, 18),
  `cancel_source_rest::an_entry_that_expired_reads_failed_with_its_reason_and_no_run_and_the_source_is_as_it_was`.
- **D2060** (2026-11-09, T018-057, plan lock-order row 18) **One transaction, the same write as every other end of an entry, and no wait for a run.**
  `expire_reset` takes the fence of the shard and the entry `FOR UPDATE` as a failure does (`lock_pending_row`), then reads under that lock whether the
  entry was requested at or before the cutoff the pass gave it (a second reading of `created_at` against **the caller's** value, `NotDue` otherwise:
  *amended 2026-11-09, D2079: the first version of this text said that a caller that computed the cutoff wrongly could not end an entry inside its window, which is
  false; the cutoff is the pass's database-clock time minus the window, the trait is the engine's own and nothing a tenant sends reaches it, and the tests pin the
  clock the pass is given*) and whether the source has ended, then writes the entry and its audit record with **the very function `fail_reset` uses**
  (`end_locked_entry`: no second code path). The place of the entry in its namespace is freed by that write: the count that judges the limit is the
  count of the entries `PENDING_SOURCE`, derived, so nothing is decremented (test `the_place_an_entry_held_is_free_for_a_new_request_at_the_limit`).
  The row of the source is read `FOR SHARE ... SKIP LOCKED`: the transaction that ends a run holds that row, so **a source that is being ended, or is in
  the hands of any other transaction, makes the expiry leave the entry for the next round** (`Expiry::SourceBusy`, answered `Completed::Busy`, which the pass leaves out for a
  second: D2081) and never wait
  for a run, and a source that has ended before the read is `SourceEnded`, the completion's. **An ended source wins**, and an entry is never both
  expired and completed: two outcomes of one entry meet at its row, and whoever takes the lock second finds it done (`Refused`) or finds the source
  ended. *Which test guards which (amended 2026-11-09, review D):* the guard "an ended source wins" is the deterministic tests
  `the_store_ends_an_entry_only_when_it_is_past_the_cutoff_and_its_source_is_open` (`SourceEnded`), `a_source_that_ends_after_the_entry_has_lapsed_but_before_the_pass_looks_is_completed_and_not_expired`
  (the completer leaves it to the completion) and part (2) of `an_entry_is_never_both_expired_and_completed_whichever_comes_first`; the guard "never wait for a run, and take the lock an
  ending `UPDATE` conflicts with" is the same part (2) and `the_store_never_waits_for_a_run_...`, which hold the source **with an uncommitted `UPDATE runs SET updated_at = updated_at`**
  (`FOR NO KEY UPDATE`, what the end of a run takes: a `SELECT ... FOR UPDATE` conflicts with `FOR KEY SHARE` too and would have let that weaker mutant pass), and are bounded with
  `within` so a lost `SKIP LOCKED` fails instead of hanging; `an_expiry_and_a_completion_that_meet_at_the_entry_make_one_outcome` is the **race** test (one outcome), not the guard test:
  `join!` polls the completion first, so the expiry nearly always gets the lock second and is refused. The order of locks is the completion's (the fence, the entry, then the run: aggregate before run, plan row 18, which now says so); the expiry
  takes no quota key and no membership. `FOR SHARE` on `runs` needs the `UPDATE` the runtime role has: the sweep with `SAGAT_TEST_AS_RUNTIME_ROLE=1`
  runs every test below. Tests (fault): `an_expiry_and_a_completion_that_meet_at_the_entry_make_one_outcome`,
  `an_entry_is_never_both_expired_and_completed_whichever_comes_first` (the expiry first, the source held by the transaction that would end it, the
  source ended), `two_engines_expiring_the_same_entry_record_one_expiry_and_the_second_is_refused`, `a_replica_that_lost_its_lease_expires_nothing_and_the_one_that_holds_it_does`,
  `an_expiry_that_dies_at_any_write_leaves_the_entry_pending_and_sent_again_ends_it_once` (three writing statements in one transaction on
  `run_resets`, `audit_records` and `quota_transaction_deltas`), and, in the integration suite, `the_store_never_waits_for_a_run_and_leaves_an_entry_whose_source_is_in_the_hands_of_another_transaction`
  (the call returns while the row is held, bounded by a timeout, never a sleep).
- **D2061** (2026-11-09, T018-057, D2006) **The scan is bounded the way the completion's is.** `expired_resets` (`EXPIRED_RESETS_SQL`) takes the oldest
  `per_tenant` entries of each tenant among the entries of the shards the replica holds that are `PENDING_SOURCE`, requested at or before the cutoff and
  whose source is open (`finished_at IS NULL`), leaves out those the pass backs off from, and then the oldest `batch` of those; **a round expires at most
  `batch` entries (100 by default, at most 1,000) and at most `per_tenant` (10) of any one tenant, and completes at most as many again**: the two stages
  are counted apart (so the worst round makes `2 x batch` writes), never an unbounded loop. An entry whose expiry errors (a defect, an outage that touches
  one row) is left pending (**a transient error is never recorded as the end of an entry**), joins the same backoff as the completion's (2 s, doubling to
  5 minutes, at most 1,024 remembered, ids in every line) and does not stop the others of the round; the next scan leaves it out until its time. *Amended 2026-11-09 (D2081):*
  an entry whose source another transaction holds is left out for a second too, and a round that expired entries is followed by the pause they cost at a hundred a second:
  the caps bound a round, not a rate. Every line the
  pass logs about an expiry names `tenant_id` and `reset_id` and the window, never a justification. The query reads `run_resets` through an index of the
  entries that wait (`run_resets_pending_idx (shard, created_at)` or `run_resets_pending_namespace_idx`, both partial on the state), so what it reads is bounded
  by the entries that wait (16 a namespace by default, 64 at most) however many the register keeps. A test asks the planner (`the_scan_for_expired_entries_can_be_served_by_the_index_of_the_entries_that_wait`,
  with sequential scans disabled, because on a handful of rows the planner would choose one whatever an index is worth): it is the shape of the plan, not a measure (G2015).
  Tests: `a_round_expires_at_most_its_batch_oldest_first_and_the_rest_wait_for_the_next`, `the_entries_of_one_tenant_cannot_fill_a_round_of_expiries_and_starve_another_tenant`,
  `the_scan_for_expired_entries_is_oldest_first_bounded_per_tenant_and_leaves_out_what_it_is_told_to`, and (fault)
  `an_entry_that_errors_stays_pending_is_backed_off_from_and_does_not_stop_the_others`.
- **D2062** (2026-11-09, T018-057, D2002) **The setting.** `SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS` of the **engine**: whole seconds, the unit every other duration of
  the engine takes in its name (`_SECS`, `_MS`, `_DAYS`), default 86,400 (24 hours), accepted from 3,600 (1 hour) to 604,800 (7 days) **inclusive**. It is parsed
  strictly: a value outside the range, not a whole number (`24h`, `1d`, `86400s`, `1.5`, a negative, an overflow) refuses to start naming the variable and the bounds, and
  does **not** become the nearest bound (`ResetSettings::effective` clamps as a last defence only); an empty value is unset; it is read even where the pass is off, so a
  deployment that sets it means it. It is in `--help` of the engine. **Only the engine reads it**: the control plane accepts the request and expires nothing, so it has no
  window, does not refuse a deployment that sets the variable for both processes, and its `--help` says the setting is the engine's. Several engines with different
  windows each apply their own to the entries of the shards they hold, **and shards move between engines**, so the effective window of an entry is the one of the engine
  that holds the lease of its shard when the round looks: set the same value on every engine (D2082, runbook; each engine logs its window once when the pass starts). Tests: `apps/execution-engine/tests/config.rs`
  (`a_reset_that_waits_expires_after_a_day_unless_...`, `a_window_outside_an_hour_to_seven_days_or_not_a_whole_number_of_seconds_refuses_to_start_naming_the_variable`,
  the usage), `apps/control-plane/tests/config.rs` (`the_window_of_the_expiry_of_a_reset_that_waits_is_not_a_setting_of_the_control_plane`),
  `the_window_is_the_one_the_deployment_set_and_an_engine_with_another_one_does_not_expire_what_is_not_its_own`, `a_restart_of_the_engine_does_not_start_the_window_again`.
- **D2063** (2026-11-09, T018-057) **Migration `20270115000000_reset_expiry`** (the maintainer renumbers at merge): the `run_resets_failure_reason_valid` CHECK admits
  `SOURCE_DID_NOT_END` and nothing else changes (no table, column, index, grant or classification: the reason is a column the Rust side keeps closed and the CHECK holds
  closed in the database, so a migration was needed). Additive: a reader that does not know the reason reads it as text (D2064). The `down` refuses while an entry that
  expired exists (`object_not_in_prerequisite_state`: it is evidence of a request that cancelled a run and made no replacement) and otherwise restores the list of
  `20270108000000`. **Rollout: migrate before the engine that expires**: an engine that writes the reason against a schema without it is refused by the CHECK, which the pass
  treats as a failure of the platform (the entry stays pending and is tried again after a pause), never as the end of the entry. The three upgrade oracles
  (`release_upgrade`, `_inflight`, `_callback_cutover`) now expect the version `20270115000000`: **not run here** (they build the released binaries; they run when a release
  is closed). **Accepted (review of 2026-11-09, F):** re-validating the CHECK takes `ACCESS EXCLUSIVE` on `run_resets` for the time of the scan of the register, which is small
  (the entries of runs that were reset, kept as long as the runs are) and is one transaction, as `20270108000000` did; `NOT VALID` followed by `VALIDATE CONSTRAINT` gains nothing
  inside one transaction. Tests: `reset_expiry_migration` (the list admits the new reason and stays closed, the rollback refuses with an entry that expired and restores the list
  when none does, the up applies again; the migration before it is read from the list the migrator embeds, so a renumbering at merge changes `THIS` only) and the two of
  `pending_resets_migration` still green with this migration above them. That a reason belongs only to an entry that failed, and a run to one that was created, is decided by
  `run_resets_failure_only_when_failed` and `run_resets_run_only_when_created` of `20270107000000`, not by this migration, and is not asserted here.
- **D2064** (2026-11-09, T018-057, `FR-018-041`, `FR-018-047`) **What a client sees.** `Reset.failureReason` is a string in the contract, not an enum: the OpenAPI
  descriptions (`specs/001-platform-foundation/contracts/openapi.yaml`, `contracts/rest.openapi.yaml`) name the new reason and the generated Studio types follow
  (`npm run api`, `check:api`); `contracts/errors.md` says it in one sentence. The six SDKs read it as an opaque string (`Option<String>`, `string | null`,
  `FailureReason`), which is why an older client that gets it keeps working, and no SDK source changed. **The SDK scenario `reset` is not changed**: it cannot wait 24 hours
  and the window cannot be set below an hour, so the contract is tested in the integration suite with an injected clock (D2068). The CLI says the reason in words
  (`workflow reset --cancel-source`, `workflow resets`), the Studio lists the entry as one that could not be made with the reason in words and offers nothing to do with
  it (no link, no button); both still say a reason they do not know as written. No new error code and no `diagnostics.rs` or quickstart sentence: the reason is not an
  error code. Tests: `cancel_source_rest::an_entry_that_expired_reads_failed_...`, `apps/cli/tests/reset_and_with_start.rs` (two), `ResetRun.test.tsx` (two).
- **D2065** (2026-11-09, T018-057, D2003) **What it counts.** The counter that exists, `workflow_reset_completions_total{outcome,reason}`, gets the outcome `expired` with the reason
  `SOURCE_DID_NOT_END` (no new label, no new unbounded value: `outcome` is `created`, `failed` or `expired`; `reason` is `none`, the six of the completion or the one of the expiry),
  incremented after the transaction that ended the entry has committed, so a step that was refused, found the entry inside its window, found the source ended or busy, or was cut
  off counts nothing. **`outcome="failed"` keeps its meaning**, the judgement of the platform on the journal the source ended with, so an alert on it does not fire for an
  expiry. The gauge `workflow_resets_pending` falls with the expiry (it counts the entries that wait). **No alert is added**: a rule needs its `promtool` test and `promtool` is not
  installed on this machine (G2016); the existing `SagatSchedulerLate` is about the lag after a source has ended and is not changed. Test:
  `what_the_expiry_counts_is_counted_once_it_is_committed_under_the_closed_labels` (one series, the two closed label keys).
- **D2066** (2026-11-09, T018-057, D1998) **Audit.** The expiry is a system action: one `run.reset` record on the source, actor `system:engine`, reason
  `completes reset {id}: failed SOURCE_DID_NOT_END` (the same shape as every other end of an entry, so the join by the id of the entry that the runbook describes holds), in the same
  transaction as the end of the entry. The requester stays the actor of the request, whose records are not touched; no free text of the caller is in it. Test:
  `the_expiry_is_a_system_action_that_leaves_the_requester_the_actor_of_the_request_and_writes_no_free_text`, and the tenant of the record is the entry's
  (`the_expiry_of_an_entry_is_audited_in_its_own_tenant_and_touches_no_other`, which also shows that naming the entry of another tenant under one's own reaches nothing).
- **D2067** (2026-11-09, T018-057, D1996) **What an operator is left with.** The request cancelled the customer's run and the expiry never takes that back, so an entry that expired is
  a cancelled run and no replacement, like any other `FAILED` entry; the way forward is to end the source (retry, skip or resolve the step) and reset it again, which is made at
  once because the source has ended. A retry of the request that was accepted, with the same key, is still answered from the register (now `FAILED`): to ask again use a new key. The
  runbook (section 4.2) says it.
- **D2068** (2026-11-09, T018-057, AGENTS rules 12, 17, 18) **How it was tested, and what was not.** Time is never slept for: the completer is given a clock the test sets (`TestClock`)
  and the cutoff is the clock minus the window, so the boundary is tested to the microsecond and a day is a number; the races are held row locks and the queue of the lock
  (`pg_stat_activity`), the crashes are the connection killed after each writing statement. The integration, fault and platform-foundation suites ran in both roles and the
  contract suites and the unit tests once, as the note under T018-057 in `tasks.md` says with the counts. Seen failing for the right reason before the code (the store returned nothing and nothing was ended): the integration tests of the expiry, the engine
  and control-plane configuration, the CLI words and the Studio words; **the fault tests of the expiry were written after the store and were seen failing only when the two guards they
  stand for were removed by hand for one run** (the look at the source under `FOR SHARE ... SKIP LOCKED`, and the check that the source has ended), each by the test that names it. **No mutation battery was run**:
  OD-2026-11-06-2 (mutation testing only when a release is closed) says not to, and the brief of this slice asked for one; the release closing carries it over the guards of this slice.
- **D2069** (2026-11-08, T018-060/061, third review A1, A2) **One definition of "converged", for the reconciliation and for the handover.** The database has it once, the view `schedule_trigger_drift` (invoker's rights): a cron trigger in force is listed when it has no schedule (MISSING), a tombstone (REVIVED), a head revision that is not its expression and zone (RESPEC), a paused schedule while the state is `LEGACY` (PAUSED), or an active schedule that lacks its nominal or its actual next time and is not exhausted (NO_NEXT_TIME). The reconciliation reads the view to repair, `schedule_trigger_convergence` and `schedule_trigger_unconverged` read it to count, so "nothing to repair" and "converged" cannot disagree (the first build had two predicates that did). **A calendar with no occurrence** (`0 0 31 2 *`): feature 004's `CronCalendar::validate` refuses it at publication (`InvalidDefinition`, "the schedule has no next time"), which is the legacy behaviour and is kept; a row that exists anyway (written by an older build or by a calendar that changed) is accepted: its schedule is marked **exhausted** (no next time, `remaining_actions = 0`, one warning naming the trigger) and that is converged, so one tenant's row never blocks the handover of the platform. A change of the expression to one that fires resets it (Respec and Revive write the calendar's answer). **A paused trigger schedule in `LEGACY`:** before the handover the API refuses every pause, trigger schedules have `pause_on_failure` false and there is no "disabled trigger" in the model, so a pause can only be an accident (an engine build that lists `schedules-v1` and revoked the engine's authority, a hand-written update): the reconciliation resumes it (`Change::Resume`, audited `schedule.resumed` by the engine), and the handover counts it until then. After the handover a pause is the operator's and is not drift. **No released build lists `schedules-v1`:** the registry commit `91977bfb` is in no tag (`git tag --contains` is empty; the tags are `v0.1.0` and `v0.2.0`), so the registry exists only on this branch. Tests: `trigger_round3::the_reconciliation_has_nothing_to_repair_exactly_when_the_handover_would_count_none_over_every_state` (seven states of a schedule, both states of the handover), `a_cron_that_never_fires_is_converged_as_exhausted_and_does_not_block_the_handover`, `a_paused_trigger_schedule_in_legacy_stops_the_dry_run_and_the_reconciliation_resumes_it`.
- **D2070** (2026-11-08, third review B3, B4) **The locks of the handover, in one order and with a bound.** The order is the requirement epoch (the caller's), then the table of the cron triggers, then the handover row: `schedule_trigger_handover_complete` now takes them in that order like the census (it took the row first, so two operators could deadlock). Both take the table through `schedule_trigger_handover_lock()`, which sets a `lock_timeout` of two seconds for that statement and answers `HANDOVER_BUSY` ("run the command again"): a `SHARE ROW EXCLUSIVE` request waits behind every writer in flight and, while it waits, holds back every later writer (a publication, the legacy pass's moving of a trigger), so without a bound one long publication would stall them all. **The most any writer stalls because of the command is two seconds, once per attempt**; the command fails in words and records nothing. Tests: `the_function_takes_the_table_of_the_triggers_before_the_handover_row` (derived from `pg_locks`: while the function waits for the table it holds no `RowShareLock` of the handover row; deleting or moving the lock turns it red), `a_long_publication_makes_the_command_try_again_and_does_not_hold_the_writers_behind_it`; `tests/support/lock_order.rs` has the slot of the table before the handover row, and `the_slots_of_the_trigger_row_the_schedule_row_and_the_handover_row...` states the plan the two tests above check against the real locks.
- **D2071** (2026-11-08, third review B5) **Under the lock, whether; outside it, how many.** The census under the lock and the function ask `schedule_trigger_unconverged()` (an `EXISTS`, which stops at the first row), not a count of up to 100,000 run twice while publications wait. The count and the sixteen ids the operator is told are read after the transaction ended (`unconverged_now`), and the dry run reads them outside any lock. The atomic guarantee is unchanged: `trigger_followup::a_trigger_published_while_the_handover_counts_is_waited_for_and_named_in_the_refusal` stays red if the lock is removed. A function that finds the state anything but `LEGACY` or `HANDED_OVER` refuses (a state a newer build wrote is never moved by this one; `the_function_moves_only_the_state_legacy`).
- **D2072** (2026-11-08, third review B6, C10, C11) **A round costs a window and what it writes; the loop reads the state before each trigger; the control opens after the engines have stopped.** (a) The drift query reads a **window**: the next 2,000 cron triggers after a cursor `(shard of the id, id)` among the shards the replica holds (partial indexes ordered by that key), and the schedules of the same range for the orphans; a window that the round's limit cut short stays where it is (what was written is no longer drift), one that was consumed moves on, the last one starts the sweep over. A sweep does not wait for the poll: after a window that found nothing and is not the last, the loop waits 250 ms. The convergence count runs every sixth round that ends a sweep and takes one bounded statement. (b) `MAX_BATCH` is 500, and the pause after a productive round is no longer shortened to the poll: the pause of the largest batch is five seconds, equal to the poll, so "100 writes a second" holds for every setting (`the_largest_batch_is_followed_by_a_pause_no_longer_than_the_poll_of_the_loop_so_the_rate_holds`). (c) The legacy cron pass reads the handover state (cached, at most two seconds old) **before each trigger** and not once per round, so a slow batch that began before the handover cannot fire after it (`a_slow_start_of_the_legacy_pass_cannot_be_followed_by_another_after_the_handover`, held at the table of the runs). (d) `CONTROL_WAIT_SECONDS` (5) is tied to `LEGACY_BOUND` (2) by a unit test with the real constants and a stated margin of two seconds, and by `an_engine_with_the_default_bound_never_fires_after_the_commit_while_the_control_is_open`. **Measured** (one developer machine, PostgreSQL 16 in Docker, other sessions running, `trigger_scale::a_round_costs_the_window_it_reads...`): 20,000 triggers of 200 tenants, five of them with no schedule: see ADR-0083 for the numbers; **100,000 triggers were not measured with the window; the figure for them is an estimate** (the window cost is constant, the sweep is 50 windows).
- **D2073** (2026-11-08, third review C15) **A refusal is not a failure.** An entry that another process or a lost lease got to first (`Reconciled::Refused`) is left out for one second, constant, and its attempts do not grow; only an error doubles the delay (2 s to 5 min). `an_entry_that_was_refused_is_left_out_for_a_short_constant_time_and_one_that_failed_for_a_time_that_doubles`.
- **D2074** (2026-11-08, third review C13) **The tripwire for an old binary does not replay.** The first look of a replica after it started is the baseline (what is there is history); after it, the fires since the latest one seen are counted and warned once with the ids of the latest five; a restart starts a new baseline, so it neither floods the log nor inflates the counter, and several replicas that count one fire only overcount, which an alert on `increase(...) > 0` does not mind (`SagatCronLegacyFiredAfterHandover`). It reads the nominal time of the tick (the table keeps no real fire time), so a late catch-up tick of a time before the handover is not seen (G2019), its window assumes a legacy round ends within ten seconds of the commit, and it works only while one engine of this release runs the schedule pass.
- **D2075** (2026-11-08, third review C12, C14) **A handover that cannot be read: the legacy pass goes on, and says so.** Failing closed after N failures would lose ticks on a fleet whose engines do not run the schedule pass (the census guarantees `schedules-v1` at the handover, not before it) and a process cannot tell which fleet it is in; a tick fired twice starts once (the key). The trade-off is that an engine whose read keeps failing ignores a pause made after the window. So the failures are counted (`cron_handover_read_failures_total`), the run of them is a gauge (`cron_handover_read_consecutive_failures`) and `SagatCronHandoverUnreadable` fires at five for two minutes; the per-trigger read of D2072 narrows the exposure. `trigger_schedule_last_round_timestamp_seconds` is set by every round, including that of a replica that holds no shard (the stale alert would fire for it otherwise).
- **D2076** (2026-11-08, third review B8) **Names are printed through a filter and stored under a check.** `changed_by` has a `CHECK` against control, bidirectional and zero-width characters; the text of `AlreadyHandedOver`, the OS user and the host are printed through `shown` (each such character becomes `?`), so a record written before the check, or an environment variable, cannot rewrite the line. Tests: `the_database_refuses_a_name_that_could_make_the_record_read_as_somebody_else`, the unit tests `what_is_printed_of_a_stored_or_an_environment_text_cannot_rewrite_the_line` and `the_os_user_and_host_of_the_log_line_are_printed_through_the_same_filter`; `an_operator_name_that_could_make_the_record_read_as_somebody_else...` asserts the reason text and uses a name with letters of another alphabet.
- **D2077** (2026-11-08, third review B9, D18) **The command is one function.** `run_handover_command` returns the words the binary prints (`main` only prints them), including "This was a dry run: --apply was not given" only for a verification that nothing stops; the live count it reports is the one the step counted under the lock; the step is `handover_step` of the infrastructure crate, so the control plane names no `sqlx` type (`pub use sqlx::PgConnection` is gone); the dead `ScheduleStore::trigger_handover` port is deleted. `trigger_round3::the_apply_path_of_the_command_hands_over_and_says_what_the_binary_prints`.
- **D2078** (2026-11-08, third review C16, D17, D19, B24) A backfill or a run-now of a schedule whose trigger is gone is refused: the backfill step answers `Fired::Refused` once, a run-now answers `409 SCHEDULE_TRIGGER_GONE` (documented in `errors.md`, `error.rs`, the OpenAPI); an API schedule whose name looks reserved can still be updated, only a **change** of the name to a reserved one is refused; the classification of `changed_db_role` is `global` and names the file that writes it; `hand_over_now` of the test support says that it skips the command's checks, and each path has a test through the real command.

- **D2079** (2026-11-09, T018-057, review of the expiry, A) **The window runs on the database's clock from the request to the pass: verified, and pinned.** The entry's `created_at` is
  `tx.now` (the bind of the `INSERT` in `register_pending`, `crates/infra-postgres/src/execution/resets.rs`), and `register_pending` takes only the transaction of the **source**
  run (it refuses `registration.source != tx.info.run_id`), whose `PgRunTx.now` is `SELECT clock_timestamp()` sampled after the row lock is held (`crates/infra-postgres/src/execution.rs`, `begin`).
  The control plane's `SystemClock` (`self.clock.now()` in `crates/application/src/runs/pending_reset.rs`) stamps the **audit records** of the request only. So a control plane whose
  clock is off cannot make an entry expire early or never; **G2014 was wrong and is closed as "verified not a risk"** (the claim that the fix needed the owner's migration slot is withdrawn
  with it). The regression guards the review asked for: `a_control_plane_whose_clock_is_three_hours_off_neither_cuts_the_window_short_nor_stretches_it` (a run service on a clock +3 h and
  -3 h from the database takes the request; `created_at` is between two readings of the database's clock while the audit record carries the process clock; then, with the entry made
  older, the pass expires nothing a minute short of the window and expires it two minutes past); with the hand mutant that binds the audit record's time (the process clock) into
  `created_at` it was red at the bracket of the readings (`skew 3h: asked at 08:29, the database read 05:29`), then the line was put back. And
  `an_engine_whose_clock_is_three_hours_off_reads_the_window_on_the_database_clock` goes through `loops::reset_round`, the function the engine's loop calls, over a `DatabaseClock` whose
  process clock is +3 h and -3 h away: an entry 22 hours old must not expire under 24, one 24 hours and a minute old must; with the hand mutant that removes the `sync()` of `reset_round` it was red
  (`expired: 1` at 22 hours with the clock ahead), then the call was put back. What the types already pin: the pass of the engine is an `EngineResets` over `DatabaseClock<SystemClock>`, so
  a `SystemClock` in its place does not compile; what they could not pin is the move of the clock to the database's time before each round, which this test does. **The text of D2060 and the doc of
  `ResetExpiry::requested_by` that said "a caller that computed the cutoff wrongly cannot end an entry inside its window" were false and are corrected:** the store compares `created_at` with the instant
  it is given, the cutoff is the pass's database-clock time minus the window, the trait is the engine's own and nothing a tenant sends reaches it.
- **D2080** (2026-11-09, T018-057, review of the expiry, B; G2024) **The flag governs the completion and the expiry together.** The expiry is a step of the reset pass, which is not started
  where `SAGAT_RESET_CANCEL_SOURCE_ENABLED` is off (`optional_passes`): an engine with the flag off neither completes nor expires. So an entry accepted while the flag was on, and left while every
  engine has it off, **pins its place in the namespace until an engine with the flag on runs**, and then every entry older than the window expires, a few at a time, paced (D2081). The text of the
  runbook that said the entries "wait in the register" and "hold a place for the window at most" without that condition was false in that state and says it now (sections 4.2 and 6); the line the engine logs when the pass is off says
  that nothing expires either; `with_the_flag_off_the_engine_runs_no_pass_that_expires_a_reset_either` (`apps/execution-engine/tests/config.rs`) pins that a window set without the flag starts nothing and that
  there is no fourth pass that expires. It is not a regression of the design: the flag was always per process (ADR-0081), and the rollout order (engine first) is what keeps the state from arising.
- **D2081** (2026-11-09, T018-057, review of the expiry, E1 and E2) **A busy source is left out for a second, and the expiry is paced.** (a) `SourceBusy` is `Completed::Busy`, not `Idle`: the pass
  leaves the entry out of the scan for `Backoff::BUSY` (1 second, constant, as D2073 does for a refusal; its failures do not grow, a deferral never shortens what an error asked for and the memory
  stays bounded), counts it (`ResetRound.busy`) and logs it at debug with `tenant_id` and `reset_id`; before, `Idle` made the pass forget the entry, so with a share of one the older entry over a
  source that stays locked took the slot every round and hid the newer entries of its tenant (G2017). `an_entry_whose_source_another_transaction_holds_is_left_out_for_a_second_and_does_not_hide_the_newer_ones`
  (integration, with the line) and the unit test `an_entry_whose_source_is_busy_is_left_out_for_a_short_constant_time_and_its_failures_do_not_grow` were red against the old mapping (`busy: 0`, the entry
  forgotten). (b) The caps of a round (`batch`, `per_tenant`) bound a round and not a rate, and `did_work()` after an expiry made the loop start the next round at once, so the first rounds of an engine with this
  version would have expired every old entry as fast as the database allows. `ResetRound::pause_after` is the pause the entries cost at **a hundred a second**, at least 200 ms for any expiry, as the trigger
  reconciliation does (D2047, D2072); a round that only completed or failed entries is not paced (a backlog of runs to make drains at full speed, as it always did). A full round of 100 entries rests a second; the
  largest (1,000) ten. `a_round_that_expired_entries_rests_for_what_they_cost_at_a_hundred_a_second_and_one_that_did_not_does_not` (the sizes) and `the_loop_of_the_reset_pass_rests_after_a_round_that_expired_entries`
  (the loop honours it: a lower bound on the time between two rounds, a sleep never returns early) were red against a pause of zero (`the next round followed at once: 20µs`).
- **D2082** (2026-11-09, T018-057, review of the expiry, E3, F) **The window is each engine's own, and the shards move.** An engine applies its `SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS` to the entries of the shards it
  holds, and shards pass between engines, so the window an entry is judged by is the one of the engine that holds the lease of its shard when a round looks: **set the same value on every engine** (runbook section 4.2).
  Each engine logs its window, batch and share once when the pass starts (`expiry_secs`, `batch`, `per_tenant`), so that a fleet with two values can be read in the logs; that line is not tested (G2025). **Accepted, not
  done (F):** the re-validation of the CHECK under `ACCESS EXCLUSIVE` (D2063); computing the cutoff in SQL (the pass's database-clock reading is the cutoff, D2079); the duplicate of the literal of `due_resets` in `EXPIRED_RESETS_SQL`
  and the `#[doc(hidden)] pub use` that exports it to the plan test.

- **D2083** (2026-11-09, T018-012) **The lease row of a shard is a leaf lock, outside the slots; the deadlock the first slot case met was the harness's own.** A row of `scheduler_shards` is taken `FOR SHARE` by every fence (`holds_shard`,
  `check_fence`) and `FOR UPDATE SKIP LOCKED` by the keeper (acquire, renew, release: one autocommit statement each, which never waits and takes no run row), so no transaction waits for it while holding another lock and none holding it waits
  for another: it is in no cycle, and where a fence stands relative to the run rows and the aggregate rows is immaterial (the passes of the aggregates take it first, the passes that act on one run, `apply_if`, after the run's row: both are
  correct). **The first run of the case for the delayed-start release was red because its reference took the lease `FOR UPDATE` and then a run row, which models a keeper that does not exist: a modelling artifact of the reference
  transaction, not a finding about the order of production code** (an earlier version of this entry, and of ADR-0050, called it a deviation of the code from the table; the table text is corrected, the code is not touched, and no
  `Slot` stands for the lease). **Guard:** `scripts/check-architecture.py` refuses a statement on `scheduler_shards` that locks more strongly than `FOR SHARE` outside `partitioning.rs`, a keeper statement without `SKIP LOCKED`, a keeper
  that names `runs`, and a fence that is not `FOR SHARE` (its tests: `scripts/tests/test_check_architecture.py`; two hand mutants on the tree, `holds_shard` `FOR UPDATE` and the acquire without `SKIP LOCKED`, each red). The module doc of
  `partitioning.rs` said a takeover "waits for the transactions that found the lease good": the code is `SKIP LOCKED` and the text now says so, because a reader who made the code wait would break the property.
- **D2084** (2026-11-09, T018-071) **A run the engine starts begins a trace.** The OTel export canary found that a run started by the schedule pass, a buffered release or a backfill step had `trace_id` null (only the cron-tick path generated
  one, "as the legacy pass begins it"), and the telemetry projector does not tell a run with no trace, so the accepted operation had no OTel evidence (NFR-018-006). Red first (the run of the schedule was not among the `workflow.run`
  spans the collector received), then `RequestContext::traced()` (`crates/application/src/identity.rs`: a trace the context has is kept, otherwise a fresh `TraceId::generate()`, a UUIDv4: random, 32 lowercase hex, never all zero, nothing of the
  customer's) is called by `start_run`, the one place that every start of the module goes through (the generation of the cron path went: one place). Tests: `request_context.rs` (a trace and a span are kept; a generated id is 32 lowercase hex,
  not zero, and never twice the same), `traces::the_runs_a_schedule_starts_…` (two scheduled runs have two traces, and trigger-now keeps the trace of its request); one hand mutant, `traced()` that replaces a trace the context has: both red.
  **Consequence:** every run a schedule starts is now exported to the collector at the sample ratio (default 1.0), which is more span volume than before for a deployment with many schedules.
- **D2085** (2026-11-09, T018-071) **A refusal at a ceiling is not audited, and that is what the spec says.** FR-018-044 lists what is audited (schedule changes and actions, business-id conflicts, with-start outcomes,
  batch lifecycle and every item effect, resets); a `429 QUOTA_EXCEEDED` is none of those and no audit action exists for it (`AuditAction` has only `quota.changed`). The evidence of "refuses before work" is therefore the
  digest of the rows of the tables before and after the refusal (the audit trail, the receipts, the outbox included): `observability::every_ceiling_…` for the five resources, with the reason, the limit and the series
  `quota_exceeded_total{limit}` of each. An audit record for a refusal would be a decision of the owner (it would let a client fill the trail).
- **D2086** (2026-11-09, T018-076) The table of the bindings of business identifiers is `business_ids` (the brief called it `business_id_bindings`), and the entry of a reset that waits is a row of `run_resets` in state
  `PENDING_SOURCE`: both are among the tables the run-driven test locks, found by name together with every `schedule_*` and `batch_*` table.
- **D2087** (2026-11-09, T018-071) **The two other run-creating engine paths begin a trace too.** The review of D2084 asked which other operations of the engine have no trace. Read in the code: the completion of a pending reset
  (`ResetCompleter::step`, context `default()`) and the item of a batch rerun (`BatchExecutor`, context `default()`) **create runs**, and a run with no trace is never told: red first (`traces::the_run_the_completion_of_a_pending_reset_makes_…`
  and `…the_run_a_batch_rerun_makes_…`: "has no trace id: it is never told to the collector"), then both use `RequestContext::default().traced()`. The operations the engine makes on a run that already has a trace (the cancellation a
  `CANCEL_OTHER` schedule pays, the other item actions, the cancellation of the source of a reset) keep the audit record without a trace id, and the OTel evidence of the operation is the run's own: **G2032**. The backfill step starts its runs
  through `start_run` and is covered by D2084.
- **D2088** (2026-11-09, T018-012) **The step of a backfill locks the row of its backfill in slot 3, as plan row 8 says.** The slot case of the backfill step, run for every start (below), was red when the reference held the backfill row: the step
  took the row in `record_start`, after the admission (authority, quotas, the run), and the plan says schedule row, backfill row, then the admission. `lock_for_admission` now takes the backfill row `FOR UPDATE` right after the schedule row
  (`advance_backfill` finds it locked). It was no deadlock in production as far as read (the other taker of the row, the API's cancel of a backfill, takes the backfill row and then writes its audit record, and takes no lock the step holds), but the order the plan states is the one the code has now, and the case keeps it.
- **D2089** (2026-11-09, T018-012) **The slot cases are run for every start of the reference and prove the order of every pair of the locks the reference names.** The first version held the first lock of the reference and let the path block there:
  it ordered the first lock against the rest and nothing among the rest (moving the authority check after the quota locks stayed green). `against_at(start)` makes the reference hold the lock `start`, so a case runs the path once for every start
  (a schedule, a run, a backfill of its own for each): a path that took the lock `j` before the lock `k < j` holds it while it waits for `k`, and the reference asks for it. `a_start_at_every_lock_finds_the_pair_the_first_start_cannot_see`
  shows it on a scratch table. Two hand mutants in `RunStore::create`, the active-runs quota taken before the authority and the memberships taken before the schedule row: 4 of the 9 cases red each (the schedule paths), restored. A person's start
  has no authority lock (the role of the caller judges it), so the claim of an identifier has none in its reference. Each reference ends in the usage of the tenant (`tenant_retained_usage`, slot 8b, "always last").

- **D2090** (2026-11-09, T018-074) **The browser journeys of this feature are in `frontend/e2e/tests/`, not in `tests/e2e/schedules-bulk-operations/`.** The task text named the Rust e2e target of the feature; the browser workspace is the one CI runs
  (the `studio` job, `cargo test -p sagat-tests --test e2e_workflow_studio -- --ignored journeys`, which starts the platform, the engine and Chromium), and a spec added there is run by it with the rest. A Rust file of the feature would have had to start a
  platform and a browser of its own. The specs: `schedules.spec.ts` (3 journeys), `batch-operations.spec.ts` (4), `reset-run.spec.ts` (4), `business-id.spec.ts` (1); the support: `support/keyboard.ts`, `support/world.ts`. The task text is amended.
- **D2091** (2026-11-09, T018-074) **Keyboard only means a real key for every step, and the rules of WCAG 2.2 AA on every state.** `support/keyboard.ts`: `reach` presses Tab (or Shift+Tab, read from the order of the document) one key at a time until the control holds focus,
  and fails when the page cannot take the keyboard there; `activate`, `setChecked`, `chooseRadio` (arrows) and `choose` (the type-ahead of a native select; leaving the select and coming back first, because the browser keeps what was typed for a second
  and a word typed just before is glued to this one: found when a second `choose` in a row did nothing) use Enter, Space and the arrows; text goes in with `insertText` after Tab, never `fill`. `walk` Tabs once around a page or a dialog and fails on an element with no
  accessible name (read from the browser's accessibility tree) or on focus that leaves a modal dialog. `expectAccessible` runs axe with `wcag2a`, `wcag2aa`, `wcag21a`, `wcag21aa` and also `wcag22aa` (what `a11y.spec.ts` holds the Studio to) on the list, the form, the
  dialog open, the error state and the empty state of each journey. What is not a key: `page.goBack()` (the browser's own Alt+Left), the `visibilitychange` event a person's return to the tab sends (G2033), and the REST calls that set the world up and check
  what the Studio did.
- **D2092** (2026-11-09, T018-074) **The engine of the e2e platform runs the batch pass and the pass of the reset that waits for its source.** `tests/e2e/workflow-studio/studio_journeys.rs` adds `SAGAT_BATCHES_ENABLED`, `SAGAT_BATCHES_POLL_MS=100`,
  `SAGAT_RESET_CANCEL_SOURCE_ENABLED` and `SAGAT_RESET_CANCEL_SOURCE_POLL_MS=100` to the engine's environment (the control plane of the test platform has the reset cancellation on already, `Options::reset_cancel_source`). The schedule pass stays off:
  trigger-now is the control plane's, and no journey waits for a schedule to fire. Nothing in a workflow file changed.
- **D2093** (2026-11-09, T018-074) **A run is held open by an attempt the journey owns, not by a sleep.** To see a reset `PENDING_SOURCE` and then completed, the journey's worker leases the attempt of the source and its handler waits on a promise the journey resolves; a
  run that is cancelled while its attempt is in flight does not end (the platform waits for the attempt), so the entry waits for as long as the journey wants, and the engine completes it when the gate is let go. The same method with `active_runs` of the namespace
  lowered to one, while a second run of the namespace is open, makes the engine's creation of the new run refused (`ADMISSION_REFUSED`) and shows `could not be made: a quota or the platform refused the new run`.
- **D2094** (2026-11-09, T018-074) **A limit of zero is set in a namespace of the journey's own.** `ownNamespace` makes one (the only way to a namespace there is, as in `operations.spec.ts`: there is no delete) and `setQuota` sets `schedules`, `batch_jobs`, `active_runs` or `pending_resets` on
  it through `PUT /namespaces/{id}/quotas/{resource}`; no other journey's quota moves. Every name a journey creates is `<what>-<8 hex digits>`, so that a repetition of the journey (`--repeat-each`) shares nothing with the one before.
- **D2095** (2026-11-09, T018-074; amended the same day by D2096, D2100 and D2101) **What the journeys found in the Studio, each with a vitest test that was red for the right reason first.** The automated rules (axe) found no violation on any screen or dialog state of the journeys. The keyboard and the real platform found four:
  (1) **a finished batch job said `The job took no run.`** and listed no run: the page opened while the job was selecting, read its runs once, and stopped following them when the job ended, so the last read could be older than the job's last step (`BatchPage.test.tsx`
  `reads the runs of the job again when the job ends…`; fix: one more read when the job goes from at work to over). (2) **The links to runs and jobs of the schedule and batch screens were called by the first eight characters of the identifier**, which are the same for everything made
  in the same minute (the identifiers are UUIDv7): a table of runs whose links are all `01a1c079` cannot be told apart by sight or by ear. Every abbreviation is now `shortId`, the last eight (D2100). (3) **Focus fell to the top of the page** when the button that opened a dialog was gone by the time it closed (Pause, Resume,
  Cancel the job): D2101. (4) **After a stale revision the page the message sends the person back to still showed the old one** for up to five seconds, and the delete dialog's "look at it again" did not read it again: the schedule's page is marked out of date on a `REVISION_CONFLICT`; **the first version of that fix was a defect of its own** (D2096).
- **D2096** (2026-11-09, T018-074; review blocker B1) **The first fix of D2095 (4) defeated optimistic concurrency, and was found by review, not by the journeys.** It invalidated `['schedule', ns, id]` by prefix, which is also the first part of the edit form's own key (`[..., 'form']`, `staleTime: Infinity`, `gcTime: 0` so that the revision a person saves against never moves under them); an
  active query is refetched whatever its `staleTime`, so after a `409` the form's copy silently became the other person's revision, the draft and the banner stayed, and pressing Save again sent `expectedRevision` = the new revision, which the platform accepted: the lost update the `409` exists to stop. The delete dialog had the same defect (it deleted at `schedule.revision`, which the refetch behind
  the dialog had moved). **Red first** (against the code of D2095): `ScheduleFormPage.test.tsx` `keeps asking at the revision the person opened…` (the two PUTs carried `[3, 4]`, not `[3, 3]`) and `SchedulePage.test.tsx` `asks again at the revision the person was refused at…` (`['9', '10']`, not `['9', '9']`); and the journey step (`schedules.spec.ts`: after the banner, Save is pressed again with the keyboard, the platform must answer `409` with the original
  `expectedRevision`): against the old code the second save answered **200**, in the real stack. Fix: the form freezes the schedule it opened (`useState`), the invalidation is `{ exact: true, refetchType: 'none' }` on the page's key only, the delete dialog keeps the revision of the moment it opened, and every other invalidation of a schedule's page (pause, resume, run now, save) is `exact` too. The other keys
  (`schedule-backfills`, `schedule-actions`) have no sibling of another kind under their prefix (the outcome filter is the last element of the actions' key on purpose).
- **D2097** (2026-11-09, T018-074; review T2) **The `us04-act` failure is a counter shared by every run the worker serves, not a lease that lapsed.** The hypothesis was a lease lapsing and the same attempt being delivered twice. **Disproved by a repro**: with the engine's lease at 2 s and a worker with no heartbeat whose first call of `limit.release` stalls 6 s, the journal says `step.leased:reserve~c:1` then
  `step.needs_review:reserve~c:1`: the lapse sends the compensation to review (the run is `MANUAL_REVIEW`, which is what the journey wants), the handler ran **once** (`call 1 attempt 1`), with the counter fixture and with an attempt-keyed one. **Reproduced instead**: a worker serves every run of its namespace that waits for an activity, so a payment-saga run that nobody served yet (an earlier journey left one) reaches the same
  `limit.release` handler as the run of the journey, and `release += 1; if (release === 1) throw` hands the failure to whichever of the two comes first: in two runs of the repro one of the two runs was `COMPENSATED` each time, the journey's own in one of them (a race between two runs, which is why it is intermittent); with `failsFirstAttempt` (keyed on `ctx.runId` and `ctx.attempt`, `support/worker.ts`) both were `MANUAL_REVIEW` in both. The fixture of US04 uses it now; no other
  fixture counts invocations (`unknownOriginalRun` and `unknownCompensationRun` always fail), and the gate of the reset journey (`reset-run.spec.ts`) was a counter too and is keyed on the run and the attempt. **Not established:** which run was left over in the two failures (CI's earlier run, and the first whole run here, load average 9 to 10): at the start of `us04` the next two whole runs listed three runs open (the foreign sentinel and `sec-text` of the security spec, and a run my reset journey left) and two in review (mine), none a
  payment saga, so the leftover that did it was not on the machine then. The journeys of this feature now cancel the runs they leave open (D2105).
- **D2098** (2026-11-09, T018-074; review M1) **`choose` no longer types into a closed select.** The type-ahead of a native select keeps what was typed for a second and each character can fire `onChange`: on a loaded runner one stall of the main thread splits the word (about forty uses, no retries in CI). `choose` waits for the select to be enabled and to hold the option (the Runs "Workflow" select is disabled until the workflows are read),
  Tabs to it, checks in the accessibility tree that it is a combobox with a name, and sets the value with `selectOption`, retried until the select says it holds it. What it proves, and what it does not (how the browser operates a closed select is the browser's), is said in the header of `support/keyboard.ts`.
- **D2099** (2026-11-09, T018-074; review M2) **The attempts a journey holds in flight cannot time out under it.** `HOLD_MS` is 600,000 (above the 240 s of any journey): a held attempt that the platform timed out would be an unknown outcome and a run in review, not a cancelled one. The reads of what the engine does (`expect.poll` of the reset pass, `landedOn`) are bounded at 60 s, as `api.until` is.
- **D2100** (2026-11-09, T018-074; review M3) **Every abbreviation of an identifier is `shortId`.** `RunHeader` (the link back to the source, the link to the parent, the fallback name of a workflow), `RunsPage`, `StartRun`, `queries` (the fallback of a version), `SchedulesPage`, `SchedulePage`, `ScheduleFormPage` (the fallback of a workflow) and the batch and decision links now call it, so the same run is `466f8152` in the list, in the header link and in the announcement. Tests that read a prefix changed: `RunDetailPage.test.tsx` (the link back
  to the source and the link to the parent: `run 000000aa`, `run 000000ee`, not `run 019b5d6e`) and `RunsPage.test.tsx` (`workflow …` for a workflow that could not be listed); `display.test.ts` has the unit test of `shortId`. Looked for elsewhere: `apps/cli/src/worker.rs:229` prints `run <first eight characters>` in the output of `sagat worker` (G2039); no runbook or doc abbreviates an identifier (the two hits are the key id of a script signing key and the advice to fingerprint a trace id).
- **D2101** (2026-11-09, T018-074; review M4) **Focus after Pause, Resume and the cancellation of a job is not a flag.** The first version moved focus from an effect that a flag armed; the flag stayed armed when `paused` did not flip, so the next flip (another operator's pause, read on the 15 s round) stole focus; it took the first enabled button of the group, which for a role without `run:start` may be Run now or none; and it worked only because `Pause` and its open dialog
  unmounted in the commit that flipped `paused`. Now Pause and Resume are one button (`Toggle`): the dialog closes over the very button that opened it, whatever the schedule has become, and `Dialog` gives focus back to what had it when it opened (the browser does it on `close()`; this makes it the component's behaviour, the same in a test). A role that may pause and not resume has no button once the schedule is paused: focus goes to the heading of the schedule (`#schedule-title`,
  `tabIndex={-1}`), as it goes to the heading of a job that is being cancelled. Tests red first: `SchedulePage.test.tsx` `moves focus only for the pause or the resume the person made…` (focus was stolen from the Edit link) and `puts focus on the heading when the control … is not for this person…`.
  **The same component had a race, found by the backfill journey** (it failed in three runs of five, the same step, and passed in the other two): `Dialog` treated every `close` event as the dialog closing itself when the owner still held it open. The browser fires `close` in a task of its own after `close()`, so a person who pressed Escape and then Enter on the opener before that task ran reopened the dialog and the stale event closed it again (`Dialog.test.tsx` `is not closed again by the close event of an earlier closing…`, red first: the owner was told twice). It counts now only if the element is closed at that moment.
- **D2102** (2026-11-09, T018-074; review minors) **What the journeys' support now says.** (a) `withoutRefusals(problems, refusals, allowed)`: the browser's "Failed to load resource" lines are dropped only for the statuses a journey allows (409, 429), recorded as the page gets them (`recordRefusals`); a 404, a 403 or a 5xx fails the journey. (b) `inspectDialog` proves the trap (Tab and Shift+Tab pressed more times than the dialog has controls never put focus behind it) and walks the dialog for names; it is called on every dialog of the journeys: pause, resume, run now, delete, backfill, start, batch, cancel of a job, reset (both kinds). (c) The counts are anchored (`/^3 runs match\./`, `/^Kept as it was: a$/`).
  (d) One `expectAccessible` and one list of WCAG tags (`support/accessibility.ts`, used by `a11y.spec.ts` too), with no way to exclude a node; the sign-in is `signIn` of `support/studio.ts`, which takes the controls it is operated with (the keyboard's, here). (e) The filler `expect(workflow.id).toBeTruthy()` is gone. (f) One protocol session per page for the names read from the accessibility tree.
- **D2103** (2026-11-09, T018-074; review T1) **The backfill and the delay of a start have journeys; the schedule pass is on in the e2e engine.** `SAGAT_SCHEDULES_ENABLED` and `SAGAT_SCHEDULES_POLL_MS=100` join the other loops in `studio_journeys.rs` (a backfill is the schedule pass's; no journey publishes a cron trigger but the one that owns a schedule, whose firing is a year off). `schedules.spec.ts` backfill: the dialog, a range that is wrong in words (empty, in the future), a range of the three whole hours that ended last
  (`3 started a run`, `Completed`, the three decisions of kind `Backfill`), then the same range again (`3 already acted on`). It found a defect, fixed with a vitest test red first: **the decisions and counters of the schedule were not read again when a backfill went on**, so the history under a backfill that said `3 started a run` could be empty (`SchedulePage.test.tsx`, `Backfills.tsx`). `business-id.spec.ts` `START DELAY`: a delay that is not a number in words, one minute (the least), the run held
  (`Starts at` is the platform's `startDelayedUntil`, the status is `Waiting`, the list says `Starts …`) and then released and `Succeeded` (the journey takes a little over a minute: there is no way to release a start before its time). **No preview journey**: G1966, the platform has no route that counts a backfill's times. **No cancel-of-a-backfill journey**: the cancel of `Backfills.tsx` is in `SchedulePage.test.tsx` only (G2038).
- **D2104** (2026-11-09, T018-074; review g, h) **The guard of the harness is 90 minutes, and a journey's time can be read.** `studio_journeys.rs` ended the run at 40 minutes, less than 14 journeys at the 240 s each declares (56) plus the older specs (about three minutes when they pass); it is 90 now, a guard against a hang and not a budget, and the `studio` job of CI ends at 60 whatever it says (`timeout-minutes`, not touched). `STUDIO_REPORTER=list` prints the time of each journey (`playwright.config.ts`).
- **D2105** (2026-11-09, T018-074) **A journey cancels the runs it leaves open** (`cancelAfterwards`, `test.afterEach(cancelLeftovers)`): runs whose step waits for a worker nobody runs, and the run a reset made after its worker stopped. A run in review refuses a cancellation by design, so the two `batch-unknown` runs stay in review (G2040).
- **D2106** (2026-11-09, T018-072) **Every scenario of the benchmark asserts bounds, and a bound the data exceeds fails the run and is reported, not loosened.** NFR-018-005 promises nothing ("measured on a declared environment"), so the bounds are the ones the repository already states or the ones a reader would hold it to, each with its reason in the scenario's module: firing (`SagatSchedulerLate`, `max(scheduler_lag_seconds) > 60` for 5 minutes): the p95 of the lag within 60 s and the gauge not above 60 s for 5 minutes, no step refused or failed; batch: a job reaches 80% of the rate it asked for and never more than 110% (the rate is a ceiling the executor keeps), with several jobs none reaches less than half its rate, none waits 10 s for its first item and jobs of one size end within a factor of two, a selection or a drain of N runs takes less than N / 200 s (the time the highest rate would take to apply them); claim: the outcome exact in every round (a hard check), the p99 of a start that waits for the holder within 1 s, the claim within 1.5 times a plain start at the median and 2 times at the p95; reset: the largest accepted prefix within 5 s (the statement timeout of the pool) and holding the source row for no more than 1 s, the refusal beyond the ceiling typed, creating no run and answered within 1 s; triggers: every statement within the 4 s statement timeout the code sets (the check the handover makes under its lock on a converged fleet included), the count within 2 s and a window within 1 s (the bounds of `trigger_scale.rs`), a reconciliation whose rounds are full keeping half the 100 writes a second it advertises. The report says for each bound how many of the repetitions held it.
- **D2107** (2026-11-09, T018-072) **Method.** A release build, the runtime role (`SAGAT_TEST_AS_RUNTIME_ROLE=1`, the role the engine is required to run as), a PostgreSQL 16 of its own that `run.sh` starts from the image pinned by digest with the image's defaults (`fsync` and `synchronous_commit` on, `shared_buffers` 128 MB) and `max_connections=200` (the testcontainers default starts the same image with `fsync` off, which would make every commit cheaper than a deployment's), loopback only and removed when the script ends. Each scenario is a process of its own on a fresh database and is repeated three times (N = 3: a median and a spread, never one run); the load average, `docker stats` of the container and the commit latency of the database (a hundred one-row transactions) are recorded before and after each repetition; each fragment carries the machine, the commit, the settings and the bounds. The sizes of a scenario come from the environment and default to a smoke run, so that an ignored scenario started by hand does not run for half an hour.
- **D2108** (2026-11-09, T018-072) **The benchmark does not run in a pull request.** The target `perf_schedules_bulk_operations` is selected by `scripts/select-test-targets.py` when a file under `tests/performance/schedules-bulk-operations/` or its `[[test]]` entry changes (it picks that target and nothing else for the files of this change), and the CI step that runs a selected target is `cargo test -p sagat-tests --test <target>` without `--ignored`: every scenario is `#[ignore]` and what runs is the four unit tests of `measure.rs` (percentile, summary, bound, sizes; 0.00 s, no Docker). `run.sh` is not called by any workflow and refuses to run when `CI` or `GITHUB_ACTIONS` is set. `.github/` is untouched.
- **D2109** (2026-11-09, T018-072) **How the firing is measured.** The schedules are one-minute intervals with an offset in the minute (zero for all: the same instant; spread evenly over 60 s: the spread case), `ALLOW_ALL`, one action each (`remainingActions` 1), so that a burst is one time of every schedule and not a load that goes on every minute; the first time is a whole minute 10 to 70 s ahead by the clock of the database (the nominal times are occurrences of the spec, which the firing recomputes and refuses to act on otherwise: a first attempt with times that were not on the grid acted on nothing), re-timed by one `UPDATE` after the schedules are made through the service, and the passes are the engine's own loops (`loops::run_schedules` and `schedule_round`, the settings of the engine's defaults, a pool of 16 for each replica) already up and idle. The lag is read three ways: `schedule_action_lag_seconds` as the process records it, `decided_at` less the nominal time, and when a separate connection polling every 100 ms first saw a run for 50, 95 and 100% of the schedules; the gauge `scheduler_lag_seconds{pass="schedules"}` is read from the same sample the monitor reads, once a second. The quota of active runs (10,000 a namespace by default) is raised for every scenario: the first attempt at 10,000 schedules stopped at 9,998 because the 10,000 runs that never finish met it, and the pass went on refusing the admissions of the second time of each schedule (the engine answers a refused admission with `ADMISSION_RETRY` and tries again), which is a property of the default quota and not of the pass.
- **D2110** (2026-11-09, T018-072) **Reset sources are built by the engine, not written.** A run with 1,000, 5,000 and 20,000 events is a chain of activities (one activity name, so that one worker session serves all of them) completed one after the other through the worker service, which makes four events a step; every run a reset makes is checked with `verify_run` (replay equals projection). The time the source row is locked is read by a probe that asks for it with `FOR UPDATE SKIP LOCKED` in a tight loop on a connection of its own while the reset runs (its resolution is one round trip, about a millisecond, and each probe takes the row for the instant of its statement).
- **D2111** (2026-11-09, T018-072) **The trigger tables are seeded in bulk, and say how.** The steady state (a schedule equal to every trigger but one in four thousand, 200 invented tenants) is written by `INSERT ... SELECT` over `generate_series` as the owner, with the foreign keys and the user triggers of the three tables off for the writing session, as `trigger_scale.rs` does (the per-row guards and counters are what make a bulk write minutes long); the fleet is then given a schedule for every trigger by the same statements, so that the check under the handover's lock (which must look at every trigger and find none) is measured on the state the handover needs, at 5,000 to 100,000 triggers; the state after an upgrade (no schedule for any trigger) is written the same way from the trigger a real publication made for each of the two real tenants, with the guards on, so that the reconciliation really writes the schedules; the engine's own loop (`loops::run_triggers`, its pacing of a pause after a round that wrote) then works through the backlog for a fixed time and the writes a second are measured.
- **D2112** (2026-11-09, T018-072) **No production code and no ceiling was changed by this task.** The findings that follow from the data are registered as gaps (G2043 to G2050), each with what would change it, and each is left to a decision of its own: a ceiling is changed only by a decision that quotes the data, and none of the changes the data suggests (the pause of the batch loop, the paging of the restore of a reset, the plan of the handover check, the runbooks) is small enough to be made, and tested in both roles, inside a task that measures. The one decision about a ceiling that the data supports is to leave `RESET_PREFIX_EVENTS` at 5,000: the source row is locked for 4.65 s at that size, and 20,000 must not be accepted before the restore is paged (G2046).
- **D2113** (2026-11-09, T018-072) **The attempts that did not count are kept out of the report and named in it.** The first launch at 10,000 schedules (stopped by the default active-run quota, G2045), the trigger backlog scenario twice (an assertion of the harness that compared the writes of the rounds with a count that is capped at 100,000 and with a count that excludes the repairs of the two triggers a real publication made), the first steady-state scenario (replaced by one that also measures the converged fleet, because the check under the handover's lock turned out to depend on the size) and the first three repetitions of the firing at the same instant (replaced to record the outcomes of the actions) are not in the tables; the report says so under "How the run went", with the difference the replaced firing numbers made (within 6%). The bounds were not changed by any of them.
- **D2116** (2026-11-09, T018-072 follow-up, G2043) **The batch loop sleeps until the soonest slot of a job, never longer than its poll.** **[Amended the same day by D2123 to D2126, after two reviews: the statement that finds the slot is folded into the scan of the due jobs, which reads the database's clock once; only a running job's slot counts; `MIN_SLOT_WAKE` is private; a due job that the executor finds not due yet comes back in 10 ms. What follows is the first design and its measurements; where it says `BatchStore::next_slot_in`, read the second half of `due_jobs`.]** *The defect:* after an item a job's next slot is `1 / rate` ahead (50 ms at the default 20 a second), the round that follows finds nothing due, did no work, and the loop rested its whole poll (1,000 ms): one item a second. *The change:* `BatchRound` has a field `next_due_in: Option<Duration>`, set by `BatchPass::round` **only for a round that did nothing and met no refusal and no failure**, from a new port method `BatchStore::next_slot_in(shards)` (`PgBatchStore`: one statement, `unnest(shards)` cross-joined with a `LATERAL (... ORDER BY next_slot_at LIMIT 1)` over `batch_jobs_due_idx (shard, next_slot_at)` for the states `SELECTING`, `RUNNING` and `CANCELLING`, `next_slot_at > statement_timestamp()`; the answer is `min(slot) - clock_timestamp()` **by the database's clock**, so no clock of this process is involved, clamped between 0 and a day). `loops::run_batches` rests `min(jittered(poll), max(next_due_in, MIN_SLOT_WAKE))` (`idle_pause`; `MIN_SLOT_WAKE` is 1 ms): the poll keeps its jitter and is still the longest the loop rests, a slot is not jittered (it is durable and each job's own: the jitter only keeps replicas that started together apart), a slot that has come or comes within a hair is still a wait of 1 ms and never a loop with no pause, and the rest is `stop_during`, so a stop ends it. A round that did work is followed at once, as before, and does not ask. *Its cost, said:* the statement is at most 64 index probes (one for each shard the replica holds), each reading the first entry of the index after the present (0.34 ms with no job at all in the plan I looked at, `Index Only Scan` with `Heap Fetches: 0`); it is asked **only by a round that found nothing due**, which is one round a poll for an engine with no work and, for a job that applies items, one idle round after each item (a clock read, the due scan and this statement); no round that did work asks, and the round does not scan the active jobs. The jobs that the rest of the loop already left to the poll stay there: a selection waiting for a transaction and a job whose items are all deferred have their slot in the past, so they are not the hint, and a finished job's last slot, which the last item moves ahead too, is not counted (the state filter). *Not changed:* the executor (never catches up in a burst, one item of a job a step), the order of the due scan (`next_slot_at, id`), the fairness, the ceilings of a round, the fences, the rate. *Tests, red first:* 7 unit tests of the loop under `tokio::time::pause` (the tokio `test-util` feature is a dev-dependency of the engine): a round that says "50 ms" wakes the loop at 50 ms and not at the poll (it slept 1.13 s before); no hint sleeps the poll jittered; a slot an hour ahead is still the poll; a round that worked is followed at once; a stop during the wait ends the loop; 2,000 rounds that answer "now" take at least 1,999 ms (no loop with no pause: with no floor they would take none); the pure function of the pause with a fixed entropy. 3 integration tests in both roles (`batch_pass.rs`): the store for a job 30 s ahead (29 to 30 s), for a shard that is not the replica's, for none and for a finished job; the pass (a round that applied items says nothing, the idle one after it says 1 s or less for a job at one item a second, and nothing once the job is over); and **the engine's own loop with a poll of 10 s completes the four items of a lone job at the default rate within 5 s** (it timed out at 5 s before the change; the earliest a jittered poll of 10 s would wake it is 8 s). *Hand mutants:* the loop ignores the hint (killed by the wake-up test), no floor (killed by the two tests that name it), the pass does not ask (red before the method was written: the three integration tests), the state filter of the statement removed (killed by the store test and the pass test: a finished job's slot was the answer), the pass asks also after a round that worked (killed by the pass test); each restored and the tests green again. *Measured* (the benchmark's own scenario of the lone job, release build, 3 of 3 repetitions, 120 runs at the default rate): **18.3 and 18.4 items a second in two runs of three repetitions (the machine at a load of 20 to 34 and then of 5), against 0.97 to 0.98 before; the gap between two items 54.4 to 54.6 ms at the median against 1,036 ms** (the 1.6 items a second that are missing from 20 are the round that asks for the slot and the wake-up, about 4 ms an item: G2051; the addendum of the report has the load of each repetition). The bound "reaches 80% of the rate it asked for" held in 6 of 6 repetitions (it was broken in 3 of 3). The scenarios that must not change were rerun: sixteen jobs (73.5 items a second in all, the last job's end over the first's 1.00), one job of 900 runs beside fifteen of 30 (72.0, 1.04), a lone job at the highest rate (73.1); the fairness bounds held in 3 of 3 and the capacity bounds stay broken as before.
- **D2117** (2026-11-09, T018-072 follow-up, G2049) **The drift view looks the schedule up for each trigger, so its cost does not depend on the plan the statistics give it.** *What I found* (`EXPLAIN (ANALYZE, BUFFERS)` of `SELECT EXISTS (SELECT 1 FROM schedule_trigger_drift)` on the benchmark's own database at 10,000 triggers, PostgreSQL 16.15, the same image and settings as the report): the tables **were** analyzed, after the bulk insert and again after the stragglers were converged (`last_analyze` seconds before, `n_mod_since_analyze` 0 and 2), so the bulk insert's missing statistics are **no part** of it. The plan was `Nested Loop Left Join ... Join Filter: (s.trigger_id = t.id)` over a `Materialize` of a sequential scan of `schedules`, `Rows Removed by Join Filter: 99990000`, 30,497 buffers, 3.1 s with `ANALYZE`. The estimate was `rows=6069500` for the outer join: 10,000 triggers times **610 rows for `schedule_trigger_handover`**, a one-row table that is never analyzed (`reltuples = -1`: the planner's default for a table it knows nothing of is 10 pages) times 0.995 (the planner's estimate of `CASE ... IS NOT NULL`). An `EXISTS` is planned for its first row, so with six million estimated rows the cost of a plan is its start-up cost, and a nested loop with no start-up beat the hash join (423 to start) whatever its total (1.7 million). **`ANALYZE schedule_trigger_handover` alone, nothing else, brought the same statement to 21.5 ms with `Index Scan using schedules_trigger_uniq` (60,174 buffers).** At 15,000 triggers the estimates cross and the planner picks the index plan, which is why the cost looked like it followed the size. In a deployment the singleton is as unanalyzed as here (a row inserted by a migration and updated once). *What was changed* (in place in `20270114000000_trigger_handover`, never applied outside tests and not released; its `down` is as it was): the view reads the schedule by `LEFT JOIN LATERAL (SELECT ... FROM schedules x LEFT JOIN schedule_revisions r ... WHERE x.trigger_id = t.id LIMIT 1) s ON true` (a trigger has at most one schedule: `schedules_trigger_uniq`) and the singleton by a scalar subquery inside the `CASE` (an uncorrelated `InitPlan`, evaluated once, and the table has exactly one row: a primary key on a boolean with a check, no delete, no truncate), so the plan is **an index probe for each trigger** and no statistic, missing or stale, offers the planner the join that it rescanned. The form does not by itself rule out a per-trigger sequential scan with a limit (PostgreSQL does not take a partial unique index as proof that a trigger has one schedule, and with no statistics it would estimate about half a percent of the rows): **`trigger_plan.rs` is what holds the plan**, by its assertions on the shape of the plan in three states of the statistics, and it would fail on that plan too. The view stays the ONE definition read by the reconciliation (`DRIFT`), the count of the gauge and the dry run (`schedule_trigger_convergence`) and the check under the lock (`schedule_trigger_unconverged`): their SQL is unchanged. *Not done, and why:* an `ANALYZE` by the command before it takes the lock, or in the migration: it would make the form a plan the planner may still choose badly for another reason and would take a lock the command does not need to take; the singleton's statistic is now irrelevant. *Tests, red first, in both roles:* `trigger_plan.rs`: at 10,000 triggers never analyzed and at 5,000 analyzed before they were written (the singleton unanalyzed in both) the plan of the check holds no sequential scan of `schedules` or `schedule_revisions` and uses `schedules_trigger_uniq`, and the check answers within a second (a machine ten times slower than this one still does; the old plan was 2.85 s here), in three states of the statistics each (none, the big tables analyzed, everything analyzed); and the view lists, with the previous definition kept in the test as an oracle, **exactly the same rows, columns and kinds for 1,700 triggers in seventeen classes (eleven here, six more by D2128)** (planned, no schedule, deleted, three kinds of respec, no revision row, paused, not planned, exhausted, and the two precedences: deleted before respec, respec before paused) in the states `LEGACY` and `HANDED_OVER` (the latter in a rolled-back transaction: the state has no reverse), the count of the gauge equal (1,300) and the check true. *Hand mutants:* the expression compared at `{cron,1}` (killed by the oracle test), the pause listed in the state `HANDED_OVER` and not `LEGACY` (killed). *Measured* (the benchmark's scenarios `triggers-steady-small`, `triggers-steady` and `triggers-steady-mid`, release build, 3 repetitions each, a converged fleet, p50 of the check under `EXISTS`, before → after): 5,000 triggers 1,009 → 10 ms; 10,000 2,853 → 19.7 ms; 15,000 28.0 → 28.2 ms; 20,000 43.5 → 39.2 ms; 50,000 111 → 112 ms; 100,000 280 → 263 ms (the slowest sample 344 → 335 ms: over the 300 ms that was asked for by 12%, and what the old index plan gave there); the windows and the whole sweep of 100,000 triggers are faster (86 → 48 ms a window, 9.1 → 2.4 s), the convergence count is slower up to 20,000 triggers (10 → 19 ms at 10,000) and faster from 50,000 up, and an unbounded `count(*)` of the view, which nothing in the product issues, is 1.3 to 2 times slower. The statement timeout of 4 s and the lock timeout of 2 s hold with a margin of two orders of magnitude to 100,000 triggers; the cost is now linear in the number of triggers (G2052).
- **D2118** (2026-11-09, G2048) **The reconciliation's "100 writes a second" is a ceiling, and the documents say "at most" and the measured range.** The pause after a round that wrote (`TriggerRound::pause_after`) is added to the time of the round, so the pace is lower than 100: measured with two tenants and the engine's own loop for 60 s, **per size**: 100,000 triggers 62 a second under the default settings (27 minutes) and 77 with the per-tenant cap of a round raised in code to 100 (21 minutes); 10,000 triggers 30 and 69 a second (6 and 2 minutes). The two sizes differ under the defaults because the sweep over a small backlog restarts and a round that finds nothing to write is followed by the poll of 5 s (7 or 8 such rounds in the 60 s at 10,000, against none that mattered at 100,000): read from the counts of the rounds in the report, not tested alone. (An earlier text said "30 to 62 a second ... 27 minutes": the 30 is the 10,000-trigger scenario and at 30 a second 100,000 triggers would take 55 minutes.) Corrected: ADR-0083 (decision 7, the consequences and the estimate), `docs/runbooks/schedules.md` (the reconciliation paragraph and step 3 of 6.2), the notes of D2047 and G2023 in this file, and the doc comments of `TriggerRound::WRITES_PER_SECOND` and `TriggerSettings::MAX_BATCH`. The pause of the pass that expires `CANCEL_SOURCE` entries (`ResetRound::pause_after`, "a hundred a second") was not measured and is not touched.
- **D2119** (2026-11-09, G2044, G2045) **The capacity of one replica and the quota of active runs are in the schedules runbook, as measurements.** `docs/runbooks/schedules.md` 4.1 has the table of the report (a burst of N schedules due together is late by about N / 61 s at the last, two replicas about halve it, spread over a minute 1,000 are kept up with and 10,000 are not, the round ceilings are not a lever), says in its first lines that the figures are one machine on one day and not promises (NFR-018-005) and names what was not measured; it also names the default quota of 10,000 active runs, `ADMISSION_RETRY`, the metric and what an operator sees (a lag that grows without end and `SagatSchedulerLate` that fires on it and not on its cause), and section 3 points to it. "About three replicas or more for 10,000 per-minute schedules" is an extrapolation and is called one.
- **D2120** (2026-11-09, G2046, G2047) **The reset runbook says what the lock time and the replay cost mean for the person who runs it** (`docs/runbooks/run-reset.md` 2.1): a reset holds its source row for about 0.93 ms an event (4.65 s at the 5,000 the ceiling accepts, 0.15 to 0.24 s inside the 5 s statement timeout of the pool), the points, the preview and the reset replay the whole journal at a cost more than linear in its length (9 ms at 1,002 events, 89 ms at 5,002, 1.3 s at 20,002), a refusal past the ceiling is cheap (46 ms), and the ceiling is not to be raised before the restore is paged. No code changed: the restore is still one statement an event and the replay is still whole (G2046, G2047 stay open).
- **D2121** (2026-11-09, G2043, G2044) **The batch runbook says what a job gets** (`docs/runbooks/batch-operations.md` 2.1): the rate is a ceiling; a lone job at the default rate now applies 18.4 items a second (1.0 before D2116); one replica applies about 73 to 75 items a second to all its jobs together and shares them fairly (sixteen jobs end within 1% of each other, none waited more than 0.4 s for its first item, a job of 900 runs beside fifteen of 30 held no small one back), so sixteen jobs at the default rate get about 4.5 a second each and a job at the highest rate about 75, not 200 (the two bounds of the benchmark that stay broken are the replica's capacity, not the loop: G2044); how long a job takes is its runs over `min(rate, 73 / the active jobs on the replica)`. The batch pass on two replicas was not measured (G2050).
- **D2122** (2026-11-09) **The benchmark is rerun for what changed, and its report gets an addendum.** The original tables, bounds and findings of `tests/performance/reports/schedules-bulk-operations.md` are not rewritten (D2106, D2112: they are what the unchanged code did); the section "After the fixes of 2026-11-09" has the scenarios rerun with the same `run.sh`, binary profile, database image and settings, three repetitions each, the load of the machine before and after each (it was 20 to 34 for the first run of the lone job, after a rebuild, so that scenario was run again at a load of 5, and both are shown), and says which bounds that were broken now hold and which still do not and why. The scenarios rerun are the ones that the changes touch (the lone job at the default rate, the checks of the handover at 5,000, 10,000, 15,000, 20,000, 50,000 and 100,000 triggers) and the ones that guard what must not change (the sixteen jobs at the default rate, the lone job at the highest rate, one big job beside fifteen small ones: fairness and the replica's capacity). The firing, the claim, the reset, the selection and the drain were not rerun: nothing they read was changed. The windows of the reconciliation read the view, so the scenario that drains a backlog (`triggers-backlog`, about six and a half minutes a repetition) was run **once** (N = 1, a point and not a median, and the addendum says so): the count and the first window of a backlog are three to four times faster at 100,000 triggers and the drain is at the same pace (66.7 and 77.3 writes a second at 100,000, 25 and 21 minutes).
- **D2123** (2026-11-09, G2043, the correctness and the architecture review, A1) **The slot ahead is found in the statement that finds the due jobs, at the instant that statement reads the clock, so a slot cannot come between the two and be lost.** The first design asked for the slot in a second statement (`next_slot_in`, with the bound `next_slot_at > statement_timestamp()`): the scan could find a job's slot 1 ms ahead, the round do nothing, and by the time the second statement ran the slot had come, so it answered nothing and the loop slept its whole poll while an item was due (reasoned from the code by both reviewers, not reproduced: the exposure is highest at 100 to 200 items a second, where the cost of an item is near the slot, and the benchmark at 20 a second did not show it). *Now:* `BatchStore::due_jobs` returns `DueJobs { jobs, next_slot_in }` from **one statement**: a CTE `t0` reads `clock_timestamp()` once; a job is due when its slot is not after `t0`, and the slot ahead is the first one after `t0` over the running jobs of each shard (one index probe a shard, 64 at the most, `Index Cond: (shard = s.shard) AND (next_slot_at > $t0)` on `batch_jobs_due_idx`, checked with `EXPLAIN` on a scratch database), measured at the end of the statement by the same clock and never below zero: **a slot that comes while the statement runs, or after it, is ahead of the instant the round was decided at, and is a zero, which the loop floors to 1 ms**. A slot that had already come at `t0` and is not due (a selection waiting for a transaction, items all deferred) is in neither answer, so such a job still cannot make the loop spin. `next_slot_in` is gone from the port. The due scan's own bound on the slot is now an index condition too (it was a volatile call per row). The scan costs 64 index probes more on every round, in the same statement, and one round trip less on an idle one. *Tests, red first:* `a_slot_that_comes_between_the_statements_of_an_idle_round_is_not_lost` (`batch_pass.rs`, with `gap_store.rs`: the real store, which makes a job's slot come right after the scan has answered and before the caller has seen the answer: no sleep, the store does the move; red against the two-statement design: "the slot that came while the round was between its statements was lost"); and `a_slot_that_comes_while_the_scan_runs_is_a_zero_and_not_lost`, which gives the statement the instant it was decided at (`PgBatchStore::due_jobs_as_of`, a seam the production call does not use: it passes none and the statement reads the clock itself) taken before a slot comes, and holds that the slot is ahead of it and zero (a lock on the table of items would hold the statement back before it reads the clock, not after: relation locks are taken before execution, which is why the first attempt, with a held lock, did not hold the window). *Hand mutants:* the bound of the slot ahead taken from `clock_timestamp()` at the later look instead of `t0` (killed by the second test; it survives the first: the window it closes is the statement's own), the two-statement design (killed by the first), the state filter removed (killed), the pass asking also after a round that worked (killed).
- **D2124** (2026-11-09, G2043, A2) **A due job that the executor finds not due yet is looked at again in 10 ms, not after the poll.** The scan decides by the database's clock; the executor decides `now < job.next_slot_at` by `DatabaseClock::now()`, which is read at the start of the round and is behind by up to half a round trip, and by whatever it has drifted since: the loop woke at the slot plus a millisecond, the scan said due, the step said idle, and the next look found a slot that had passed and said nothing, a whole poll again (a miss costs one jittered poll in production, and the integration test of the loop with a poll of 10 s and a bound of 5 s would have gone red on a slow runner). *Now:* a round that did nothing, met no refusal and no failure, and in which a step found a due job with nothing to do (`found_idle`) says `DUE_BUT_NOT_YET`, 10 ms, or the slot ahead when that is sooner (`idle_hint`, a pure function with its unit tests). It cannot spin: the job leaves the scan as soon as the engine's clock agrees; a round costs several statements; the loop never rests less than 1 ms; so it is 100 rounds a second at the most, and only while the clocks disagree. A due job that no step looked at (its shard's lease lapsed between the scan and the step) is **not** this case and is left to the poll: the keeper renews it, and looking again every 10 ms would be a hot loop for as long as it takes. *Tests, red first:* `a_due_job_that_the_executor_finds_not_due_yet_is_looked_at_again_in_a_few_milliseconds` (an executor whose clock reads an hour before the database's: a round with `due: 1, steps: 0` and no refusal; red: no hint) and three unit tests of `idle_hint` (the cases above and the rounds that worked, were refused or failed). *Hand mutant:* the hint whenever a job was due, stepped or not (killed by the lease case of the unit test).
- **D2125** (2026-11-09, G2043, A3, A4) **Hygiene of the wake-up.** (a) Only a **running** job's slot is ahead: an applied item moves it; a job in selection is due from its creation (or waits for a transaction, which no slot is for) and one that is being cancelled is moved back to its request; the `SELECTING` and `CANCELLING` entries of the state list were vestigial and are gone, with `a_slot_ahead_of_a_job_that_is_not_running_is_not_a_wake_up` holding it. (b) `MIN_SLOT_WAKE` is private to `loops.rs` (it was `pub const`, used only there and in its tests). (c) The tests that look at the time to a slot use the slowest rate the code names (`rate_milli` 1, one item in 1,000 s: `slowest`), so that no test races a slot: the one that asserted `(due, steps) == (0, 0)` after an item at one a second held only if less than a second passed between two rounds. (d) The jitter of the loop is covered by `the_idle_pause_is_the_poll_or_the_slot_whichever_comes_first` with a fixed entropy (80 %, 100 %), so removing the jitter from a slot's wake-up is held there, and the paused-time test of the poll keeps the range (800 to 1,200 ms) the system's entropy gives.
- **D2126** (2026-11-09, A, the benchmark) **The lone-job scenario is rerun after the fold, and it stays at about 18 a second.** See the second addendum of the report ("After the review fixes of 2026-11-09"): 18.3 [18.3 - 18.4] items a second at a load of 4.5 to 6.4 (the gap between two items 54.6 ms at the median) and 18.3 [18.2 - 18.3] right after the build at a load of 23 to 34, the bound "reaches 80%" held in 6 of 6; the fairness is as before (sixteen jobs 72.2 items a second in all, last over first 1.01; one big job beside fifteen small ones 72.3 and 1.05; a lone job at the highest rate 74.5).
- **D2127** (2026-11-09, G2049, B1 to B3) **The claims about the view are what the tests hold.** ADR-0083 "Enforced by" names `trigger_plan.rs` (the plan shape in three states of the statistics, the time bound, the oracle of the previous definition) and says it is run in a pull request only when a file of its target changes (`tests/AGENTS.md`). "The only plan this form has" is gone from the migration comment, D2117 and the ADR: a `LATERAL ... LIMIT 1` gives the planner a probe for each trigger and no join to rescan, but PostgreSQL does not take a partial unique index as proof of uniqueness, so a per-trigger sequential scan with a limit is not ruled out by the form; the assertions of `trigger_plan.rs` are the protection (and fail on that plan too). The measurements ("2.85 s at 10,000", "2.6 microseconds a trigger") are out of the migration comment (they go stale) and in ADR-0083 and the report. The ids in the migration and the test are D2117 (they cited D2116). The migration comment says the one behaviour that differs: with the singleton row missing (unreachable: primary key on `only_row`, ENABLE ALWAYS triggers that refuse DELETE and TRUNCATE) the old `CROSS JOIN` listed nothing and the scalar subquery lists MISSING, REVIVED and RESPEC and never PAUSED.
- **D2128** (2026-11-09, G2049, B4) **The oracle covers seventeen classes, and a mutant of each part of the CASE is killed.** `trigger_plan.rs` seeds 1,700 cron triggers in 17 classes of 100 and 50 triggers that are not cron: the eleven of D2117 and, new, an active schedule with only the nominal time missing, with only the actual time missing, with five actions left and neither time, a planned schedule with no action left, a schedule whose head is revision 2 equal to the trigger over a revision 1 that is not (the lookup reads the head), the same with the head not equal over a revision 1 that is, and the event triggers (the `t.kind = 'cron'` filter). Expected: MISSING 100, REVIVED 200, RESPEC 500, PAUSED 100 (LEGACY only), NO_NEXT_TIME 400, so 1,300 listed (1,200 once handed over). *Hand mutants, each killed and restored:* the `OR` of the two times of NO_NEXT_TIME made `AND`; the filter `t.kind = 'cron'` removed; the revision read by number (`r.revision = 1`) and not by head; (from D2117) the expression compared at `{cron,1}`, the state of the pause inverted.
- **D2129** (2026-11-09, G2048, C1, C2) **The reconciliation's pace is stated per size, and the ceilings of the other passes are called ceilings.** "30 to 77 writes a second" next to "21 to 27 minutes" did not add up: 30 a second is the 10,000-trigger scenario under the defaults, and at that pace 100,000 triggers would take 55 minutes; only the 100,000-trigger scenarios (62 and 77 a second) give 27 and 21 minutes. Stated per size in `trigger_schedules.rs`, ADR-0083, `schedules.md` and D2118: 100,000 triggers 62 a second under the defaults (27 minutes) and 77 with the per-tenant cap at 100 (21); 10,000 triggers 30 and 69 (6 and 2). The difference under the defaults is read from the report's counts of rounds (at 10,000, 7 or 8 rounds of the 60 s found nothing to write and were each followed by the poll of 5 s, which a backlog of 100,000 hardly meets), not tested alone. The pass that expires `CANCEL_SOURCE` entries pauses "at a hundred a second" the same way (the pause is added to the time of the round): `run-reset.md` (two places), `resets.rs` and `run_resets` now say a ceiling and that its pace was not measured. The stale line of the `DRIFT` statement ("joined to its schedule, not looked up") is corrected.
- **D2130** (2026-11-09, review D2) **The test of the window of `Keeper::leave` waits, with the limit of its neighbours.** `Keeper::leave` gives the shards back and then, in a second statement, withdraws the announcement; the test read the members the instant the shards read zero and failed once as the runtime role. It waits with `eventually` (`tests/support/mod.rs`: polls every 50 ms and fails after 5 s with "timed out waiting for: and leaves the replicas that count for the shard shares"), the same limit as the five other waits of that test; the window itself is milliseconds. The migration `20270114000000_trigger_handover` is edited in place: the refs that contain it are listed in the report of this task (`origin/feature/018-business-ids` only: no tag, no `develop`, no `main`), and any database that applied the old text must be recreated (sqlx validates the checksum).
- **D2131** (2026-11-09, T018-075) **The compose file passes the four switches of the feature from the environment, each `false` unless the environment says `true`, and the quickstart says which lines of `infra/docker/.env` turn them on.** `SAGAT_SCHEDULES_ENABLED`, `SAGAT_BATCHES_ENABLED` and `SAGAT_RESET_CANCEL_SOURCE_ENABLED` go to the control plane and to the engine (they must agree, ADR-0081), `SAGAT_RESET_REAPPLY_ENABLED` to the control plane only (the engine has no such setting, D1990); the other settings of the passes (`SAGAT_SCHEDULES_BATCH`, `_POLL_MS` and the rest) stay at the binaries' defaults and are not in the compose file. **No default of a binary changed** and the demo showed the effect (`the optional passes of this replica: ["schedules", "batches", "resets"]`). The lines were **not** added to `infra/docker/.env.example`: the session that did T018-075 was refused every read of a file named `.env*`, so it could neither read nor edit it without guessing its content; the four lines are in section 1 of the quickstart and the maintainer adds them (commented, off by default) when committing (G2061).
- **D2132** (2026-11-09, T018-075) **`sagat batch cancel <ID> --reason <TEXT> [--key K]` sends the JSON body the contract asks for.** `cancelBatchOperation` requires `{"reason": 8 to 2,000 characters}`; the CLI sent a POST with no body and no content type and the platform answered `400 content type must be application/json`, so no job was ever cancelled by it (the unit test of the command asserted the request line and the key and never the body, which is why it passed). Found by the demo (step 6.5), seen failing first (`variant BatchCommand::Cancel has no field named reason`, the body and the content type asserted in `a_job_is_read_listed_its_items_shown_and_cancelled`), fixed in `apps/cli/src/{args,batch}.rs`; `--reason` is required (usage error, exit 2), the length is the platform's to judge (`422` in words); the key of a cancel now depends on the reason. The usage line of `docs/reference/cli.md` was edited by hand to the same words and `cargo test -p sagat-tests --test contract_developer_experience --locked` passes (8 tests, among them `the_command_reference_is_what_the_tool_prints`, without the bless switch).
- **D2133** (2026-11-09, T018-075) **The description of a schedule lists only the runs that are still open.** `openRuns` was the schedule's own `tracked_runs`, which the pass forgets at its next step, so `sagat schedule get` said `1 open run(s)`, then `2`, for a schedule whose runs had all finished, and went on saying it for as long as the schedule was paused (a paused schedule takes no step). The firing already treats an ended run as closed (`open_run_ids`); `ScheduleService::describe` now reads the status of the tracked runs (`ScheduleStore::run_statuses`, one query, none when nothing is tracked) and returns the open ones; the stored tracking is unchanged and the pass still forgets the rest as before. Seen failing first (`firing::a_description_lists_only_the_runs_that_are_still_open_though_the_pass_has_not_forgotten_the_ended_one`: `left: [run] right: []`), green after, `rest::` and `service::` 95 passed; rerun on the stack: the database still tracked the ended run and the description said `0 open run(s)`. `ScheduleDescription.record.tracked_runs` is therefore the open ones in a description and the tracked ones everywhere else (said in the field's comment).
- **D2134** (2026-11-09, T018-075) **`quickstart.md` is rewritten to what the stack did.** Every command is the one that ran (the CLI's real flags: `schedule get <id>`, `--from/--to`, `--start-delay` in seconds, `batch --selector`, `batch items`, `batch cancel --reason`; the business-id and Signal-with-Start demos use `cart`, which stays open, because `hello-world` finishes in 0.3 s and the conflict is then a race), every "Expected" is what it showed (the run resource carries `startedBy` only, G2057; the nominal and actual times are on the action; `ALREADY_ACTED` is counted in the backfill; the note is on the schedule and not in the audit; the page says that the dev worker stops with the engine), the failure table says for each row what was shown and names the tests, and the four rows a stack cannot show are marked as not demonstrated with the reason. The runbooks were corrected where the demo showed a sentence false: `batch-operations.md` (`UPDATE` is accepted, `REPIN` refused), `run-reset.md` (`CANCEL_SOURCE` is built), and where a pointer was missing (`schedules.md`: the compose lines, the handover on the dev stack, the note not being audited, `openRuns`; `business-ids.md`: `--start-delay` is in seconds).
- **D2135** (2026-11-09, T018-075) **`verification.md` is the record of T018-075 and uses four statuses, strictly.** VERIFIED (the evidence proves the whole sentence), PARTLY (a part, with what is missing), BLOCKED (a feature that is not approved or merged), NOT DONE; a test that proves part of a criterion makes it PARTLY. Result: of the 17 criteria 6 VERIFIED and 11 PARTLY, FR-018-046 and FR-018-049 PARTLY, NFR-018-005 VERIFIED as measured. Evidence that is not the author's (the task notes, CI run `42649041664` on `345dd70e`) is named as such. The "Final gates" table is **blank on purpose**: the gates are the maintainer's (rule 17). `tasks.md` is not ticked: T018-075 has a dated note.
- **D2136** (2026-11-09, T018-075) **The run resource not carrying the schedule's provenance is recorded as a gap and the demo step as FAILED, not fixed here.** FR-018-016 says runs "expose schedule id, nominal time, kind and revision"; `runs` has the columns (migration `20261230000200`, written at the action, immutable) and no response reads them. Exposing them is an additive field of `Run` in `openapi.yaml` (and the journal contract names `schedule {scheduleId, nominalTime, kind, revision}` on the run, `contracts/journal-and-events.md`), a change of the reference blessed from it, of the Studio's generated types and of what six SDKs parse, which the maintainer reruns as a set; the page says where to read the times today (G2057).

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
| G1993 | The request of `CANCEL_SOURCE` takes its locks in an order that differs from the table of ADR-0050 decision 7 (D2001, ADR-0081 decision 5), and no slot-order barrier harness runs over it: T018-012, which builds that harness for the admission paths, stays open. What is tested instead: the crash matrix of the request (19 writes), the race of two requests for the last place, two equal requests at the row of the source, and a requester revoked while the run is admitted. | The harness is a task of its own (T018-012) and runs over every path of the drafts; one path does not justify building it. | Open (T018-012) |
| G1992 | Three things about the tests of `CANCEL_SOURCE` are weaker than the clause. (1) *The point becomes ineligible after the request* cannot be produced by a run the platform makes: the prefix is immutable, and the clause "compensated, skipped, resolved or reconciled later" cannot be reached by a journal the reducer accepts (G1979 (2)). The re-check is proved by entries written by hand with a wrong digest and a sequence that is not a point (they fail `BOUNDARY_NO_LONGER_ELIGIBLE`), and the change of the journal that a real run does produce, a step that completes after the point, is proved end to end (`REEXECUTION_NOT_ACKNOWLEDGED`, D1999). (2) *Closed in part 2026-11-06 (D2009):* that an engine **without** the flag makes nothing is now a test of a pure function (`optional_passes` names the reset pass only for the flag, and the settings of the pass alone do not turn it on), and the engine starts exactly the passes it names; what stays unproved is the observation itself (nothing happens), which a test can only wait for, and the positive wiring is tested with a real engine. (3) The shared scenario in the six SDKs proves the client surface and, with a real engine, that the run is made; the cases of the quota, of the setting off and of a stuck source are not in it (a scenario cannot change the server between steps). | (1) needs a feature that appends compensation to the journal of a point already passed; (2) is the nature of the observation; (3) the scenario file is one server. | Open |
| G1991 | An entry that waits cannot be withdrawn, and does not expire (D2000). Everything that follows from that: (1) a source that never ends (a run in manual review nobody looks at) holds one of the namespace's places (16 by default) for as long as it lives; (2) the person who asked has no call that gives the place up, and **an administrator has none either**; (3) **revoking the requester does not free the place**: the engine fails the entry (`AUTHORITY_REVOKED`) only when the source has ended, so the one lever on an entry that waits is to end the source (retry, skip or resolve the step), or to remove the member and wait for it to end; (4) the limit can be raised, but only up to the hard ceiling of 64 (`quota_ceilings`), so a namespace in which sources are stuck can be filled for good; (5) the customer's run was cancelled by the request and stays cancelled whatever becomes of the entry. **Amended 2026-11-09 (OD-2026-11-08-1, D2059 to D2068): the expiry is decided and built, so (1) and (4) no longer hold for good: a place is held for the window at most (24 hours by default, 1 hour to 7 days), after which the entry ends `FAILED` `SOURCE_DID_NOT_END` and the place is free; (3) now has a second lever, waiting out the window. (2) and (5) stand: there is still no call to withdraw an entry, and the request's cancellation is never taken back.** | A call that withdraws an entry is a product decision, and the owner delegated only the expiry (the withdrawal is G2018). The runbook (section 4.2) says what the levers are. | Expiry built, review pending; withdrawal open (owner) |
| G1990 | A completion that fails for a reason that is not an outcome (storage, a wiring that is missing, an error the mapping does not know, a conflict nobody foresaw) leaves the entry pending and is tried again, with a warning in the log naming the tenant and the reset, for as long as it fails. *Head of the line (review A, D2006):* the scan is oldest first, so such an entry would be first in every round and starve the younger ones; three things bound it: at most `SAGAT_RESET_CANCEL_SOURCE_PER_TENANT` entries (10) of any one tenant are taken per round, the pass leaves an entry that failed out of the scan until its time (2 s, doubling to 5 minutes, at most 1,024 remembered), and only the two known conflicts are taken for races. What stays: the memory is the process's, so a restart tries every entry once; the entry has no durable attempt counter and is never given up (`FAILED` with which reason is a decision, and the batch items have a bounded retry; the schedule backfill step has the same gap, G1952); a tenant with more than `per_tenant` failing entries still occupies its whole share every round, which cannot take the share of another. | An attempt counter and a `next_attempt_at` are a column of the register and a decision about when an entry is given up; the engine's log, `workflow_resets_pending` and `scheduler_lag_seconds{pass="resets"}` show it. | Open |
| G1989 | The setting is read by each process (ADR-0081). **With it on in the control plane and off in every engine the request still cancels the customer's run and answers `202`, and nothing replaces the run until an engine's flag is turned on**: nothing is lost (the entry waits in the register, counted against the limit), `scheduler_lag_seconds{pass="resets"}` grows once the sources have ended, and `SagatSchedulerLate` fires, but the cancellation cannot be taken back in the meantime. The converse (on in an engine and off in the control plane) makes the entries that exist and refuses new requests. The rollout order (engine first, control plane first to turn off) is in ADR-0081 and in the runbooks of schedules, batch operations and resets (section 4.2); `no_shipped_manifest_turns_the_reset_switch_on` keeps the repository's own manifests off. | The instance-capability registry of ADR-0050 decision 9 would refuse the mismatch and is not built (T018-080; the same as G1965 and G1988). | Open |
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
| G1996 | A row of `serving_instances` was owned by knowing its random `instance_id`, which the runtime role can read: another process holding the runtime role could refresh, delete or rewrite it. | *Closed 2026-11-08 (D2024):* a row is touched only for the token that registered it, and the runtime role cannot read the hash. What a holder of the runtime role can still do is G2000. | Closed |
| G1997 | The capability was the binary's, not the setting's: an engine with `SAGAT_SCHEDULES_ENABLED` unset listed `schedules-v1` and ran no schedule pass, so the handover (T018-061) would have stopped the legacy cron pass in an engine that fires no schedule. | *Closed 2026-11-08 (D2023):* an engine lists `schedules-v1` only when its configuration runs the schedule pass; an activation refuses a live engine without it, naming its id. | Closed |
| G1998 | The engine's worker gRPC API could have kept answering while its passes were stopped by a requirement it lacks. | *Closed 2026-11-08 (D2021, D2031):* the admission layer of the gRPC server answers every new call `UNAVAILABLE` with `x-sagat-error: BINARY_CAPABILITY_MISSING` before it authenticates while the gate is closed, and the producer of a task stream checks the gate before every claim, so a stream open when the gate closes leases nothing more (it ends with that answer at its next claim, within its window of 5 s); a worker goes to another replica. A frame already queued is delivered. | Closed |
| G1999 | The registry does not carry `SAGAT_BATCHES_ENABLED`, `SAGAT_RESET_REAPPLY_ENABLED` and `SAGAT_RESET_CANCEL_SOURCE_ENABLED`: no capability is defined for them. G1965, G1988 and G1989 are therefore unchanged: with the flag on in the control plane and off in every engine the registry does not refuse. | Moving them is a change of its own (a capability value, a migration that widens the CHECKs, the engines listing it, the activation commands) that supersedes ADR-0081, which says so in its amendment of 2026-11-08. | Open |
| G2000 | What a holder of the runtime role can still do to the registry, which no function of the database can close because the role legitimately calls them: (1) register up to 1,000 rows of its own, so that every new process finds the table full (it serves, uncounted: D2025) and an activation's verification is incomplete; (2) hold the registration lock open (the epoch `FOR SHARE` and the public advisory lock that counts the table, in a transaction it does not finish), so that new processes time out and stay refused until it lets go; (3) fail a victim's heartbeat by holding its row lock, which closes the victim's gate after 30 s (D2026); (4) at the cap, restarts evict nothing while the rows are live. It cannot record a requirement, and it cannot refresh, delete or rewrite another's row. | The registry protects a fleet from mistakes, not from a hostile holder of the credentials of the processes it protects; that is the credentials' boundary (rotate them, find the sessions, runbook section 6). | Accepted |
| G2001 | Feature 029's request path reads the epoch row `FOR SHARE` inside tenant transactions, but `sagat_app` has `SELECT` only on `platform_requirement_epoch` and a share lock needs `UPDATE`; a share lock held for a whole tenant transaction would also make a queued activation's `FOR UPDATE` wait, and that wait stalls the registration of every new process. | Not built for 029 (not approved). It needs a definer function the runtime role can execute that *reads* the epoch (and does not hold it), and its own proof that the wait of an activation cannot starve new registrations. A dated note is in 029's `plan.md` and `data-model.md`. | Open (029) |
| G2002 | What the activation command of a feature (`control-plane schedule-trigger-handover`, T018-061) must also do: (1) take `--expect-live N` and refuse when fewer than N live instances of the required kinds exist (an activation with none passes vacuously); (2) map `permission denied for function` to "run with the owner role"; (3) say in `cli.rs` which commands need the owner role (the usage text now names the activation commands of the instance registry). | Not built here: T018-061 owns the command. | Open (T018-061) |
| G2003 | A precondition for activating 029's `access-narrowing-v1` or 031's `payload-encoding-v1`: the whole fleet runs builds with the fail rule of D2026 (a heartbeat that fails for longer than the liveness window for a reason other than a lost connection closes the gate), with the token of D2024, and an engine's capabilities follow its configuration (D2023). A build without them would keep serving invisibly after a lock timeout on its own row. | Stated in the runbook (section 3) and ADR-0082; checked by nothing but the runbook, as G1607. | Open (029, 031) |
| G2004 | The Studio could not show the pause, resume and run-now buttons of a trigger-owned schedule as disabled before the handover. | *Closed 2026-11-08 (D2053):* the schedule resource carries `controllable` for a `TRIGGER` schedule; the Studio disables the buttons with the reason. | Closed |
| G2005 | An engine binary older than the registry release neither registers nor reads the handover: after it, it keeps its legacy cron pass, which starts no tick twice (shared key) but **ignores a pause** of a trigger-owned schedule (G1011, said in the runbook, section 6.4; test `upgrade::an_old_engine_binary_keeps_firing_after_the_handover…`). A registry-aware engine without `schedules-v1` refuses to run instead. | Nothing in the database can stop a binary that never looks. | Accepted |
| G2006 | For the seconds between a publication and the reconciliation's next round a new trigger-owned schedule has no next time (publication has no calendar), and while the state is `LEGACY` the legacy pass plans the trigger's first time by its own clock: the first tick of a new trigger is fired by whichever pass planned it, and the schedule's next time is the trigger's own when it has one. | A tick that is fired by one pass only is still fired once. | Accepted |
| G2007 | A trigger whose four possible schedule names a tenant took in advance has no schedule (`MISSING` every round, logged with tenant and trigger, backing off to five minutes); the legacy pass still fires it. | Only the tenant that made the names suffers it. | Accepted |
| G2008 | The handover writes no tenant `audit_records` row, and a `schedules-v1` requirement recorded by hand while the state is `LEGACY` was refused with no repair. | *Closed 2026-11-08 (D2051):* the command completes it after its checks, and the unused audit action was removed; there is still no tenant audit record (D2042). | Closed |
| G2009 | The six SDKs keep `controllable` reachable in the raw resource and have no typed field for it. | `sdks/` was not touched in this change; the typed field belongs to the next SDK pass. | Open (SDKs) |
| G2010 | The handover takes a table lock on `workflow_triggers` for its transaction (at most the operator's 3 s): a publication that is in flight makes the activation fail ("run it again"), and one that arrives waits. | The lock is what makes the convergence count atomic with the state change (D2046). | Accepted |
| G2011 | `trigger_schedule_unconverged` is measured every sixth round of each replica (about every 30 s) and is the same number from every replica: read it with `max`. | One bounded query for the whole fleet, not one per round. | Accepted |
| G2012 | The three alerts of D2056 and their promtool test are written and pass `promtool check rules` and `test rules` locally, but the CI step of `.github/workflows/ci.yml` lists only `batch_stalled_test.yaml` (and the registry's file is not listed either). | Touching the workflow runs the whole stack on a pull request (Constitution 4.3.0): the line waits for the pull request that has to touch it anyway. | Open |
| G2013 | A tick fired by the legacy pass **at the same instant** as the commit of the handover may start a run for a schedule that was just paused if the pause is accepted inside the window: closed by the control window of D2053 (a pause is refused for 5 s); an old engine binary that does not read the state is G2005. | | Closed (D2053) |
| G2014 | *(Raised 2026-11-09 and withdrawn the same day: it said that the control plane writes `run_resets.created_at` with its own process clock, so a skewed control plane would move every expiry. That was wrong.)* `created_at` is `tx.now` in `register_pending`, and `register_pending` takes only the transaction of the **source** run (it refuses `registration.source != tx.info.run_id`), whose `PgRunTx.now` is `SELECT clock_timestamp()` sampled after the row lock is held (`execution.rs`, `begin`); the control plane's `SystemClock` (`self.clock.now()` in `pending_reset.rs`) stamps the audit records of the request only. A control plane whose clock is off by any amount neither cuts the window short nor stretches it. | Nothing to fix, and no column or migration slot is needed. Pinned by `a_control_plane_whose_clock_is_three_hours_off_neither_cuts_the_window_short_nor_stretches_it` and `an_engine_whose_clock_is_three_hours_off_reads_the_window_on_the_database_clock` (D2079). | **Closed (verified not a risk)** |
| G2015 | The plan of the expiry scan is shown to be servable by a partial index of the entries that wait (`the_scan_for_expired_entries_can_be_served_by_the_index_of_the_entries_that_wait`, with sequential scans disabled), **not measured at the size of a deployment**. On a handful of rows the planner reads `run_resets_pending_namespace_idx` and filters the shard and the time; with many namespaces it may prefer `run_resets_pending_idx`. Either way what is read is the entries that wait (at most 64 a namespace), but the choice and the cost with, say, 100,000 waiting entries were not run. | A load test of the pass with a register of that size, as ADR-0083 did for the reconciliation. | Open |
| G2016 | **No alert and no signal before an entry expires.** `promtool` is not installed on the machine this slice was built on, so a rule for `increase(workflow_reset_completions_total{outcome="expired"}[1h]) > 0` could not have its test and was not written. And nothing tells an operator that an entry is about to expire (no gauge of the age of the oldest entry that waits for an open source): they learn it from the entry that ended or from the counter. | A rule and its `promtool` test (and the line of CI that runs it: G2022), and a gauge of the age of the oldest waiting entry whose source is open, next to `workflow_resets_pending`. The rule and its test are a **precondition for lifting ADR-0081 decision 4** (G2026). | Open |
| G2017 | The expiry leaves an entry whose source row another transaction holds for the next round and never waits for it (`SourceBusy`, D2060). **Amended 2026-11-09 (D2081):** the entry is now left out of the scan for a second, counted in the round (`busy`) and logged at debug with its ids, so it no longer keeps its slot in the share of its tenant and hides the newer entries. What stays: a source that is **never** free (a transaction that never ends on that run) would never let its entry expire, and no metric counts the rounds an entry was left for it; a run is locked for milliseconds, so it is a theoretical case. As for the completion (G1990) the entry has no durable attempt counter: the backoff of an entry that errors is the process's memory, and a restart tries every entry once. | A counter of `SourceBusy` and a durable attempt count, if a deployment ever shows it. | Open (narrowed) |
| G2018 | **No withdrawal.** The owner delegated the expiry only (OD-2026-11-08-1): there is still no call by which the person who asked, or an administrator, gives up an entry before the window is over, and the request's cancellation of the run is never taken back. An entry can hold its place for up to the window (7 days at most). The SDK scenario `reset` does not exercise the expiry either (it cannot wait a day and the window cannot be set below an hour). | A withdrawal is a later decision for the owner (G1991); the expiry is proved by the integration and fault suites with an injected clock. | Open (owner) |
| G2019 | The tripwire for an old engine binary (`cron_legacy_fires_after_handover_total`) reads the nominal time of the tick (`last_fire_at`), the only time the table keeps: an old binary that fires a late or catch-up tick whose nominal time is **before** the handover plus ten seconds is not seen; it assumes a legacy round ends within ten seconds of the commit; and it works only while one engine of this release runs the schedule pass (D2074). | Another column would need a migration of a hot table; the tripwire is a signal, the census and the attestation are the guard. | Accepted |
| G2020 | The census covers execution engines only. A control plane older than this feature can publish a cron trigger after the handover and its first ticks are lost until the reconciliation plans the schedule (a sweep, seconds, and the five-second poll; longer under a backlog; `SagatTriggersUnconvergedAfterHandover` fires after five minutes). | A control plane has no `schedules-v1` to list and the registry does not count it; the operator step "attest that every control plane runs this build" is explicit (6.2 step 1). | Open (a control-plane census) |
| G2021 | A backfill of a trigger-owned schedule over ticks the legacy pass started more than **seven days** ago (`idempotency_retention`, 7 days, purges `command_dedupe`) starts a second run for them: the legacy pass leaves no action row, only the key. | Looking the tick up in `runs` by trigger and nominal time needs an index of the runs by the document of the tick; the window is documented in the runbook (6.4). | Accepted, documented |
| G2022 | The CI step `promtool test rules` (`.github/workflows/ci.yml`, the step after `promtool check rules`) runs only `batch_stalled_test.yaml`: `trigger_schedules_test.yaml` (this feature) and `instance_registry_test.yaml` (ADR-0082) exist and pass locally and are not run by CI. The line to add to that step is `/tests/instance_registry_test.yaml /tests/trigger_schedules_test.yaml` after `/tests/batch_stalled_test.yaml`. | A change of the workflow reruns the whole stack; it is made with the next change of the workflow. | Open (workflow) |
| G2023 | The windowed reconciliation was measured with 20,000 triggers of 200 tenants (ADR-0083); **100,000 were measured only for the one-statement query of round 2 (drift 122 ms, count 34 ms) and the time a backlog of that size takes to drain at 100 writes a second is an estimate** (about 17 minutes for one tenant at 20 entries a round, and fewer across tenants). | A hundred thousand triggers in a disposable database cost minutes of seeding; the window cost does not depend on the number. | Estimated; **measured 2026-11-09 (G2048): 21 to 27 minutes, not 17** |
| G2024 | **The flag off in every engine pins the places** (D2080). The expiry is a step of the reset pass, which an engine with `SAGAT_RESET_CANCEL_SOURCE_ENABLED` off does not run: an entry accepted while the flag was on holds its place in the namespace until an engine with the flag on runs, and then every entry older than the window expires, a few at a time (D2081). The runbook says it (sections 4.2 and 6), and so does the log line of an engine with the flag off. The alert on the lag after a source has ended (`SagatSchedulerLate`) does not fire for a source that is still open, so nothing tells the operator that the places are pinned except `workflow_resets_pending` staying high. | The registry that refuses a fleet that does not agree (ADR-0081 decision 4, T018-080 for this capability), or a gauge of the age of the oldest waiting entry (G2016). | Open |
| G2025 | **Engines with different windows.** An engine applies its own `SAGAT_RESET_CANCEL_SOURCE_EXPIRY_SECS` to the entries of the shards it holds, and shards move, so the window of an entry is the one of the engine that holds its lease when a round looks (D2082). The runbook says to set one value on every engine; nothing refuses a fleet with two, and the line each engine logs at the start of the pass (`expiry_secs`) is not tested. | The capability registry carrying the value, or a column of the entry that holds the window it was accepted under (a migration: the owner's). | Open |
| G2026 | **An alert and its `promtool` test are a precondition for lifting ADR-0081 decision 4** (the flag on by default): an alert on `increase(workflow_reset_completions_total{outcome="expired"}[1h]) > 0`, with its rule test, and the line of CI that runs it (G2022). No untested alert was written (`promtool` is not on the machine this was built on). | Write the rule and its test where `promtool` exists, and add it to CI. | Open (precondition) |
| G2027 | **A batch selector by `businessIdPrefix` is still refused** (`BATCH_SELECTOR_UNSUPPORTED`, `application/src/batch.rs`, "the business-id binding arrives with its own story") though the bindings exist and `GET /runs` filters by the prefix; the remediation the wire gives says "Business ids are not bound to runs yet" and the row of `contracts/errors.md` says it too. FR-018-022 and the selector of a batch job (`research.md` section 3) name it. Found by the export canary, which therefore searched the refusal of the selector and not a job that used it. | Select by `runs.business_id LIKE prefix%` (escaped, an index on it) in the selection round and the preview, lift the refusal, correct the remediation and the contract row; the prefix is customer text: it stays out of the job's log lines and spans, which the canary already checks for the refusal. | Open |
| G2028 | The OTLP canary searched the bodies of the traces the exporter sent (the live spans of `sagat_*` targets that have a trace, the spans the projector tells, and the `http.request` spans of the REST calls of the second test), the Prometheus page and every log line at all levels; **not** the OTLP logs and gauge endpoints (`/v1/logs`, `/v1/metrics`), which the collector double does not receive. A string that went through JSON is searched as JSON writes it: a canary is `[a-z0-9-]`, so that is the text itself; a text with a quote, a backslash or a character outside ASCII would need its escaped form searched too. | A collector double that keeps the three endpoints. | Open |
| G2029 | `T018-012` has slot cases for the schedule pass (action, buffered release, backfill step), trigger-now, `CANCEL_OTHER` payment, the release of a delayed start, the business-id claim and the request and the completion of a pending reset, each run for every start of its reference (D2089). **Not built:** the cases of feature 019 (not approved) and of 022 (its slot, the semaphores, does not exist), and of the batch item (row 15; the monitor of `batch_locks.rs` watches it under load), the reset `REQUIRE_TERMINAL`, with-start and update-with-start; and nothing is proved about a lock a reference does not name or an interleaving of more than two transactions. The cases are a **release gate**: a pull request runs `fault_schedules_bulk_operations` only when its files change. | Add the cases as the paths get their slot; `against_at` takes any path. | Open |
| G2030 | The slot cases assert the `40P01` outcome of the reference and of the path (both are read), not `pg_stat_database.deadlocks`, which PostgreSQL reports a moment late and per backend: a negative reading of it is not proof. The counter is read only by the self-test that makes a deadlock on purpose. | None needed; the explicit outcomes are the proof. | Accepted |
| G2031 | The audit-rollback matrix (`audit_rollback.rs`) covers the schedule changes (create, update, pause, resume, delete) and the creation and cancellation of a batch job. **Not covered:** the firing of a schedule (the crash matrix of `firing.rs` compares the digests, but a decision with no run, such as a skip, is not checked for its record), the creation and the cancellation of a backfill, trigger-now, `revoke_authority` (the audit record and the pause in one transaction), the item audit of the batch executor, and the handover command (outside, D2013). | The same driver for each, as their crash matrices are written. | Open |
| G2032 | The operations the engine makes on a run that already has a trace carry no trace in their context (the cancellation a `CANCEL_OTHER` schedule pays, the item actions that are not a rerun, the cancellation of the source of a reset): their audit records have `trace_id` null. The run they act on has its own trace, and the OTel evidence of the operation is that run's; the runs the engine **creates** (a schedule's, a pending reset's, a batch rerun's) now begin one (D2084, D2087). | Give each its own span under the run's trace, if an operator needs the operation and not only the run. | Open |
| G2033 | **The list of resets made from a run is read again only when the window is focused** (`ResetsMade`, by design: "not read on a timer"). When the source of a pending reset has ended the header of the run says `Cancelled` and the entry still says `waiting for this run to end` until the person comes back to the tab or reloads, which the engine makes the new run within a moment of. The journeys send the event the browser sends on a return to the tab (`visibilitychange`) to see the entry complete. | Read the list while an entry waits and the live status of the run is over (a bounded poll), or when the status turns terminal. | Open |
| G2034 | **Two failure states of the task text are not reachable from the Studio against the real platform.** A batch job is made from the filters of the run list (status, workflow, origin), never from ticked runs, so the platform's `BATCH_SELECTOR_INVALID` ("selector too wide") for a preview cannot be provoked (the journeys cover the preview that matches nothing, the one capped by `Most runs`, and the quota refusal of the job); and `RESET_POINT_NOT_ELIGIBLE` and `RESET_REEXECUTION_NOT_ACKNOWLEDGED` from the platform cannot be, because the dialog asks for the acknowledgement before it sends and the journal of an ended run does not change (the journeys cover the acknowledgement missing in the dialog, the identifier held by another open run, and the limit of waiting resets). | Their words are in `ResetRun.test.tsx` and `BatchFromRuns.test.tsx`, and the platform's answers in the REST suites. Whether that is enough for T018-074 is a question for the owner: **O1952**. | Open (O1952) |
| G2035 | **`SOURCE_DID_NOT_END` and `AUTHORITY_REVOKED` are not waited for or made in the browser.** The first needs a wait of an hour at the least (the engine's setting is 3,600 to 604,800 seconds); the second needs the member who asked to lose a role, and the members of the e2e platform (`olivia`, `dev`, `vera`) are shared by the other journeys. Their wording is tested in `ResetRun.test.tsx` (`FAILURES`); `ADMISSION_REFUSED` is the failed entry the journey makes. | A later journey with a member of its own, if the harness mints one; or the owner's answer to **O1952**. | Open (O1952) |
| G2036 | **Identifiers called by their first eight characters remain only where no screen shows them as a name**: nowhere in the Studio now (D2100). The command line still does (G2039). | | Closed for the Studio |
| G2037 | **The journeys ran on one machine (macOS, headless Chromium 153).** The selects are operated by the type-ahead of the browser (an arrow key opens the popup on macOS, so none is used), after a Tab and a Shift+Tab that start the search again; Linux, which CI runs, shares the code of the browser and not of the platform's popup, and the runs in a row (the note of T018-074) were not made there. | The result of the `studio` job of the pull request. | Closed 2026-11-09: the `studio` job of pull request #98 on `345dd70e` (run 42649041664) ran the whole Playwright suite on Linux and passed; one run only (a flake would show as a failed job on the next ones). |
| G2038 | **Two flows of FR-018-047 have no browser journey.** The cancel of a backfill (`Backfills.tsx`) and the preview of a backfill (G1966: the platform has no route that counts the times of a range, so the Studio has none to show). The cancel is tested in `SchedulePage.test.tsx` only. | A journey that holds a backfill open (a range the schedule's overlap rule makes wait) and cancels it; the preview waits for its route. | Open |
| G2039 | **`sagat worker` prints `run <first eight characters of the run id>`** (`apps/cli/src/worker.rs:229`): the identifiers are time-ordered, so the lines of the runs made in one minute begin the same. | `shortId`'s rule in the command line (and its tests): the last eight. | Open |
| G2040 | **Runs in review stay in the shared namespace after a journey**: the two runs of `batch-unknown` (a run in review refuses a cancellation by design, T037). They hold no task, and no journey that follows serves their activity, but they stay in the list of runs of the namespace. | A namespace of its own for that journey, as the others with limits have. | Open |
| G2041 | **The journey of the start delay takes a little over a minute** (one minute is the least a start can be delayed, and nothing releases a start sooner; the other thirteen take 1 to 9 seconds) and its assertions about the held run have the one minute: a machine slow enough to take a minute between the start and the first assertion would see the run released. | None that the Studio can give; the margin is the machine's. | Accepted |
| G2042 | **`smoke.spec.ts` and `database-connections.spec.ts` call axe themselves**, with their own lists of tags (the second leaves out `wcag21a` and `wcag22aa`); only `a11y.spec.ts` and the journeys of 018 share `support/accessibility.ts`. | Move the two onto the shared call, which may make the second find what its narrower list does not. | Open |
| G2043 | **A lone batch job applies about one item a second, not the 20 it asks for by default** (and 75 a second, not 200, at the highest rate it may ask for). The executor never catches up in a burst, by design (`next_slot = max(slot, now) + interval`, "a job that ran late never catches up"), one step applies at most one item of a job, and the engine's loop rests `SAGAT_BATCHES_POLL_MS` (1,000 ms) whenever a round found no job due: after an item the job's next slot is 50 ms away, the next round finds nothing due, and the loop sleeps. Measured (3 of 3 repetitions, 120 runs, the release build): 0.97 to 0.98 items a second, the gap between two items 1,036 ms at the median; with a poll of 50 ms 13.3 to 13.6 items a second (on a machine at load 15 to 24); sixteen jobs at once get 4.5 items a second each, because the other jobs keep the loop busy, so **the rate of a job depends on how many other jobs there are**. The runbook calls the poll the "pause when a round found nothing" and the rate "one item per slot" and does not say the two together bound a lone job at one item per poll. | The loop should wait until the earliest `next_slot_at` of a due job, bounded by the poll, instead of the whole poll: a small change of `run_batches` / `BatchPass::round` that needs its own decision and test (both roles). Or the runbook says it. Not changed here (D2112). The bounds "reaches 80% of the rate it asked for" are broken in 3 of 3 repetitions at both rates. | **Closed 2026-11-09 for the lone job (D2116):** the round says when the soonest slot of a job comes and the loop wakes then; the same scenario now applies 18.4 items a second (the bound held in 6 of 6 repetitions; addendum of the report). The two bounds that stay broken are the replica's capacity, not the loop (G2044, D2121). |
| G2044 | **One engine replica starts about 60 runs a second, and a burst is as late as its size over that.** 10,000 schedules due at the same instant: the lag at p50 82 s, p95 158 s, max 166 s (60 actions a second; 3 of 3 repetitions broke the bound of 60 s at the p95); with two replicas on the same database 131 to 135 actions a second and p95 70 s, max 74 s; spread evenly over a minute (167 a second) p95 100 s with one replica; at 1,000 schedules p95 16 s at the same instant and 1.0 s spread over a minute. The round ceilings do not change it: `SAGAT_SCHEDULES_ROUND_ACTIONS` 1,000 with `SAGAT_SCHEDULES_BATCH` 500 gives p50 82 s, p95 157 s, max 165 s and 60.6 a second, with rounds of 8.3 s instead of 1.6 s (a ceiling bounds the length of a round and so the fairness search, not the rate). An action costs about 16 ms of which a commit is 0.5 ms (a sequence of round trips; not taken apart); one replica applies about 75 batch items a second to all its jobs together (13 ms an item, 7.4 ms of it the action). `SagatSchedulerLate` (60 s for 5 minutes) would not fire in any repetition (the gauge was above 60 s for 110 s at most), though the gauge the replicas scale on would. | The scaling runbook should say what the numbers say: N schedules (or batch items) due together are late by about N / 60 s (N / 75 s) per replica, two replicas about halve it, and 10,000 schedules of a per-minute spec need three replicas or more. A change of the pass (parallel firing inside a replica) is a design, not a setting. Nothing changed here. | **Said in the runbook 2026-11-09 (D2119)**: `docs/runbooks/schedules.md` 4.1 has the table and the extrapolation, as measurements of one machine. A design for firing in parallel stays open. |
| G2045 | **A burst of schedules in one namespace meets the default quota of 10,000 active runs, and the pass goes on refusing what it cannot admit.** The first launch of the benchmark at 10,000 schedules (the runs never finish: no worker) stopped at 9,998 schedules acted; the engine answers a refused admission with `ADMISSION_RETRY` and tries again, and the pass spent its rounds on the second time of every schedule it could not admit (10,500 refusals in 426 s, 50 a second, no end). Nothing says so: the schedules runbook does not name `ADMISSION_RETRY` or the quota, and `SagatSchedulerLate` fires on the lag that results, not on the cause. The benchmark raises the quota (`active_runs` and `tenant_active_runs`) for every scenario. | Name the quota and `ADMISSION_RETRY` in `docs/runbooks/schedules.md` ("a schedule did not start a run" list). Not changed here. (The rate in the first sentence is wrong: 10,500 refusals in 426 s are 25 a second, not 50.) | **Closed in the runbook 2026-11-09 (D2119)**: `schedules.md` 3 and 4.1 name the quota, `ADMISSION_RETRY`, the metric and what the operator sees. |
| G2046 | **A reset holds the row of its source run for the whole re-emission, about 0.93 ms for every event of the prefix.** The source row is locked from the start of the creation transaction to its commit: 0.95 s at 1,000 events, **4.65 s at 5,000** (`RESET_PREFIX_EVENTS`, the largest prefix accepted; 3 of 3 repetitions broke the bound of 1 s; the reset as a whole 4.7 s, within the 5 s bound by 0.15 to 0.24 s), and about 19 s at the hard ceiling of 20,000 that the plan names (a linear extrapolation: no code path accepts 20,000 today). Every other writer of that run (a cancel, a retention sweep, another reset) waits that long. The restore writes the events one statement at a time (the transaction is 5,000 round trips). | Write the restored rows in pages (one statement for hundreds of events) or lower the ceiling: a decision with the numbers above, tested in both roles; `RESET_PREFIX_EVENTS` is left at 5,000. Do not raise the ceiling toward 20,000 before that. | Open. The runbook says what it means for a person (D2120, `run-reset.md` 2.1); the restore is still written one statement at a time. |
| G2047 | **The points, the preview and the reset replay the whole journal of the source, and the replay costs more than linearly in its length, not in the length of the prefix.** `reset_points` 9 ms at 1,002 events, 89 ms at 5,002, 1,313 ms at 20,002 (3 of 3 repetitions: 1,273 to 1,328); the preview of the same prefix of 5,000 events 98 ms on a journal of 5,002 and 1,367 ms on one of 20,002; the reset of it 4.7 s and 6.0 s (the extra 1.3 s is before the transaction). Reading the journal is linear and cheap (the refusal of a point past the ceiling takes 46 ms at 20,002 events, in the preview and in the reset: it is answered before the replay, and creates no run); `verify_run` of a run a reset made takes 20 ms at 5,000 events and replay equals projection in every one of the 15 runs the resets made (5 for each of 3 repetitions at each size). | A journal of hundreds of thousands of events would make the page that lists the points a multi-second request: look at `reset_points` / `reset_plan` before the point list is shown for long runs. Not changed here. | Open. The runbook says what it means for a person (D2120, `run-reset.md` 2.1); the replay is unchanged. |
| G2048 | **The reconciliation of trigger schedules drains a backlog at 30 to 77 schedules a second, not at the 100 it advertises, and 100,000 triggers take 21 to 27 minutes, not the 17 of the estimate (G2023).** Measured with two tenants and the engine's own loop for 60 s: at 100,000 triggers 62 a second under the default settings (20 an entry a tenant, 39.6 writes in a round of 209 ms, then a pause of 400 ms: 27 minutes) and 77 a second with the per-tenant cap at 100 (193 writes in a round of 677 ms, then a pause of 1.9 s: 21 minutes); at 10,000 triggers 30 and 69 a second (42 to 46 rounds that wrote and 7 or 8 rounds that found nothing to write, each followed by the poll of 5 s: the sweep over so few triggers restarts and the loop rests). The pause that bounds the rate (10 ms a write) is added to the time of the round, so the pace can only be below 100. The convergence count (one statement over every shard, capped at 100,000) took 194 ms at the median with 100,000 unconverged and 6 ms at 10,000; the first window of a sweep 46 ms (max 336 ms) and 7 ms; every statement far inside the 4 s timeout; no write failed or was refused. | The estimate of G2023 is replaced by these figures; the runbook's "about 3 minutes for 10,000 triggers of one tenant" is about 6 at 2 tenants under the defaults. No change. | **Measured; the documents are corrected 2026-11-09 (D2118)**: ADR-0083, `schedules.md` and the notes of D2047 and G2023 say "at most 100 a second" and the measured range. |
| G2049 | **The check the handover makes under its lock depends on the plan the statistics give it, and took 2.85 s at 10,000 triggers.** `schedule_trigger_unconverged()` (an `EXISTS` over the drift view) stops at the first row when there is one and must look at every trigger when there is none, which is the state the handover needs. On a converged fleet its median was 1,009 ms at 5,000 triggers (3 of 3 repetitions: 999 to 1,014 ms), **2,853 ms at 10,000** (2,851 to 2,856 ms), 28 ms at 15,000, 43 ms at 20,000, 111 ms at 50,000 and 280 ms at 100,000: at 5,000 and 10,000 the planner chose a nested loop that rescans `schedules` for each trigger (a sequential scan of `schedules` under a `Materialize`, 99,990,000 rows removed by the join filter in the plan of one repetition at 10,000, kept in the fragment) and from 15,000 on it used `schedules_trigger_uniq`. The plan that was chosen at 10,000 is quadratic, so a fleet that gets it at a larger size would exceed the statement timeout of 4 s (the same plan at 14,000 triggers would take about 5.6 s: an extrapolation, not measured). The convergence count and the windows of the reconciliation did not show it (11 ms and 14 ms at 10,000). | Look at the plan of the check (and of the view under `EXISTS`) at sizes between 5,000 and 20,000 and make it independent of the statistics (a lateral or a `NOT EXISTS` form, or an index-forcing rewrite) with its own decision and test; the property the handover needs is "within the timeout at every size", which this run shows at the six sizes it measured (5,000 to 100,000) and not between 10,000 and 15,000, where the plan changes. Not changed here. | **Closed 2026-11-09 (D2117)**: the view looks the schedule up for each trigger, in place in `20270114000000_trigger_handover`; the check took 19.7 ms at 10,000 triggers (2,853 ms before), 10 ms at 5,000 (1,009 ms) and 263 ms at 100,000 (280 ms) (addendum of the report). The cause was not the statistics of the bulk-inserted tables (they were analyzed) but the singleton `schedule_trigger_handover`, never analyzed, and the planning of an `EXISTS` for its first row. |
| G2050 | **What this benchmark did not measure, and why.** The selection of 100,000 runs (the hard `maxItems`): starting 100,000 runs that never finish takes ten minutes here, and 10,000 took 0.3 s (31,000 runs a second); the cross-tenant fairness of the batch and the schedule passes (one tenant of one namespace in every scenario but the trigger ones, which have two); the batch pass on two replicas; the claim across processes (one process with a pool of 48 held the 32); the HTTP API in front of any of it (authentication, the rate limiter); workers that run what is started (the load of a real platform is more than the engine passes); a reset with reapplied signals or Updates, with a business id, `CANCEL_SOURCE` and its pass, and a reset at a prefix that the ceiling would accept above 5,000; backfills and catch-up storms; the delayed-start release pass; PostgreSQL on another machine or another version; any run longer than the hour each scenario takes; and the effect of the load of the other work on this machine (the figures vary by a few percent between repetitions, which the brackets show, and the load of the 50 ms poll variant was three times that of the others). | A run on a quiet machine with the database on another, and the scenarios above, belong to the release gate of a feature that makes a promise; NFR-018-005 makes none. | Accepted |
| G2051 | **The wake-up of the batch loop (D2116) covers the slot of a job and nothing else, and costs one idle round after each item.** A job whose selection waits for a transaction, and a job whose items are all deferred (the retry delay starts at 5 s and doubles), have no slot ahead, so they are still looked at once a poll (`SAGAT_BATCHES_POLL_MS`, 1 s): a deferred item can be retried up to a poll after it may be. And the round that applies an item is followed by one round that finds nothing due (a clock read and the due scan, which since D2123 also answers the slot ahead), so a lone job at the default rate costs one round more for each item than it would if the round that applied the item knew the slot; the work round does not ask because the next one follows at once and may find more due (a job that selects a page and applies an item in the same round, a round that stopped at its ceiling). | The round that worked and found no due job left behind could return the slot it has just written (`record_item` returns the job); not done: it is an optimisation of a statement a second, and the fairness argument for the idle round is simpler to hold. A wake-up for the retry time of a deferred item is a design of its own. | Accepted |
| G2052 | **The check the handover makes under its lock is now linear in the number of cron triggers, and nobody measured a fleet bigger than 100,000.** After D2117 it is one index probe for each trigger (about 2.6 microseconds: 263 ms at 100,000, 20 ms at 10,000), which, by extrapolation and not by measurement, would reach the 4 s statement timeout at about 1.5 million triggers; the lock timeout of 2 s is the writers' wait and does not bound the check once the lock is held. A platform with that many cron triggers is far from anything measured or asked for (the limit of triggers per workflow is 16). | A counter or a flag kept in the same transactions that write the triggers and the schedules (the quotas keep deltas this way) would make the check constant; not warranted by any size seen. Look at it before a deployment with more than half a million cron triggers hands over. | Accepted |
| G2053 | **Another statement joins the unanalyzed singleton `schedule_trigger_handover` with a plain `CROSS JOIN`.** (Not fixed here: it has `LIMIT 20` and a selective filter, `h.state = 'HANDED_OVER'` and `last_fire_at` after the handover over a partial index, so it cannot be quadratic the way the view was; and the same remedy for the singleton's missing statistic, `ANALYZE` or a seeded `reltuples`, was rejected by D2117 because the view no longer depends on it.) `legacy_fires` (`crates/infra-postgres/src/trigger_schedules.rs`, the tripwire of an old engine binary, run every round that reads the state) reads `workflow_triggers t CROSS JOIN schedule_trigger_handover h` with `h.state = 'HANDED_OVER'` and a partial index on `last_fire_at`; the table is estimated at 610 rows, as in G2049. It is the same ingredient (an estimate of the singleton that is six hundred times too high) in a statement that is not asked under a lock and was not measured: nothing here says it is slow, and its plan was not looked at. | Look at its plan with `EXPLAIN (ANALYZE, BUFFERS)` at 100,000 triggers after `HANDED_OVER`; read the singleton by a scalar subquery as the view does if it is not an index range. Not done: the benchmark's states are `LEGACY`. | Open |
| G2054 | **The window of a slot that comes while the due scan runs is held by a seam, not by the production call.** The statement reads the clock once at its start; `PgBatchStore::due_jobs_as_of(shards, limit, Some(at))` gives it the instant, and the test takes the instant before a slot comes. The production call passes none, so what the test cannot show is that the statement reads the clock at the start and not later (a mutant that took the bound of the slot ahead from the later clock survives the pass-level test and is killed only by the seam test); a lock cannot hold the statement back after the clock is read, because the relation locks are taken before execution. | A statement that sleeps for the test (a function in the schema for tests) would hold it with no seam; not warranted. | Accepted |
| G2055 | **A due job that the executor keeps finding not due makes the loop look again every 10 ms.** `DUE_BUT_NOT_YET` is a hundred rounds a second at the most (a clock read, the scan and a step each), and only while the engine's clock reads before the slot the database's clock calls due: by half a round trip and a little drift in the ordinary case, which ends at the first or second look. A `DatabaseClock` that cannot sync (it keeps the last difference and says so in a warning) and has drifted by seconds would be this case for as long as the outage lasts, where it used to be one look a poll. The loop keeps no state between rounds, so it cannot back off. Not measured. | Give the loop a back-off (double the wait after each round that was idle with a due job, up to the poll) if a deployment shows it; the clock that cannot sync is already a warning. | Open |
| G2056 | **The two windows of G2043 were found by reading the code and have not been reproduced under load.** Both reviewers reasoned from `due_jobs`, `next_slot_in` and the executor's clock check; the benchmark at 20 items a second did not show them (a miss costs one poll in 50 to 100 items, and the figure was 18.3 and 18.4 in six repetitions), and at 100 to 200 a second the lone job is bound by the cost of an item (13 ms against a slot of 5 ms) and is not idle between items. The tests hold each window deterministically (the store that moves the slot, the executor with a clock behind, the instant given to the scan), not the rate at which a loaded replica would meet them. | A run of the lone job at 100 a second on a loaded machine would say how often; not warranted: both windows are closed by construction. | Accepted |
| G2057 | **A run does not expose the schedule that started it beyond `startedBy`.** `GET /runs/{id}` and `GET /runs?scheduleId=` give `startedBy: system:schedule:<id>`; the nominal time, the kind and the revision (`runs.schedule_nominal_at`, `schedule_action_kind`, `schedule_revision`) are stored and never read, and the journal's `run.started` shows no detail through the REST events route. The nominal and actual times are on the **action** the run's id names (`GET …/schedules/{id}/actions`), so an operator can join them. FR-018-016, `contracts/journal-and-events.md` and SC-018-002 ("with `startedBy` and provenance") ask for it. Quickstart step 2.4 FAILED. | An optional `schedule {scheduleId, nominalTime, kind, revision}` on `Run` (and on the run list), read in the queries that build a run, with the OpenAPI, the reference, the Studio types, the six SDK models and a contract test (D2136). | Open |
| G2058 | **The CLI can neither list nor cancel the backfills of a schedule**, and the outcome `ALREADY_ACTED` of a backfill exists only as a count in the backfill (`GET …/backfills`: `counts`), not as an action row. The REST routes exist (`listScheduleBackfills`, `cancelScheduleBackfill`); `sagat schedule backfill` creates one and prints `PENDING`. | `sagat schedule backfills <id>` and `… backfill-cancel`; the page uses `curl` for the count. | Open |
| G2059 | **`sagat workflow publish` prints no workflow id and no CLI command finds one**, while a schedule definition (`action.workflowId`), a batch selector (`workflowId`) and a REST start need it; the quickstart reads the ids with `curl` and `jq`. | Print `workflow <name> <id>` after a publish, or a `sagat workflow list`. | Open |
| G2060 | **`sagat worker dev` ends when the engine does** (`the platform refused to hand out tasks (Unknown): transport error`) instead of asking again, so a demo that restarts the engine leaves every run waiting for a worker (the `SKIP` schedule of the demo recorded `SKIPPED_OVERLAP` for eight minutes until the worker was started again). A property of the developer tool of feature 012, found here. | The dev worker reconnects with a backoff, or says what to do. | Open |
| G2061 | **The four lines are not in `infra/docker/.env.example`** (D2131): the session could not read it. | The maintainer appends, commented and off by default, `SAGAT_SCHEDULES_ENABLED`, `SAGAT_BATCHES_ENABLED`, `SAGAT_RESET_REAPPLY_ENABLED`, `SAGAT_RESET_CANCEL_SOURCE_ENABLED`. | Open |
| G2062 | **Two texts still say what the demo showed false or leave out what exists:** the description of `BatchAction` in `openapi.yaml` ("UPDATE and REPIN are refused with 422 BATCH_ACTION_UNAVAILABLE in this build": `UPDATE` is accepted, a job of it was created with `202`) and the usage text of `sagat batch create` (the CLI parses `--update` and `--args` and the usage names neither, nor `update` among the actions). The runbook was corrected (D2134); the OpenAPI text feeds the blessed reference and the Studio's types, so it is changed with them. | Correct the contract text, bless `docs/reference/api.md`, regenerate the Studio types (`npm run api`), add the two flags to the usage and the golden. | Open |

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
- **O1952 — Which failure states must the browser journeys of T018-074 show?** The journeys show every refusal the real platform gives through the Studio's own controls (a name in use, a quota of zero, a stale revision, no run matched, a count capped, items skipped and failed with their reasons, the acknowledgement missing in the dialog, the identifier held by an open run, a finished run that remembers it, the limit of waiting resets, a reset that could not be made). Four failure states of the task's brief are not shown (G2034, G2035): the preview refused for a selector that is too wide (the Studio builds none), `RESET_POINT_NOT_ELIGIBLE` and `RESET_REEXECUTION_NOT_ACKNOWLEDGED` from the platform (the Studio does not send a request that gets them), `SOURCE_DID_NOT_END` (an hour at the least) and `AUTHORITY_REVOKED` (a member of the journey's own). (a) Accept their words in `ResetRun.test.tsx` and the REST suites as the evidence, and close T018-074 on the journeys it has; (b) add the journeys that can be made, with `page.route` altering the request the Studio sends (a refusal of the real platform for a request the Studio would not make), a member the harness mints, and an engine whose expiry is at its one-hour minimum and a clock that can move. **Recommended: (a) for the first three, (b) for `AUTHORITY_REVOKED` if the harness can mint a member.** T018-074 is *narrowed pending the owner* until it is answered.
