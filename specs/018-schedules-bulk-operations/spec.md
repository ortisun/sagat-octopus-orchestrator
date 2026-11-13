# Feature Specification: Schedules and bulk run operations

**Feature Branch**: `feature/018-schedules-bulk-operations`

**Created**: 2026-10-26

**Status**: Approved for implementation by the owner on 2026-10-29 (this feature only; ADR-0050 accepted; O1000–O1002 answered with the recommended defaults). Implementation in progress, see `tasks.md`

**Input**: Bring Temporal's operational features for many runs: a first-class Schedule that
generalizes today's five-field cron trigger without breaking it (calendar, interval and cron specs,
several per schedule, start/end bounds, jitter, DST rules, pause/resume with a note, trigger-now,
backfill over a past range, overlap policies, a catch-up window after an outage, a remaining-actions
limit, recent and next actions, an idempotent run identity per schedule and nominal time); durable,
resumable, rate-limited batch operations that select runs by a query and cancel, signal, update,
rerun, retry a step, pause, resume or mark them for review, with a dry-run count, per-item outcome,
progress, cancellation, audit and authorization per action; reset to a point as a new run built from
a source run's journal prefix, with a reapply policy and an explicit source-run policy; Signal-with-Start,
Update-with-Start, business workflow identifier uniqueness with reuse and conflict policies, and start
delay.

## Context and delivery boundary

Today a workflow version can carry a five-field cron trigger in an IANA zone
(`crates/domain/src/triggers.rs`, `crates/infra-ir/src/cron.rs`); a missed tick fires once, late, and
a trigger cannot be paused without republishing (004 D87, G84). Every run operation addresses one run;
a client cannot name "the order process of order 123", start a run later, or restart one from a point.

Every part of this feature keeps:

- **One transaction per transition** (constitution I, ADR-0002): a schedule action, a business-id
  claim, a batch item and a reset each commit their state with the run admission or change, journal,
  tasks, timers, outbox and audit.
- **A terminal run never returns to running.** Reset and rerun open new runs with provenance (ADR-0039
  guard). There is no terminate; cancellation drains attempts and compensates.
- **Signals are never buffered** (ADR-0030 decision 17): Signal-with-Start delivers only to a step
  waiting when its transaction commits.
- **Scheduler ownership** (ADR-0020): pass-driven work checks its shard fence in its own transaction;
  time decisions use the database clock.
- **No transition driven by a run itself** (completion, timer, timeout, deadline, terminal, handoff)
  **locks a schedule, business-id, batch or pending-reset row**; aggregates read run status, which is
  monotonic once terminal. Only two aggregate lock orders exist (ADR-0050).
- **Flow control and visibility** (features 022, 020, drafts): every run created here takes 022's run
  permit or queues like any admission; a reset cannot queue; a reset re-creates 020's search-attribute
  rows and marks restored history.

Hard-to-reverse rules: [ADR-0050](../../docs/adr/ADR-0050-many-run-operations-admit-under-stable-keys-and-never-buffer-or-reopen.md)
(Accepted 2026-10-29). Decisions, gaps and owner questions: [research.md](research.md).

## User Scenarios & Testing *(mandatory)*

### US01 — Run a workflow on a first-class schedule (Priority: P1)

A developer schedules a published workflow on business days at 09:00 in `Europe/Lisbon` and on the
first day of each month, with jitter, and sees when it will act next and what it did last.

**Why this priority**: Recurring work is the most common caller-less start; the current trigger cannot
be paused, bounded or inspected.

**Independent Test**: In a test deployment with the spacing floor at 10 s, create a schedule with an
interval and a calendar spec, let three nominal times pass, kill the engine between admission and
commit of one of them, and assert one run per nominal time, the next ten times and three recent actions.

**Acceptance Scenarios**:

1. **Given** a published workflow, **When** a developer creates a schedule with calendar, cron and
   interval specs, a zone, `startAt`/`endAt` and jitter, **Then** the answer carries the id, revision 1
   and the next nominal and actual times, and nothing runs before `startAt`.
2. **Given** a due nominal time, **When** two replicas consider it (one with a stale shard lease),
   **Then** exactly one run starts, keyed by schedule and nominal time, with
   `startedBy = system:schedule:<id>` and schedule provenance; the stale transaction is fenced and counted.
3. **Given** a crash after admission and before commit, **When** the engine restarts, **Then** no run
   exists for that attempt and the time is acted on once.
4. **Given** a spec that never fires inside its bounds or has two occurrences closer than the minimum
   spacing, **When** submitted, **Then** it is refused `VALIDATION_FAILED` with a reason; nothing is stored.
5. **Given** no active version or a full active-run quota, **When** a time falls due, **Then** no run
   starts and the time stays owed and is retried under the same key until the catch-up window expires.

### US02 — Operate a schedule (Priority: P1)

An operator pauses a schedule for maintenance with a note, resumes it, triggers it by hand, backfills
missed days and chooses what happens when a run is still open.

**Why this priority**: Outages and maintenance are when operators need these controls.

**Independent Test**: For each overlap policy start a long run, let two times fall due and assert the
outcomes; pause/resume and assert paused times are not caught up; backfill over acted times.

**Acceptance Scenarios**:

1. **Given** an active schedule, **When** it is paused with a note, **Then** no time is acted on while
   paused, note and actor are audited, and resume continues from the first time after the resume.
2. **Given** a paused schedule, **When** it is triggered now, **Then** one `TRIGGERED` run starts,
   remaining actions are unchanged, and a retry with the same key returns the same action.
3. **Given** a backfill of a past range, **When** processed, **Then** each time without an action gets
   one (sequential under the default `BUFFER_ALL`), acted times are `ALREADY_ACTED`, progress is
   visible and the backfill can be cancelled.
