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
| G1980 | Four limits of the first reset store (D1977) are not settled. (1) A prefix that holds a scoped event (an event of a child scope, a parallel branch, a collection item) is blocked (`ResetBlock::ScopedEvents`, answered `RESET_POINT_NOT_ELIGIBLE`): restoring it needs the side tables of those scopes (`REMATERIALIZE` in D1605) and none exists yet. Points after the first scoped event of a run therefore cannot be reset to, and `reset_points` says so for them. (2) The ceiling `RESET_PREFIX_EVENTS` is a constant of the application, not a setting of the deployment or the namespace: a longer prefix is refused, not truncated. (3) Only `REQUIRE_TERMINAL` sources and `reapply = NONE` exist: `CANCEL_SOURCE` (T018-057) and `SIGNALS`/`UPDATES`/`ALL` (T018-056) are not built. (4) A reset is recorded by the new run alone (`rerun_of`, `reset_sequence` and the `run.reset` row); the `run_resets` table that lists the resets of a source and the routes that read it (T018-054) are not built. | (1) needs the scoped side tables, which belong to the features that own them; refusing is the honest answer and the refusal names the event type. (2) A setting is a product choice (how large a history may be copied in one transaction) that the owner decides with the namespace limits of 022. (3) and (4) are the next slices of US06 and each has its own task. | Open (T018-054, T018-056, T018-057) |
| G1979 | Three things about the reset points of D1976 are not settled. (1) By D1022 a loop that succeeds is not a point: it ends at its last checkpoint (`loop.iteration_checkpointed` that completes it) and `loop.finished` is journaled only for a loop that failed or was cancelled; `timer.fired`, `wait.satisfied` and the decision of an approval complete a root node too and are not in the list. The literal list is implemented, which is the conservative reading: those completions are not offered, and the next completion after them is. (2) The clause "a prefix instance compensated, skipped, resolved or reconciled later" cannot be reached by a journal the reducer accepts: a compensation exists only inside a saga, whose node is parked while its body runs, so every point before a compensated instance is already refused for a waiting step. The clause is implemented and its predicate is unit-tested, but no end-to-end journal exercises its wiring. (3) The corpus of `tests/reset.rs` has no approval run and no loop run, for (1): their completions are not candidates; approval decisions are covered where real approval flows exist (T018-055). | (1) widening the points is a product choice (more places to reset to) and the owner decides it; (2) is a defence for event shapes a later feature may add. | Open (owner, for 1) |
| G1978 | Two consequences of D1975 for the reset are not settled. (1) The retained arguments are on the receipt of the run that applied the Update. A reset re-emits the prefix of the source with its `retained: true` markers under a new run, so a reset of that reset run finds `retained: true` rows whose arguments sit on the receipts of the source: the reset must follow `reset_of` to the source's receipts, or copy the references. (2) A Signal-with-Start whose signal reaches an open holder is recorded `signal_with_start`, as the contract's table says, so a reset with `SIGNALS` (which re-delivers `via: signal`) does not deliver it again, although it is a signal to an open run; delivering it would make the reset re-signal what the caller asked for in one request that also may have started a run. | Both belong to the reset (T018-053, T018-054, T018-056): the rows are written now so the reset can choose; changing what `via` says later would need a rewrite of history. The reset reports a row it cannot reapply (`NOT_REAPPLIED`) with a reason. | Open (T018-053, T018-056) |
| G1977 | The `reapplyOnReset` capability (D1974) has no setting of a deployment: `set_reset_reapply` is called by no binary, so a workflow that opts in cannot be published anywhere yet. And the CLI's `sagat check`, which has no deployment, accepts a document that the server will refuse. | Wiring it before the reset exists would let workflows opt in whose arguments nothing retains or reads; the setting arrives with the reset flag of US06 (T018-056, T018-057), where the retention (T018-043) and the reapply are proved together. A client cannot know the capabilities of a server (ADR-0032, consequences), and the refusal names the field. | Open (T018-043, T018-056) |
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