4. **Given** an open run, **When** the next time falls due, **Then** `SKIP` records `SKIPPED_OVERLAP`;
   `BUFFER_ONE` keeps one waiting and drops later ones (`DROPPED_BUFFER_FULL`); `BUFFER_ALL` buffers up
   to its ceiling; `CANCEL_OTHER` cancels the open run through ordinary cancellation and starts after
   it is terminal; `ALLOW_ALL` starts at once.
5. **Given** a three-hour outage over hourly times, **When** the platform returns, **Then** mode `ONE`
   acts on the earliest missed time in the window and records the others `COALESCED`; `ALL` acts on each
   up to the catch-up ceiling; times older than the window form one `MISSED_CATCHUP_WINDOW` summary.
6. **Given** `remainingActions = 2` and `pauseOnFailure`, **When** the first run fails, **Then** the
   schedule pauses with a system note; after resume the second action exhausts it.
7. **Given** the authorizing subject loses `run:start`, **When** the next time falls due, **Then** no run
   starts, the action is `AUTHORITY_REVOKED` and the schedule is paused and audited (O1000).

### US03 — Start a run under a business identifier, optionally delayed (Priority: P1)

A client starts the order process with business id `order-123` so that a second start cannot create a
duplicate open run, and starts a reminder that begins in two days.

**Why this priority**: Removes duplicate-run bugs; US04 depends on it.

**Independent Test**: Race two starts on one id; repeat with `USE_EXISTING`; end the run and test each
reuse policy inside and after the window; start with a delay and cancel during it.

**Acceptance Scenarios**:

1. **Given** an open holder of `order-123`, **When** another start uses it with `FAIL`, **Then** `409
   CONFLICT` `BUSINESS_ID_IN_USE` names the holder; with `USE_EXISTING` the holder is returned with `200`.
2. **Given** the holder ended `SUCCEEDED`, **When** a start uses `ALLOW_DUPLICATE_FAILED_ONLY` or
   `REJECT_DUPLICATE`, **Then** it is refused `BUSINESS_ID_REUSE_REJECTED`; `ALLOW_DUPLICATE` binds a new run.
3. **Given** a run that continues as new, **When** its successor is open, **Then** the id resolves to the
   chain and new starts conflict until the chain is final.
4. **Given** `startDelayMs = 172800000`, **When** the run is created, **Then** it is `WAITING` with
   `startDelayedUntil`, counts as active, holds its id, refuses signals with 409 and releases its entry
   once at the instant; cancelled during the delay it ends `CANCELLED` with no step run.

### US04 — Signal or update a run, starting it if necessary (Priority: P2)

A cart service sends `item-added` to the cart workflow of `cart-42`, starting it if no open run exists;
a booking service applies a declared Update to `booking-7` and receives its result in the same call.

**Why this priority**: Removes the check-then-start race for entity workflows; depends on US03.

**Independent Test**: Signal-with-Start on an absent id, an open waiting run, an open run not waiting,
and an entry that does not wait; Update-with-Start accepted, rejected and undeclared.

**Acceptance Scenarios**:

1. **Given** no open execution and an entry that waits for `item-added`, **When** Signal-with-Start is
   called, **Then** the run is created and the signal wakes that wait in the same transaction (`started: true`).
2. **Given** an open run waiting for the signal, **When** called, **Then** it is signalled exactly like
   `signalRun` (`started: false`).
3. **Given** an open run not waiting, or a new run whose entry does not wait, **When** called, **Then** it
   is 409 (`NOT_WAITING` / `SIGNAL_NOT_ACCEPTED_AT_START`) and no run, signal or binding is created.
4. **Given** Update-with-Start with `USE_EXISTING`, **When** the Update is accepted, **Then** start (if
   any), state, receipt, journal, audit and outbox commit together; a validator rejection leaves the run
   started with a stable `REJECTED` receipt; an undeclared Update or invalid arguments create nothing.

### US05 — Apply an operation to many runs (Priority: P2)

An operator cancels every `RUNNING` run of a faulty version created before 14:00 after seeing how many
match, watches progress and stops the job half way.

**Why this priority**: Incident response at scale.

**Independent Test**: Seed 1,000 runs, preview, start a cancel job at 50 items/s, kill the engine,
restart, cancel the job; assert at most one effect per run, one outcome per item, counters equal to
items, and no item transaction holding two run locks.

**Acceptance Scenarios**:

1. **Given** a filter, **When** previewed, **Then** the matched count (capped at `maxItems + 1`) and at
   most 20 run ids are returned and nothing changes.
2. **Given** a job, **When** it runs, **Then** it snapshots runs created at or before its creation, applies
   the action one run per transaction at the job rate and records `SUCCEEDED`, `SKIPPED` (reason) or
   `FAILED` (error code) per item with progress counters.
3. **Given** a running job, **When** cancelled, **Then** no further item is applied, applied items stay
   applied and the rest are `CANCELLED`.
4. **Given** a creator without `run:batch` or the action's permission, **When** creating, **Then** `403`;
   **Given** the creator loses access mid-job, **Then** the job stops `FAILED` (`AUTHORITY_REVOKED`).
5. **Given** a rerun job at the active-run quota, **When** an item is refused, **Then** it is deferred and
   retried, failing only after the bounded retry budget.

### US06 — Reset a run to a point as a new run (Priority: P2)

After fixing an activity, an operator resets a failed run to just after its `reserve` step, sees what
is restored, what runs again and which recorded signals can be reapplied.

**Why this priority**: Recovery without re-executing everything; builds on the rerun identity guard.

**Independent Test**: Reset a terminal run at each listed point; compare the new run's replay with its
projection and its restored outputs with the source at that sequence; crash during the reset; race a
`CANCEL_SOURCE` reset with the source's completion.

**Acceptance Scenarios**:

1. **Given** a terminal run, **When** points are listed, **Then** each gives sequence, node and time;
   points with an attempt in flight, a waiting step, an active scope, a compensating saga, an unknown
   outcome or later compensation of a restored step are absent.
2. **Given** a point, **When** previewed, **Then** the plan lists restored steps (approval decisions
   included), steps that run again with their effect markers, and reapply candidates; re-executing
   steps that completed in the source requires `acknowledgeReexecution: true`.
3. **Given** `REQUIRE_TERMINAL`, **When** the reset commits, **Then** a new run of the same pinned version
   holds the restored prefix (marked restored) with `rerunOf`/`resetFrom`, takes a fresh run permit when
   the version declares concurrency (or is refused `CONCURRENCY_PERMIT_UNAVAILABLE`), the source is
   unchanged and the business id is rebound if free.
4. **Given** `CANCEL_SOURCE` on an open source, **When** requested, **Then** the source is cancelled
   normally and the new run is created only once it is terminal and the point still eligible, else
   the reset fails `BOUNDARY_NO_LONGER_ELIGIBLE`.
5. **Given** reapply `SIGNALS`, **When** the run is created, **Then** recorded signals after the point are
   delivered in journal order to steps waiting within the creation transaction; the first that no step
   awaits ends signal reapply and it and the rest are `NOT_REAPPLIED`.

### US07 — Operate existing cron triggers as schedules (Priority: P3)

An operator sees the cron triggers of active versions in the schedule list and pauses one without
republishing.

**Why this priority**: Gives existing triggers the controls without changing how they fire.

**Independent Test**: Upgrade a deployment with a cron trigger while ticks fall due on old and new
engines; assert one run per tick, the 004 input document and key; pause and trigger work, spec edit refused.

**Acceptance Scenarios**:

1. **Given** an active version with a cron trigger, **When** schedules are listed, **Then** a `TRIGGER`
   schedule shows its expression, zone and next time.
2. **Given** that schedule after the operator handover, **When** paused, resumed or triggered, **Then** it
   behaves as any schedule; before the handover these are refused `SCHEDULE_TRIGGER_HANDOVER_PENDING`;
   **When** its spec is edited, **Then** it is refused `SCHEDULE_TRIGGER_OWNED`.
3. **Given** a rolling upgrade, **When** an old dispatcher and a new pass reach one tick, **Then** one run
   starts (shared key `cron-{epoch}` in scope `trigger:{id}`).

### Edge Cases

- DST gap (`SKIP`/`SHIFT_FORWARD`) and repeated hour (`ONCE_EARLIER`/`TWICE`); zone rules changing
  between computing and acting (the stored nominal instant is used).
- Jitter wider than the gap (bounded) or crossing `endAt` (no action after `endAt`).
- Update or delete while actions are buffered (they keep their revision / become `DROPPED_DELETED`).
- `ACTIVE` version changing between actions; `CANCEL_OTHER` racing the run's own completion (first
  commit wins, the cancel of a terminal run is a recorded no-op); trigger-now at the same instant as a
  scheduled time (distinct keys); backfill and catch-up over the same times (`ALREADY_ACTED`).
- A holder's terminal commit racing a new start (a stale "open" read yields `CONFLICT`/`USE_EXISTING`;
  a "terminal" read is never wrong); Signal-with-Start racing the awaited wait's timeout.
- A batch item whose run stays locked (bounded wait, item deferred); a run changed after selection
  (applicability decides); a batch rerun racing a manual rerun (two new runs, both audited).
- A reset twice with one key (same reset) or two keys (second conflicts on the business id under
  `KEEP`); a prefix over the ceiling (`RESET_PREFIX_TOO_LONG`); crash between a `CANCEL_SOURCE` request
  and source completion (pending reset is durable).
- A reset whose concurrency key has no free permit (refused, it cannot queue); a source purged by 020
  before or during the reset (`RUN_PURGED`); a 022 draft test run as source or batch target (refused /
  skipped); a Signal-with-Start whose new run 022 would queue (refused, nothing created); a start delay
  on a version with run-level concurrency (the permit is taken or the run queued at release, 022);
  a prefix holding an event type without a reset rule (refused `RESET_RULE_MISSING`).
- A trigger-owned schedule paused while old engines still run the legacy cron pass (refused until the
  handover); an old control plane publishing a cron trigger during the upgrade (the reconciliation
  pass creates its schedule).
- Quotas lowered to zero while schedules, jobs and delayed runs exist (new admissions refused,
  obligations drain).

## Requirements *(mandatory)*

### Functional Requirements

**Schedules**

- **FR-018-001 — Schedule resource.** Namespace-scoped create, read, list, update, delete, pause, resume,
  trigger and backfill. A schedule has a stable id, a namespace-unique name, origin `API` or `TRIGGER`,
  state `ACTIVE`/`PAUSED`/`DELETED`, an immutable current revision and optimistic concurrency
  (`expectedRevision`, else `409 REVISION_CONFLICT`).
- **FR-018-002 — Spec model.** A spec MUST be the union of calendars (second, minute, hour, day of
  month, month, year, day of week as values, ranges, steps), five-field cron strings of the feature-004
  grammar and epoch-aligned intervals with phase; minus exclusions; within optional `startAt`/`endAt`;
  in one IANA zone (UTC default); DST gap `SKIP` (default) or `SHIFT_FORWARD`, ambiguous time
  `ONCE_EARLIER` (default) or `TWICE`.
- **FR-018-003 — Spec validation.** Writes MUST refuse with `VALIDATION_FAILED` a spec over its
  ceilings, an unknown zone, no occurrence in bounds (`SCHEDULE_NEVER_FIRES`) or two of the next 1,000
  occurrences closer than the minimum spacing (`SCHEDULE_TOO_FREQUENT`). Occurrence computation MUST be
  deterministic for a spec, zone database version and instant.
- **FR-018-004 — Nominal time and identity.** Each action has a nominal time, a kind (`SCHEDULED`,
  `TRIGGERED`, `BACKFILL`) and an identity origin: `SPEC` for scheduled and backfill actions, `MANUAL`
  for trigger-now. At most one action per (tenant, schedule, origin, nominal time) MUST exist; its run
  MUST be admitted under scope `schedule:{id}` with key `nominal-{epoch ms}` (`SPEC`) or
  `manual-{epoch µs}` (`MANUAL`). Jitter MUST be a deterministic function of schedule id and nominal
  time, bounded by the setting and the next gap.
- **FR-018-005 — Firing transaction.** Acting on a time MUST commit in one transaction the shard fence
  check, schedule CAS, decision, authority check, run admission (journal, tasks, timers, outbox, audit),
  action record and schedule state. A refused admission MUST commit nothing of the action, keep the time
  owed and retry it with bounded backoff under the same key. Due-ness uses the database clock.
- **FR-018-006 — Overlap.** `SKIP` (default), `BUFFER_ONE`, `BUFFER_ALL` (bounded; overflow
  `DROPPED_BUFFER_FULL`), `CANCEL_OTHER` (ordinary cancellation of open runs; new action buffered until
  they are terminal) and `ALLOW_ALL`. No terminate variant.
- **FR-018-007 — Open runs by looking.** A schedule MUST track its runs last seen open and learn their
  completion by reading run status; a schedule transaction changes runs only through the ordinary
  cancel use case (`CANCEL_OTHER`); no run transition MAY lock or write a schedule row.
  `pauseOnFailure` pauses with a system note when a run ends `FAILED`, `TIMED_OUT`,
  `COMPENSATION_FAILED` or `PARTIAL`.
- **FR-018-008 — Catch-up.** Missed times MUST follow `catchupWindow` (default 366 d, 10 s..366 d) and
  `catchupMode` (`ONE` default: earliest missed in the window, the others one `COALESCED` summary;
  `ALL`: each in order up to the catch-up ceiling, the rest one `COALESCED` summary); misses outside the
  window MUST be one `MISSED_CATCHUP_WINDOW` summary with count and range. Summaries are action rows
  with a count, never one row per missed time. Trigger-now and backfill are not limited by the window.
- **FR-018-009 — Pause and resume.** Each takes a plain-text note (≤ 512 chars), records actor and time
  and is audited. Paused times MUST NOT be acted on or caught up; resume continues from the first time
  after the resume instant.
- **FR-018-010 — Trigger now.** One `TRIGGERED` (`MANUAL`) action at the request's database time,
  also while paused, optional overlap override, deduplicated by `Idempotency-Key`, no remaining-actions
  decrement; it never shares an identity with a scheduled time.
- **FR-018-011 — Backfill.** `[start, end)` with `end` not after now, bounded in actions and in
  concurrent backfills, processed in bounded transactions with progress and cancellation, default
  `BUFFER_ALL`; times with an action are `ALREADY_ACTED` and never get a second run.
- **FR-018-012 — Remaining actions.** Optional `remainingActions` (1..1,000,000), decremented only by
  `SCHEDULED` actions that start a run; at zero the schedule acts as paused with no next action.
- **FR-018-013 — Describe.** Reads return revision, state, note, next N times (default 10, ≤ 100,
  computed on read), recent actions (default 10, ≤ 100, with kind, times, outcome, reason, run), tracked
  open runs, buffered count and per-outcome counters. Actions are retained at least for the last 1,000
  or 30 days per schedule, within the hard ceiling.
- **FR-018-014 — Revisions, update, delete.** Updates create immutable revisions; an action uses the
  revision it was decided under; after an update the next time is the first after the update. Delete
  tombstones the schedule, records buffered actions `DROPPED_DELETED`, leaves runs untouched, frees the name.
- **FR-018-015 — Schedule authority.** A schedule records `authorizedBy` (last creator, updater or
  resumer). Every action MUST check that subject's current `run:start` (and 017 database authority
  where needed); otherwise record `AUTHORITY_REVOKED`, pause with a system note and audit.
- **FR-018-016 — Scheduled runs.** The action names workflow, version policy (`ACTIVE` or pinned),
  input (validated against the target input schema at write and at action), optional run timeout,
  optional `nominalTimeField` (a declared top-level property set to the nominal time, RFC 3339 UTC) and
  optional business-id prefix with policies (id `<prefix>-<nominal time>`). Runs have
  `startedBy = system:schedule:<id>` and expose schedule id, nominal time, kind and revision.
- **FR-018-017 — Trigger-owned schedules.** Each cron trigger of an active version MUST appear as a
  `TRIGGER` schedule fired by the schedule pass with feature 004's key (`trigger:{id}`/`cron-{epoch}`),
  input document, `system:engine`, `ALLOW_ALL`, catch-up `ONE`/366 d, no jitter, DST
  `SKIP`/`ONCE_EARLIER`. Its spec changes only by publication (`SCHEDULE_TRIGGER_OWNED`). Pause, resume
  and trigger MUST be refused (`SCHEDULE_TRIGGER_HANDOVER_PENDING`) until an audited operator handover,
  an activation of the instance-capability registry (FR-018-052) requiring `schedules-v1` from every live
  engine, stops the legacy cron pass; a
  reconciliation pass MUST keep trigger-owned schedules equal to the cron triggers in force. A rolling
  upgrade MUST NOT start a tick twice.

**Business identifiers and start options**

- **FR-018-018 — Business identifier.** A start MAY carry `businessId` (1–200 visible ASCII). At most
  one open execution (a run, or a chain by its initial run) per (tenant, namespace, id) at any commit.
- **FR-018-019 — Conflict policy.** With an open holder, `FAIL` (default) refuses `409 CONFLICT`
  `BUSINESS_ID_IN_USE` naming the holder; `USE_EXISTING` returns it with `200` and creates nothing. No
  terminate-existing policy.
- **FR-018-020 — Reuse policy.** With a closed holder inside the reuse window (default 30 d,
  namespace-configurable ≤ 366 d), `ALLOW_DUPLICATE` (default) allows; `ALLOW_DUPLICATE_FAILED_ONLY`
  allows unless the holder ended `SUCCEEDED` (`COMPENSATED`, `PARTIAL` count as not succeeded);
  `REJECT_DUPLICATE` refuses `BUSINESS_ID_REUSE_REJECTED`. After the window the id is free.
- **FR-018-021 — Registry semantics.** The binding is claimed in the creation transaction in the
  ADR-0050 lock order; transitions driven by the run MUST NOT lock or write it; openness is derived from
  the holder's terminal status (a chain is open until final). A business id is customer content: 020
  erasure MAY set `runs.business_id` to null and delete the run's bindings; no other change is allowed.
- **FR-018-022 — Lookup.** `run:read` holders can read a binding (holder run or chain, open, last status,
  reusable-after) and filter run lists by business id, business-id prefix and schedule; with feature 020
  present these filters are 020's `BusinessId` and `ScheduleId` system attributes (one semantics in
  `listRuns` and in queries).
- **FR-018-023 — Start delay.** A start MAY carry `startDelayMs` (1 ms..366 d): the run is created
  `WAITING` with `startDelayedUntil`, counts toward active-run quotas, holds its id and releases its
  entry once at the instant through a fenced run-shard pass. Cancel and deadline apply; signals are
  refused 409; declared Updates are admitted; not combinable with Signal-with-Start or schedules. With
  022 run-level concurrency the run holds no permit during the delay and the release transaction
  acquires it or enqueues the run (022 FR-022-018).

**Signal-with-Start and Update-with-Start**

- **FR-018-024 — Signal-with-Start.** Under the binding lock: an open holder is signalled exactly as
  `signalRun` (409 `NOT_WAITING` if no step waits, nothing changed); otherwise, subject to reuse policy,
  a run is created and the signal is delivered in the same transaction to every step waiting after its
  initial reduction; if none waits (including a new run that 022 would queue) the whole request rolls
  back with 409 `SIGNAL_NOT_ACCEPTED_AT_START`. No signal is ever buffered.
- **FR-018-025 — Update-with-Start.** Takes a business id, a declared Update (`updateKey`, arguments,
  optional expected revision) and conflict `USE_EXISTING` (default) or `FAIL`; start (if any), state,
  receipt, journal, audit and outbox commit in one transaction. Pre-admission failures create nothing;
  an admitted business rejection leaves the run started with a stable `REJECTED` receipt (ADR-0031).
- **FR-018-026 — With-start idempotency.** Both require `Idempotency-Key` with a fingerprint over every
  field; replay returns the original outcome after current authorization; a changed body is
  `409 IDEMPOTENCY_KEY_REUSED`.

**Batch operations**

- **FR-018-027 — Batch job.** Create a job with a selector (structured filter over workflow, version,
  status, origin, schedule, creation range, business-id prefix; or ≤ 1,000 run ids; 020's query when
  present), an action, its parameters, a justification (8–2000 chars), `maxItems` and a rate;
  idempotent by `Idempotency-Key`.
- **FR-018-028 — Preview.** Returns the matched count capped at `maxItems + 1` and ≤ 20 run ids, changes
  nothing, finishes within a bounded statement time.
- **FR-018-029 — Selection.** Selection MUST start only after every transaction in progress at job
  creation has finished (`pg_snapshot_xmin` past the horizon recorded at creation), then page by run id
  over runs created at or before the creation instant, in bounded transactions, up to `maxItems`. The
  result is evaluated per round and fixed once an item is materialized; a materialized item's filter is
  not re-evaluated. With feature 020 the selector is 020's `RunSelector` (`contracts/list-query.md`:
  creator's rights re-resolved each round, keyset `(StartedAt, RunId)`, bounded preview); a query while
  020's capability is off is refused `VALIDATION_FAILED`/`QUERY_UNSUPPORTED_OPERATOR`.
- **FR-018-030 — Per-item transaction.** Each item commits in its own transaction that locks only its
  run (bounded wait), checks fence and job state, applies the action through the single-run use case
  under key `batch:{job}:{run}`, and records the outcome and counters with the effect. No batch
  transaction holds two run locks. Creator authority is rechecked per item.
- **FR-018-031 — Outcomes and progress.** Items end `SUCCEEDED`, `SKIPPED` (reason), `FAILED` (error
  code) or `CANCELLED`; quota/rate refusals defer with bounded backoff; counters equal outcomes; the job
  ends `COMPLETED`, `CANCELLED` or `FAILED` (for example `AUTHORITY_REVOKED`).
- **FR-018-032 — Batch cancellation.** Stops new items after the in-flight one, marks the rest
  `CANCELLED`, undoes nothing, is audited.
- **FR-018-033 — Rate, concurrency, retention.** Items are applied no faster than the job rate (durable
  slot); active jobs per namespace are bounded; jobs and items are retained for the window (default
  30 d, ≤ 90 d) and purged in bounded batches.
- **FR-018-034 — Batch authorization and actions.** Creation requires `run:batch` plus every
  permission of the single action. Actions: `CANCEL`, `SIGNAL`, `UPDATE`, `RERUN`, `RETRY_STEP`,
  `PAUSE`, `RESUME`, `MARK_MANUAL_REVIEW`, and `REPIN` (only when feature 019 is enabled, else
  `BATCH_ACTION_UNAVAILABLE`; needs `deployment:manage` and `run:operate`; each item is 019's single-run
  repin, FR-019-009); the set grows only additively under the same contract.

**Reset to a point**

- **FR-018-035 — Reset points.** List the eligible points of a root run outside continuation chains
  that is neither a 022 test-tree run (`RESET_SOURCE_TEST_RUN`) nor purged (`RUN_PURGED`), and whose
  prefix holds only event types with a rule in the reset-rule registry of
  `contracts/journal-and-events.md` (`COPY`, `REMATERIALIZE`, `EXCLUDE`, `REFUSE`; else
  `RESET_RULE_MISSING`), applying those rules:
  `run.started` and completions of root nodes whose replayed state has no attempt in flight, no
  waiting step, no active scope, no compensating saga, no unknown outcome or manual review, and no
  prefix instance compensated, skipped, resolved or reconciled later.
- **FR-018-036 — Reset preview.** List restored steps (approval decisions included), re-executed steps
  with effect markers (`compensateWith`, `reconcileWith`, `irreversible`, `highRisk`) and reapply
  candidates; re-executing steps that completed in the source requires `acknowledgeReexecution: true`.
- **FR-018-037 — Reset execution.** One transaction creates a run of the source's pinned version and
  input whose journal re-emits source events `1..N` under the new identity, then `run.reset` (source, N,
  prefix digest) and the settle reduction, with `rerunOf`/`resetFrom`, the ADR-0039 guard, new effect
  keys and a justification. Every re-emitted row MUST carry `restored: true`. Its replay MUST equal its
  projection without the source; telemetry and 020 analytics skip the restored prefix; 020's
  `run_search_attributes` rows are re-created from the re-emitted upserts; a run permit is acquired
  after `run.reset` (FR-018-050).
- **FR-018-038 — Source policy.** `REQUIRE_TERMINAL` (default) refuses an open source
  (`RESET_SOURCE_NOT_TERMINAL`); `CANCEL_SOURCE` cancels it normally and creates the run only once it is
  terminal and the point still eligible (`BOUNDARY_NO_LONGER_ELIGIBLE` otherwise). The source never
  returns to running.
- **FR-018-039 — Reapply.** `NONE` (default), `SIGNALS`, `UPDATES`, `ALL` over signals recorded
  `via: signal` and Updates whose declaration opts in with `reapplyOnReset: true` (their arguments are
  then retained as charged content) after N, in journal order, ≤ the ceiling, inside the creation
  transaction; the first undeliverable signal ends signal reapply; each candidate is reported
  `REAPPLIED`, `REJECTED` or `NOT_REAPPLIED` with a reason (`NOT_RETAINED`, `LEGACY_EVENT`).
- **FR-018-040 — Business id of reruns and resets.** `KEEP` (default) binds the source's id when no
  other open execution holds it, else `BUSINESS_ID_IN_USE`; `DROP` starts without one.

**Cross-cutting**

- **FR-018-041 — Authorization and tenant boundary.** The catalog gains `schedule:read`,
  `schedule:write`, `schedule:operate`, `run:batch` with the plan's grants. Tenant and namespace come
  from the authenticated context; foreign schedules, jobs, resets, runs and business ids are opaque
  `NOT_FOUND` with the probe audited.
- **FR-018-042 — Disclosure.** Schedule input, batch payloads and retained Update arguments are charged
  content; ordinary schedule, batch and reset reads return digests and sizes only; schedule input values
  only through an audited read with feature-016 audiences. Business ids MUST NOT appear in metric
  labels, span attributes, logs, audit reasons or outbox payloads. Payload-referencing columns
  (`schedule_revisions.input_digest`, `batch_jobs.payload_digest`, retained Update arguments) MUST be
  registered in 020's content-store registry (`specs/020-visibility-analytics-archival/data-model.md`).
- **FR-018-043 — Typed errors.** Existing `Error.code` set with the `details.reason` values of
  `contracts/errors.md`, retryability and trace id, no payloads.
- **FR-018-044 — Observability and audit.** OTel spans, the plan's fixed-label metrics and redacted
  logs; schedule changes and actions, business-id conflicts, with-start outcomes, batch lifecycle and
  every item effect, and resets are audited in the decision's transaction.
- **FR-018-045 — Finite ceilings.** Every resource has a default and a hard ceiling (plan table);
  effective value = min(request, namespace, deployment); zero refuses new admissions; admission fails
  before work.
- **FR-018-046 — Contracts and compatibility.** REST additive under `/v1`; worker protocol unchanged;
  the workflow IR gains only the optional, capability-gated `reapplyOnReset` on Update declarations
  (ADR-0032); AsyncAPI gains only optional `scheduleId`, `nominalTime`; existing starts, signals,
  reruns, cron triggers and SDK clients behave unchanged.
- **FR-018-047 — Studio.** Schedule list/detail/create/edit, pause/resume with note, trigger-now,
  backfill with preview; batch creation from a run-list filter with preview and confirmation, progress
  and items; reset dialog with points and preview; business-id and delay fields; provenance links —
  with design tokens, WCAG AA, table alternatives and no sample values without data (FR-007-010).
- **FR-018-048 — Five SDKs.** TypeScript, Go, Python, C# and Java expose the schedule, start-option,
  with-start, batch and reset operations with one semantics and pass shared scenarios in
  `tests/contract/sdk-conformance/scenarios/`.
- **FR-018-049 — Delivery.** Runbooks (schedules, batch, reset), alert rules for late schedules and
  stalled batches, trigger-owned migration notes and the reproducible demo of `quickstart.md`.
- **FR-018-050 — Flow-control admission (022).** Every run created by a schedule action, a business-id
  start, Signal-/Update-with-Start, a batch rerun, a rerun or a reset MUST pass 022's run-permit
  admission (acquire or enqueue) in its creation transaction (slot 7 of ADR-0050 decision 7); a delayed
  start does so in its release transaction. A full queue is an admission refusal (owed
  time, deferred item, synchronous 429 `CONCURRENCY_QUEUE_FULL`). A reset MUST NOT queue: without a free
  permit it is refused 409 `CONCURRENCY_PERMIT_UNAVAILABLE` (a `CANCEL_SOURCE` completion ends
  `FAILED`). The `run.reset` reducer clears restored permit state, so ADR-0056's holder count sees only
  the fresh acquisition. Batch actions skip test-tree runs (`TEST_RUN`).
- **FR-018-051 — Visibility interplay (020).** A reset or rerun of a purged source MUST be refused
  `CONFLICT`/`RUN_PURGED` (020's code) after the ADR-0039 guard, and a purge committing first MUST make the reset roll
  back with the same answer (payload foreign keys of re-emitted rows). Restored rows (`restored: true`,
  `reset_of` on the re-emitted `run.started`) MUST let 020 analytics skip restored attempts.
- **FR-018-052 — Instance-capability registry.** Every control-plane and execution-engine process MUST
  register its kind, binary version and capability set before serving, heartbeat on the database clock
  (10 s; live = seen within 30 s), and refuse to serve while a recorded requirement for its kind names a
  capability it lacks (re-checked whenever the requirement epoch moves). A requirement MUST be recorded
  only by an audited operator activation command that, in one transaction, verifies every live instance
  of the required kinds and records it — never inside a tenant or user transaction; until then the
  feature refuses its own use with its own reason. 018's trigger handover is its first
  activation; features 029 and 031 activate through the same registry.

### Non-functional requirements

- **NFR-018-001 (durability)**: Committed actions, bindings, item outcomes and resets survive process
  and broker loss; duplicates, retries and racing replicas never create a second run for one (schedule,
  nominal time), a second open holder of one business id, or a second effect for one (job, run).
- **NFR-018-002 (security)**: Zero cross-tenant or cross-namespace access in the battery; authorization
  fails closed at creation and at every action or item; secrets never enter schedule input, batch
  payloads, journal, logs or audit.
- **NFR-018-003 (resource bounds)**: Ceilings are enforced before work; a lowered ceiling refuses new
  admissions and lets existing obligations drain.
- **NFR-018-004 (compatibility)**: Existing runs, cron triggers, clients and SDKs keep their behaviour
  across a rolling upgrade; rollback disables new admissions while readers keep serving recorded data.
- **NFR-018-005 (performance)**: Firing lag, batch item rate, business-id claim latency and reset
  transaction time are measured (p50/p95/p99) on a declared environment; nothing is promised.
- **NFR-018-006 (observability)**: Every accepted operation has persisted audit and bounded OTel
  evidence; metric labels are closed sets without tenant, namespace, run, schedule, job or business ids.

### Key Entities

- **Schedule** (name, origin, state, note, authority, remaining actions, next wake, counters) and its
  immutable **revisions** (spec, policies, action).
- **Schedule action** — one decision per nominal time: kind, revision, outcome, reason, run.
- **Backfill** — a bounded past range with cursor, overlap policy and progress.
- **Business-id binding** — namespace-unique id → current execution, reuse window.
- **Delayed start** — a run created waiting with the instant its entry is released.
- **Batch job / batch item** — selection snapshot, action and parameters, rate, state, counters; one item
  per selected run with its outcome.
- **Reset point / plan / reset** — an eligible sequence; what is restored, re-executed and reapplied;
  the request linking source, sequence and new run.

## Success Criteria *(mandatory)*

All evidence below is future evidence; none exists yet.

| Criterion | Required automated proof | Requirements |
|---|---|---|
| SC-018-001 | At least 200 table cases over every spec kind, zone and DST case give the expected nominal and jittered instants, equal across two engine processes. | FR-018-002..004 |
| SC-018-002 | Crash at each statement of the firing transaction, a stale-fence replica and a duplicate pass give exactly one run and one action per nominal time, with `startedBy = system:schedule:<id>` and provenance (real PostgreSQL). | FR-018-004, FR-018-005, FR-018-016; NFR-018-001 |
| SC-018-003 | Every overlap, catch-up, pause/resume, trigger-now, backfill, remaining-actions, pause-on-failure and revision/delete scenario of US02 records the expected outcome table. | FR-018-006..014 |
| SC-018-004 | Revoking the authorizing subject pauses the schedule at its next action; no run starts. | FR-018-015 |
| SC-018-005 | A rolling upgrade with old and new engines firing one cron trigger gives one run per tick and the 004 input document; pause is refused before the handover and honoured after it; a trigger published by an old control plane gets its schedule from reconciliation. | FR-018-017, FR-018-046 |
| SC-018-006 | 32 concurrent starts on one id give one open run; every reuse/conflict policy matches its table inside and after the window; a chain keeps the binding until final. | FR-018-018..022 |
| SC-018-007 | Delayed runs release once at the instant, are cancellable and refuse signals; a crash around release leaves one release; with 022 run-level concurrency the release acquires the permit or queues the run. | FR-018-023, FR-018-050 |
| SC-018-008 | The four Signal-with-Start and three Update-with-Start cases of US04 pass, plus a Signal-with-Start whose new run 022 would queue; no signal is stored without a waiting step. | FR-018-024..026, FR-018-050 |
| SC-018-009 | A 1,000-run job with an engine kill, a lost broker hint and a cancellation applies each run at most once, counters equal items, a run whose creating transaction was in progress at job creation is selected, and a lock monitor never sees a batch transaction with two run locks. | FR-018-027..033 |
| SC-018-010 | Missing `run:batch`, missing action permission (including `deployment:manage` for `REPIN`), mid-job revocation, foreign run ids and `REPIN` without feature 019 are refused or skipped as specified. | FR-018-034, FR-018-041 |
| SC-018-011 | For every eligible point of a seeded corpus (including a run holding a 022 run permit and one with search-attribute upserts) the reset run's replay equals its projection, restored outputs and attribute rows equal the source at N, the 022 holder count equals the open non-restored acquisitions, a prefix with an event type lacking a registry rule is refused, and ineligible, test-tree and purged sources are refused. | FR-018-035..037, FR-018-050, FR-018-051 |
| SC-018-012 | `CANCEL_SOURCE` racing source completion, crash during the reset transaction, reapply order and `NOT_REAPPLIED` reporting behave as specified; the source never reopens. | FR-018-038..040 |
| SC-018-013 | OpenAPI additive, AsyncAPI baseline only grows, IR and worker baselines unchanged, reasons documented; five SDKs pass the shared scenarios. | FR-018-043, FR-018-046, FR-018-048 |
| SC-018-014 | Canary schedule inputs, batch payloads and business ids are absent from logs, spans, metrics, audit and outbox; each new quota refuses before work; real OTel export has fixed labels only. | FR-018-042, FR-018-044, FR-018-045 |
| SC-018-015 | A real browser runs the schedule, batch and reset flows keyboard-only with WCAG AA contrast and table alternatives; the quickstart demo runs. | FR-018-047, FR-018-049 |
| SC-018-016 | A barrier test runs every run-driven transition (completion, timer, wait timeout, deadline, terminal, handoff) against each aggregate path; `pg_locks` never shows a run-driven transaction holding a schedule, business-id, job, item, obligation or pending-reset row, and no deadlock occurs. | FR-018-007, FR-018-021, FR-018-030 |
| SC-018-017 | An activation racing a registering instance that lacks the capability never leaves that instance serving; an instance whose capability is missing after an activation answers `BINARY_CAPABILITY_MISSING` (API) or stops its passes (engine); an instance silent for 30 s no longer blocks an activation. | FR-018-052, FR-018-017 |

## Assumptions

- The database clock is the time authority; tests use short intervals with the spacing floor (10 s)
  and real waits, and the engine's injectable clock (`crates/application/src/clock.rs`) only where the
  engine reads time; time skipping belongs to 028, whose time-class catalog
  (`specs/028-developer-tooling/contracts/time-classes.md`) classifies this feature's instants (schedule
  and delay times `workflow_due`/`workflow_record`, batch selection bounds `infrastructure`).
- The zone database is the one compiled into the engine (`chrono-tz`); a rule change arrives with an
  engine upgrade and never moves recorded nominal instants.
- Features 020 and 022 are drafts: until they merge, batch selection uses the structured filter and
  no permit or search-attribute row exists; FR-018-050/051 apply from the moment they do. Feature 029 is not merged: when it is, its evaluator decides which runs a batch
  may select, and schedule firing stays an engine-internal admission (draft FR-029-023).
- Feature 031 (client-side encryption) is not merged. Its encoded-field registry
  (`specs/031-client-side-encryption/contracts/encoded-field-registry.md`) lists this feature's
  positions: schedule input, with-start inputs, batch signal payloads and reapplied signals are write
  paths whose encoded fields stay opaque (envelope shape checked, never decrypted); `nominalTimeField`
  MUST name a non-encoded property (`ENCODING_NOT_PRODUCIBLE`); the business-id prefix is a literal;
  Update arguments cannot carry encoded values; schedule input disclosure shows an encoded field as
  encoded.
- The built-in roles stay as they are; four permissions are added.

## Dependencies

Existing: 001, 002 (runs, journal, rerun, signals), 003 (cancellation, compensation), 004 (cron
triggers, D85/D87/G84), 006 (conformance), 007 (Studio), 008 (operations, telemetry), 009 (RBAC,
quotas, audit), 010 (shards, ADR-0020), 011 (pinned versions), 014 (chains, declared Updates,
ADR-0030/0031/0032), 016 (classification), 017 (managed-database authority, ADR-0039 guard).
New: **022** (run permits through its T10 admission path, ADR-0056; `test_tree` runs), **020**
(`RunSelector` in `contracts/list-query.md`, `run_search_attributes`, `RUN_PURGED`, erasure of
customer content, content-store registry, `BusinessId`/`ScheduleId` system attributes); the drafts
that declared rules in the reset-rule registry (019, 020, 021, 022, 024, 025, 026, 030) and cite
ADR-0050's admission lock slots; optional: 019 (`REPIN` batch action), 026 (notifications), 028 (time
skipping, time-class catalog), 029 (folder visibility), 031 (encoded-field registry).

## Out of scope

Terminate; batch reset and deletion; reset of chain segments, child runs or inside active scopes;
buffered signals; test-mode pinned outputs and draft partial execution (022); search attributes and
query language (020); notification channels (026); time skipping (028); shared holiday calendars;
schedules that start anything other than a workflow run.

## Owner questions

See [research.md §5](research.md#5-owner-questions-the-plan-implements-the-recommended-defaults):
O1000 (schedule authority after access loss — recommended: pause), O1001 (business-id reuse window —
recommended: 30 days, up to 366), O1002 (who holds `run:batch` — recommended: admin and operator).
