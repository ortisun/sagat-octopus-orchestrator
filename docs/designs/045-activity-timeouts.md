# Activity timeouts, retry per failure code and buffered signals

*   **Author:** Claude Code, for the owner (under Owner decision 2026-11-20-6 and Owner decision 2026-11-24-4)
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

Release 0.3.0 gives a leased step the four timeouts a Temporal activity has, a retry policy that decides per failure code, and
signals that a run keeps until a wait consumes them. The owner put them in 0.3.0 (Owner decision 2026-11-24-1: "activity
timeouts (schedule-to-start, schedule-to-close, heartbeat timeout per activity) and non-retryable error types"). Buffered signals
are in this design because no other design covers them (see "Where buffered signals belong" below).

What a workflow author can write, on an `activity` or a `script` node:

| Field | Meaning | Temporal's name |
|---|---|---|
| `timeoutMs` (exists) | One attempt, from its claim to its report | start-to-close |
| `scheduleToStartMs` (new) | One attempt, from the moment it can be claimed to its lease | schedule-to-start |
| `scheduleToCloseMs` (new) | The whole step, from its first release to its end: every attempt, retry wait and gate wait | schedule-to-close |
| `heartbeatTimeoutMs` (new) | The lease of one attempt lapses after this long without a renewal | heartbeat timeout |
| `heartbeatFrom` (new) | `host` (the host renews while the handler runs, today's behaviour) or `handler` (only a handler's `heartbeat()` renews) | the activity heartbeats |
| `retry.nonRetryableErrorCodes` (new) | Failure codes that end the step at once | `nonRetryableErrorTypes` |
| `retry.byErrorCode` (new) | A retry policy for named failure codes, over the node's base policy | none |

And on the workflow: `signals: [{"name": "approve", "buffered": true, "maxPending": 10}]`, which keeps a signal that no step waits
for yet until a `waitEvent` on that name consumes it.

Where this goes beyond Temporal, and why each matters to a customer:

- **A timeout never repeats an effect blindly.** Temporal retries an activity whose attempt timed out, assuming it is idempotent. Here
  a deadline that ends a live attempt leaves its outcome unknown, so the step is reconciled (`reconcileWith`) or reviewed, as
  feature 037 decided for every lost attempt (constitution principle III). A deadline that ends no live attempt (a task nobody
  claimed, a retry that would start too late) is a known failure and takes the node's failure route.
- **One clock decides.** Every deadline is written from the database clock by the transition that starts it, and a claim refuses an
  attempt whose start deadline has passed, so a lease and a timeout of the same attempt can never both commit.
- **A retry policy per failure code**, not only a list of codes that are never retried: `RATE_LIMITED` can get ten attempts
  spaced a minute apart while `ACTIVITY_ERROR` keeps three. A worker can also say when the next attempt should start
  (`nextRetryDelay`, for a provider's `Retry-After`), as Temporal's `nextRetryDelay` does.
- **Two kinds of heartbeat.** `host` detects a lost process without one line of handler code; `handler` detects a stuck handler,
  which is Temporal's meaning.
- **The journal says which deadline ended an attempt** (`deadline: startToClose` or `scheduleToClose`), and the Studio shows the
  deadlines of a step before they pass.
- **Buffered signals are declared, bounded and moved by the platform across continue-as-new**, instead of being lost if user code
  does not drain them.

The rules are [ADR-0110](../adr/ADR-0110-step-timeouts-are-deadlines-on-the-database-clock-and-retry-is-decided-per-failure-code.md)
(timeouts and retry) and [ADR-0111](../adr/ADR-0111-a-version-may-declare-buffered-signals-kept-in-a-bounded-per-run-inbox.md)
(buffered signals), both proposed with this design. The register of decisions, gaps and questions is
`specs/045-activity-timeouts/research.md` (Decision 045-001 onward). The principles cited by numeral are those of
[the constitution](../constitution.md).

### Terms

- **Attempt.** One lease of a step and what follows it, numbered from 1 (`steps.attempt`). A lease withdrawn because it never
  reached or never started on a host (Architecture decision 0098, Architecture decision 0099) does not spend an attempt.
- **Release.** The transition that first makes a step due to run: `step.released` (`RunEvent::StepReleased`), or, for a node with
  a concurrency or rate gate of feature 022, the moment it starts waiting at the gate.
- **Claimable.** A step is claimable when it is `READY`: released and past any gate (Architecture decision 0089), or a retry
  whose backoff has passed (`step.retry_due`).
- **Live attempt.** An attempt whose lease is committed: `LEASED` or `RUNNING`.
- **The database clock.** The transition's time (`tx.now()`, the database time) when a deadline is written, and the database's
  `now()` when it is compared. No process clock decides a deadline.
- **Start deadline** (`steps.start_deadline_at`): claimable time plus `scheduleToStartMs`. **Close deadline**
  (`steps.close_deadline_at`): release time plus `scheduleToCloseMs`. **Attempt deadline** (`steps.attempt_deadline_at`, exists):
  the earlier of the claim's time plus `timeoutMs` and the close deadline. **Lease expiry** (`steps.lease_expires_at`, exists): the
  claim's or the last renewal's time plus the node's `heartbeatTimeoutMs` (or the platform's 30 s), never after the attempt deadline.
- **Outcome-unknown codes.** The codes for which `FailureCode::leaves_outcome_unknown` holds (`crates/domain/src/execution/event.rs`):
  today `LEASE_EXPIRED`, `ATTEMPT_TIMEOUT`, `OUTCOME_UNKNOWN`, `STORAGE_QUOTA_EXCEEDED`, `RECONCILIATION_FAILED`,
  `RECONCILIATION_UNKNOWN`, `RECONCILIATION_NOT_FINAL`; this design adds `HEARTBEAT_TIMEOUT`, making eight.

## The Problem

A step that nobody claims waits until its run's deadline: there is no way to say "if no worker takes this within five minutes, go to
the fallback". A step whose retries back off exponentially can outlive the business window it serves, because there is no budget
for the whole step. The lease time-to-live is one value for the platform (`lease_ttl`, 30 s, `crates/application/src/worker.rs`),
so a slow external call that blocks a host's event loop for 40 s loses its lease, and a worker process that dies holds a
fast step for 30 s however short the step is. A handler stuck in a deadlock keeps its lease forever, because the host renews it
while the handler runs. And the retry policy (`RetryPolicy` in `crates/domain/src/workflow.rs`) treats every failure alike: an
author cannot say "a declined card is final, a rate limit is retried ten times".

The roadmap assessment of 2026-11-11 (`docs/roadmap-assessment-2026-11-11.md`, Temporal-parity table) names these gaps
("Schedule-to-start and schedule-to-close timeouts, per-activity heartbeat timeout, non-retryable error types: no spec found")
and one more: "Buffered signals (an early signal is refused today)". Today a signal that no step waits for is refused with `409`
"the run is not waiting for that signal" (`crates/application/src/runs.rs`), so a customer who approves an order before the
workflow reaches its approval step gets an error and must retry until the run is ready.

### Where buffered signals belong

Feature 018 does not cover them: its spec lists "buffered signals" under Out of scope and states "No signal is ever buffered"
(FR-018-024), on Architecture decision 0050 decision 4 and Architecture decision 0030 decision 17. Design 044 (workflow as code)
needs them, because a workflow function reads signals from a channel when it chooses. They were weighed in three places: in
design 044 (they would wait for its determinism study, and the declarative mode, which needs them today, would wait too); in a
design of their own (the cleanest, but another number while the 0.3.0 designs are being numbered by other sessions); and here.
They are here (Decision 045-009, delegate (Owner decision 2026-11-24-4)): they share this design's race with a wait's timeout
(AGENTS.md rule 9), its fleet-requirement switch and its conformance scenarios, and design 044 can build on them. Their tasks
form a slice of their own and could be moved out without touching the others.

## Requirements

### Functional

- **Requirement 045-001 — Timeout fields.** An `activity` or `script` node MAY declare `scheduleToStartMs` and
  `scheduleToCloseMs` (each an integer from 1,000 to 31,622,400,000, that is one second to 366 days), `heartbeatTimeoutMs` (3,000
  to 86,400,000, three seconds to 24 hours) and `heartbeatFrom` (`host`, the default, or `handler`). Publication MUST refuse, with
  the JSON pointer of the field: a value out of range; `heartbeatFrom: "handler"` without `heartbeatTimeoutMs`;
  `heartbeatTimeoutMs` greater than `timeoutMs` or than `scheduleToCloseMs`; `timeoutMs` greater than `scheduleToCloseMs`;
  `scheduleToStartMs` greater than `scheduleToCloseMs`. A compensation, a reconciliation or any other kind of node MUST NOT declare
  them. *Acceptance:* a publication test per refusal and per accepted combination (`integration_gitops_versioning`), the IR schema
  test, and the Studio's form test.
- **Requirement 045-002 — Retry fields.** A `retry` (and a `compensationRetry`) MAY declare `nonRetryableErrorCodes` (1 to 32
  distinct codes) and `byErrorCode` (1 to 16 entries, each with `codes`, 1 to 32 distinct codes, and any of `maxAttempts`,
  `initialDelayMs`, `maxDelayMs`, `strategy`, `jitter` with the meaning and bounds they have in `retry`). A code matches
  `^[A-Z0-9_]{1,64}$`, appears at most once across both lists, the lists name at most 64 codes in all, and none is one the platform
  decides elsewhere: the eight outcome-unknown codes, `CANCELLED`, `TASK_NOT_STARTED`, `SCHEDULE_TO_START_TIMEOUT` and
  `SCHEDULE_TO_CLOSE_TIMEOUT`. *Acceptance:* a publication test per refusal (`RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`,
  `RETRY_CODES_TOO_MANY`).
- **Requirement 045-003 — Schedule-to-start.** When an attempt of a node with `scheduleToStartMs` becomes claimable, its start
  deadline MUST be written in the same statement. A claim MUST NOT lease an attempt whose start deadline is at or before the
  database's `now()`. Once it has passed, the step MUST end `FAILED` with `SCHEDULE_TO_START_TIMEOUT`, which is not retried (no lease
  was committed for the attempt, so nothing ran) and takes the node's failure route. A lease withdrawal MUST NOT move the start
  deadline. *Acceptance:* the barrier test of a claim against the deadline, in both orders, and the withdrawal test (Technical Plan 7).
- **Requirement 045-004 — Schedule-to-close.** The close deadline MUST be written at the step's release and kept by every later
  attempt. Reached while no attempt is live, it MUST end the step `FAILED` with `SCHEDULE_TO_CLOSE_TIMEOUT`, a known failure. An
  attempt's deadline MUST be the earlier of its claim time plus `timeoutMs` and the close deadline; when the close deadline ends a
  live attempt, the attempt ends with `ATTEMPT_TIMEOUT` and `deadline: scheduleToClose`, and the outcome-unknown rule applies.
  *Acceptance:* the tests of Technical Plan 4, one per status in which the deadline can be reached.
- **Requirement 045-005 — Heartbeat timeout.** A lease of a node with `heartbeatTimeoutMs` MUST be written and renewed with that
  time-to-live instead of the platform's, never past the attempt deadline. An attempt that lapses MUST end with `HEARTBEAT_TIMEOUT`,
  an outcome-unknown code; a node without the field keeps `LEASE_EXPIRED`. *Acceptance:* the lapse test per node kind and the parity
  test of the outcome-unknown codes across the domain, the chain-progress SQL and the Studio.
- **Requirement 045-006 — The handler as the heartbeat's source.** A node with `heartbeatFrom: "handler"` MUST be leased only to a
  session accepted at protocol 1.9 or later. Such a host MUST renew the lease only when the handler called `heartbeat()` since its
  last renewal (it sends the first beat itself when the handler starts, which journals `step.started`). *Acceptance:* the claim
  test with a 1.8 and a 1.9 session, and the conformance scenario `heartbeat-from-handler` in six languages.
- **Requirement 045-007 — The retry decision.** When an attempt ends, the step MUST be decided in this order: an outcome-unknown
  code goes to reconciliation or review (feature 037, unchanged); a step asked to stop is cancelled (unchanged); `retryable = false`
  from the worker is final; a code in `nonRetryableErrorCodes` is final; otherwise the policy is the base policy overlaid by the
  entry of `byErrorCode` that names the code, if any, and another attempt is scheduled only when the attempt count is below that
  policy's `maxAttempts` and the next attempt's time is before the close deadline; when the close deadline would be passed, the
  step fails with `SCHEDULE_TO_CLOSE_TIMEOUT` and the event records the last attempt's code. *Acceptance:* the decision-table unit
  test (every row of Technical Plan 6) and the conformance scenario `retry-by-code`.
- **Requirement 045-008 — A worker's delay.** `FailStep.next_retry_delay_ms` (1 to 86,400,000) MUST replace the computed delay of
  the next attempt, without jitter, when the decision of Requirement 045-007 schedules one; it MUST NOT add an attempt or pass the
  close deadline. *Acceptance:* unit test and conformance scenario `retry-by-code`.
- **Requirement 045-009 — Deadlines are acted on by a pass on the database clock.** The scheduler MUST end steps whose start or
  close deadline has passed, on its shard, re-checking the deadline, the status and the attempt's generation under the run lock in
  the transaction that ends it. A pass that crashes, or two passes, MUST end a step at most once. *Acceptance:* the crash and
  duplicate tests of `fault_activity_timeouts`.
- **Requirement 045-010 — Races.** Each race of Technical Plan 7 MUST resolve as its row says, and the loser MUST be a no-op that
  leaves evidence (an audited refusal for a caller, a counted no-op for a pass). *Acceptance:* one barrier test per row, with held
  locks, never with sleeps (AGENTS.md rule 18).
- **Requirement 045-011 — Gates.** A step waiting at a gate of feature 022 MUST be ended by its close deadline through the gate's
  own exit (its waiter row removed in the same transaction), and the grant routine MUST NOT make claimable a step whose close
  deadline has passed. *Acceptance:* the gate tests of Technical Plan 4, built with whichever of 022's gate and this design lands
  second.
- **Requirement 045-012 — Compensation and reconciliation keep their rules.** The new timeouts MUST NOT apply to a compensation or
  a reconciliation node; `compensationRetry` gains only the retry fields. *Acceptance:* a test that a saga's compensation is
  unaffected by a forward node's `scheduleToCloseMs`.
- **Requirement 045-013 — Journal.** The fields this design adds to events are optional; an event written before it decodes as
  before, and a run's replay MUST equal its projection with and without them. *Acceptance:* the replay-equals-projection corpus of
  `integration_activity_timeouts`.
- **Requirement 045-014 — Visible deadlines.** The REST view of a step and the Studio MUST show its start, close and attempt
  deadlines and the cause of the attempt deadline, and a timeout's code with its meaning. *Acceptance:* `contract_rest_durable_execution`
  and the Studio's run-view tests.
- **Requirement 045-015 — SDK builders.** The six SDKs' workflow builders MUST offer the fields of Requirements 045-001, 045-002 and
  045-018, validated as the server validates them, and the SDKs MUST let a worker map exception classes to failure codes at
  registration (so an author can name them in a retry list). *Acceptance:* each SDK's unit tests and the IR conformance scenarios.
- **Requirement 045-016 — SDK hosts at 1.9.** A host accepted at 1.9 MUST beat at a third of `heartbeat_timeout_ms` (within today's
  bounds of 500 ms and 10 s), honour `heartbeatFrom: "handler"`, expose the attempt's deadline from `attempt_time_left_ms` on its
  monotonic clock, and send `next_retry_delay_ms` when the handler asks. *Acceptance:* each SDK's unit tests and the conformance
  scenarios `activity-timeouts` and `heartbeat-from-handler`.
- **Requirement 045-017 — Fleet switch and runs in flight.** A version that uses any field of this design MUST publish only once the
  operator recorded the requirement `activity-timeouts-v1` (or `signal-inbox-v1` for buffered signals); a version published before
  MUST behave for every run exactly as today. *Acceptance:* the registry tests and the released 0.2 scenario of Technical Plan 13.
- **Requirement 045-018 — Buffered signal declaration.** A version MAY declare `signals` (1 to 64 entries) of `{name, buffered,
  maxPending}`, `maxPending` from 1 to 1,000 (default 100); a buffered name MUST be the event type of a `waitEvent` node of the
  version. *Acceptance:* publication tests.
- **Requirement 045-019 — Buffering and consumption.** A signal of a buffered name that no step waits for MUST be kept in the run's
  inbox in the run's transaction, journaled `signal.buffered`, audited as `run.signalled` and answered `202` with `buffered: true`.
  A `waitEvent` step MUST consume, in the transaction that makes it wait, the oldest pending signal of its event type, once. When
  steps wait, delivery MUST be as today. *Acceptance:* unit and integration tests, and the conformance scenario `buffered-signals`.
- **Requirement 045-020 — Inbox bounds.** At most `maxPending` pending signals per name and 1,000 per run; each payload at most
  256 KiB, stored as run content and counted in the run's retained bytes and the tenant's content quota. Over a bound the signal
  MUST be refused (`409 SIGNAL_INBOX_FULL`, or `429 QUOTA_EXCEEDED` as today for retained bytes) and nothing stored.
  *Acceptance:* tests at each bound and one past it.
- **Requirement 045-021 — Inbox lifecycle.** Continue-as-new MUST move pending signals to the successor in the continuation's
  transaction when the successor's version declares the name buffered, and mark the rest discarded; a terminal transition MUST mark
  every pending signal discarded and journal how many; reset reapply (FR-018-039) MUST treat a signal buffered after the reset point
  as a recorded signal. *Acceptance:* the tests of Technical Plan 12.
- **Requirement 045-022 — With-start and chains.** Signal-with-Start (FR-018-024) MUST buffer a declared name instead of rolling back
  with `SIGNAL_NOT_ACCEPTED_AT_START`, and a holder that does not wait MUST buffer it as `signalRun` does; a chain's signal reaches
  its head under the chain lock, as today. *Acceptance:* `integration_schedules_bulk_operations` cases and the slot-order test.
- **Requirement 045-023 — Signals are visible.** The run view, the REST API (`GET /v1/runs/{runId}/signals`) and the six SDK clients
  MUST list pending, consumed, moved and discarded signals by name, sequence, time and size, never the payload, which is disclosed
  only through feature 016's payload disclosure under its permission. *Acceptance:* contract and Studio tests.
- **Requirement 045-024 — Telemetry and audit.** The metrics, spans and audit records of Technical Plan 11, with closed labels and no
  tenant, namespace or run identifier as a label. *Acceptance:* the metrics text test of `integration_activity_timeouts`.

### Non-functional

- **Non-functional requirement 045-001 — No cost on the hot path.** A lease stays 38 statements in 2 transactions and a completion 50
  statements in 1 (the lines of `tests/integration/claim-transition-efficiency/work_budget.rs`); the new columns are written by
  the statements that write the step today, and the claim's new conditions are on the row it already reads.
- **Non-functional requirement 045-002 — Lag.** A deadline is acted on by the next pass of its shard owner. Criterion, measured on the
  CI machine and reported with its environment (constitution, Delivery Constraints): with 1,000 start deadlines passing in the same
  second, every step ended within 5 s of its deadline, the 99th percentile under 2 s.
- **Non-functional requirement 045-003 — Isolation.** No new grant beyond the new table's rows for the runtime role; the inbox is
  read and written only under the run's tenant; a payload is classified as a step output is (feature 016).
- **Non-functional requirement 045-004 — Compatibility.** Protocol 1.9 is additive; a host at 1.0 to 1.8 is served as today; the
  IR grows by optional fields only; the journal's new fields are optional.

### Success criteria

- **Success criterion 045-001.** A step with `scheduleToStartMs: 5000` and no worker registered ends `SCHEDULE_TO_START_TIMEOUT` and
  takes its `onError` route, and a worker that registers after the deadline is never leased it.
- **Success criterion 045-002.** A step with `scheduleToCloseMs: 60000` whose attempts fail with a retryable code never starts an
  attempt after its 60th second and ends `SCHEDULE_TO_CLOSE_TIMEOUT` with the last code recorded.
- **Success criterion 045-003.** A handler that stops calling `heartbeat()` under `heartbeatFrom: "handler"` loses its lease within
  `heartbeatTimeoutMs` plus one pass, and its step waits for reconciliation or review, in six languages.
- **Success criterion 045-004.** A failure whose code is in `nonRetryableErrorCodes` is never retried; a code with a `byErrorCode`
  entry gets exactly that entry's attempts.
- **Success criterion 045-005.** An approval signalled before the run reaches its `waitEvent` completes that wait when it is entered,
  once, and a second wait on the same name does not see it.
- **Success criterion 045-006.** Non-functional requirements 045-001 and 045-002 measured and reported.

### User stories

- **User story 045-01.** As a workflow author, I bound how long a step may wait for a worker and how long it may take in all, and
  route to a fallback when it cannot (CUJs 1 and 2).
- **User story 045-02.** As a workflow author, I say which failures are final and how each other failure is retried (CUJ 3).
- **User story 045-03.** As an operator, I see why an attempt ended and what the platform does next (CUJ 4).
- **User story 045-04.** As a caller, I can send a signal before the run waits for it (CUJ 5).

## Goals

- The four Temporal activity timeouts, with one meaning each and one clock.
- A retry decision per failure code that a reader of the version can audit.
- No automatic repetition of an effect whose outcome is unknown, whatever timeout ended it.
- Buffered signals, opt-in, bounded, with a lifecycle across continuation, end and reset.
- No new statement on the claim or the completion.

## Non-Goals

- Automatic retry after a deadline that ends a live attempt. Constitution principle III forbids it; `reconcileWith` is the way to
  make such a step go on by itself, and a qualified safe-repeat (Architecture decision 0076) is not in this design.
- Timeouts on compensation, reconciliation, managed database, lake, child, wait or approval nodes. Each has its own deadline today
  (`timeoutMs`, the wait's timeout, the child's run deadline); the managed runner nodes are Gap 045-003.
- Patterns or prefixes in the retry lists (Alternatives Considered).
- An expiry per buffered signal (ADR-0111, option 4).
- Workflow as code: design 044 decides how its activity calls carry these options and how its signal channel reads the inbox; this
  design gives it the server side.
- Heartbeat details: feature 019 (Technical Plan 5 says how they fit).

## Open Questions

- Whether the deadline pass needs its own work budget per tick or shares the lost-lease pass's (Gap 045-001); the measurement of
  Task 045-19 decides.
- Whether a step's start deadline should also be offered to 019's priority order as an urgency hint (Gap 045-002); not in 0.3.0.

## Owner decisions needed

The owner's decisions this design rests on: Owner decision 2026-11-24-1 (the scope), Owner decision 2026-11-20-6 and Owner decision
2026-11-24-4 (the delegation of its questions, of its ADRs and of its approval), Owner decision 2026-11-15-8 (the register of
protocol minors). Every question below was decided under that delegation and is recorded with its options and reasoning in
`specs/045-activity-timeouts/research.md`:

- Owner question 045-001: does a timeout that ends a live attempt retry, as in Temporal? No: outcome unknown, reconciliation or
  review (Decision 045-001, delegate (Owner decision 2026-11-24-4)); the constitution leaves no other choice, and the design says so
  to Temporal users.
- Owner question 045-002: is `SCHEDULE_TO_START_TIMEOUT` retried? No: final, failure route (Decision 045-002).
- Owner question 045-003: a new error-type field, or the existing `error_code`? `error_code` (Decision 045-003).
- Owner question 045-004: heartbeat from the host, from the handler, or both? Both, `host` by default (Decision 045-004).
- Owner question 045-005: a retry policy per code, or only a non-retryable list? Both (Decision 045-005).
- Owner question 045-006: which node kinds? `activity` and `script` (Decision 045-006).
- Owner question 045-007: pending signals at continue-as-new? Moved when the successor declares them, else discarded and journaled
  (Decision 045-007).
- Owner question 045-008: buffer every signal, or opt-in? Opt-in per version and name (Decision 045-008).
- Owner question 045-009: where do buffered signals belong? Here (Decision 045-009).

For the coordinating session to confirm, because it supersedes in part an architecture decision that the owner accepted on
2026-10-29: ADR-0111 supersedes Architecture decision 0050 decision 4 and Architecture decision 0030 decision 17 for versions that
declare a buffered signal. The design relies on Owner decision 2026-11-24-1 (every missing parity feature in 0.3.0, read with the
assessment that names buffered signals) as the owner's later word.

## Critical User Journeys (CUJs)

### CUJ 1: No worker takes the task

*   **Given**: a `chargeCard` activity with `scheduleToStartMs: 30000` and `onError: notifyOps`, and every payments worker down.
*   **When**: the step becomes `READY` and 30 s pass on the database clock.
*   **Then**: the next pass ends it `SCHEDULE_TO_START_TIMEOUT`, the run goes to `notifyOps`; a worker that comes back at second 31
    is not leased the attempt; the Studio showed the start deadline while it waited.

### CUJ 2: Retries that outlive their window

*   **Given**: `reserveSeat` with `scheduleToCloseMs: 120000`, `retry: {maxAttempts: 10, initialDelayMs: 5000}` and attempts failing
    with `UPSTREAM_BUSY`.
*   **When**: the fourth failure would schedule its retry 40 s later, at second 135.
*   **Then**: no retry is scheduled; the step fails `SCHEDULE_TO_CLOSE_TIMEOUT` with `lastAttemptCode: UPSTREAM_BUSY`, and a saga
    around it compensates what completed.

### CUJ 3: A declined card is final, a rate limit is patient

*   **Given**: `retry: {maxAttempts: 3, nonRetryableErrorCodes: ["CARD_DECLINED"], byErrorCode: [{codes: ["RATE_LIMITED"],
    maxAttempts: 10, initialDelayMs: 60000, strategy: "fixed"}]}`.
*   **When**: an attempt fails `RATE_LIMITED` with `nextRetryDelay` of 90 s, later another fails `CARD_DECLINED`.
*   **Then**: the first is retried after 90 s (the worker's delay), the second ends the step at once; the journal records which rule
    decided each.

### CUJ 4: A stuck handler

*   **Given**: `exportReport` with `heartbeatTimeoutMs: 20000, heartbeatFrom: "handler"`, `reconcileWith: findExport`, on a 1.9 host.
*   **When**: the handler deadlocks after its last `heartbeat()`.
*   **Then**: the host stops renewing; 20 s later the lease lapses `HEARTBEAT_TIMEOUT`; `findExport` asks the provider what happened;
    the operator sees "outcome unknown: reconciling" and the deadline that passed.

### CUJ 5: An approval sent early

*   **Given**: a version with `signals: [{name: "approve", buffered: true}]` and a `waitEvent` on `approve` after a long `checkStock`.
*   **When**: the approver signals `approve` while `checkStock` runs.
*   **Then**: the call answers `202` with `buffered: true`; when the run reaches the wait, the wait is satisfied in the same transaction
    with the approver's payload; the run view lists the signal as consumed by that step.

## Technical Plan (Architecture)

### 1. The IR

Additive to `specs/001-platform-foundation/contracts/workflow.schema.json` (Architecture decision 0032), on `activity` and `script`:

```json
"scheduleToStartMs": {"type": "integer", "minimum": 1000, "maximum": 31622400000},
"scheduleToCloseMs": {"type": "integer", "minimum": 1000, "maximum": 31622400000},
"heartbeatTimeoutMs": {"type": "integer", "minimum": 3000, "maximum": 86400000},
"heartbeatFrom": {"enum": ["host", "handler"]}
```

On `$defs/retry` (and so on `compensationRetry`), which keeps `additionalProperties: false`:

```json
"nonRetryableErrorCodes": {"type": "array", "minItems": 1, "maxItems": 32, "uniqueItems": true,
  "items": {"type": "string", "pattern": "^[A-Z0-9_]{1,64}$"}},
"byErrorCode": {"type": "array", "minItems": 1, "maxItems": 16, "items": {
  "type": "object", "additionalProperties": false, "required": ["codes"],
  "properties": {"codes": {"type": "array", "minItems": 1, "maxItems": 32, "uniqueItems": true,
                           "items": {"type": "string", "pattern": "^[A-Z0-9_]{1,64}$"}},
                 "maxAttempts": {"type": "integer", "minimum": 1, "maximum": 100},
                 "initialDelayMs": {"type": "integer", "minimum": 1},
                 "maxDelayMs": {"type": "integer", "minimum": 1},
                 "strategy": {"enum": ["fixed", "exponential"]}, "jitter": {"type": "boolean"}}}}
```

On the workflow document: `"signals": {"type": "array", "minItems": 1, "maxItems": 64, "items": {"type": "object",
"additionalProperties": false, "required": ["name", "buffered"], "properties": {"name": {"type": "string", "minLength": 1,
"maxLength": 128}, "buffered": {"type": "boolean"}, "maxPending": {"type": "integer", "minimum": 1, "maximum": 1000}}}}`.

The schema cannot express the cross-field rules of Requirements 045-001, 045-002 and 045-018 (a timeout greater than another, a
code in both lists, a reserved code, a buffered name that no wait consumes); `crates/infra-ir/src/convert.rs` checks them and
refuses with the field's pointer and a closed reason (`TIMEOUT_OUT_OF_RANGE`, `TIMEOUTS_CONTRADICT`, `HEARTBEAT_SOURCE_NEEDS_TIMEOUT`,
`TIMEOUT_NOT_ALLOWED_ON_NODE`, `RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`, `RETRY_CODES_TOO_MANY`, `BUFFERED_SIGNAL_NOT_AWAITED`),
listed in `contracts/errors.md` of feature 001 by Task 045-1. Two examples that pass and their reasons are in the conformance
fixtures.

In the domain (`crates/domain/src/workflow.rs`), the node gains `StepTimeouts {schedule_to_start, schedule_to_close, heartbeat:
Option<Heartbeat {timeout, from}>}` and `RetryPolicy` gains `final_codes: BTreeSet<FailureCode>` and `by_code: Vec<(BTreeSet<FailureCode>,
RetryOverride)>`, with one function `RetryPolicy::for_code(&FailureCode) -> CodeRule` (`Final`, or the effective policy). No
`serde_json` enters the domain (principle II); `infra-ir` builds the types.

### 2. The deadlines and their columns

One migration in `crates/infra-postgres/migrations/` (the `/migration` checklist), additive:

| Column of `steps` | Type | Written by | Cleared by |
|---|---|---|---|
| `start_deadline_at` | `timestamptz NULL` | the statement that makes the attempt `READY` (release past its gates, retry due, gate grant) | the claim, any end of the step |
| `close_deadline_at` | `timestamptz NULL` | the statement that releases the step (or starts its gate wait) | the end of the step |
| `attempt_deadline_cause` | `text NULL`, `CHECK` in (`START_TO_CLOSE`, `SCHEDULE_TO_CLOSE`) | the claim, with `attempt_deadline_at` | with `attempt_deadline_at` |
| `min_protocol_minor` | `smallint NOT NULL DEFAULT 0` | the release (9 for `heartbeatFrom: "handler"`) | never; reused by draft 031 for its payload plan |

Each is computed in SQL from the transition's time (`$14` of the step statement, the database clock) and the node's durations,
bound as microseconds, as `attempt_deadline_at` is today (`crates/infra-postgres/src/execution.rs`, the three statements that
write a step: the root statement, the scoped statement of a full-run transaction, the path statement `upsert_path_instance`). The
claim writes `attempt_deadline_at = LEAST(claim time + timeoutMs, close_deadline_at)` and the cause of the earlier one; ties go to
`START_TO_CLOSE`. `CHECK` constraints are added `NOT VALID` and validated in a second migration, as design 043 did.

Two partial indexes, shaped like the lost-lease indexes of `20261017000000_scheduler_shards.up.sql`:
`steps_start_deadline_shard_idx ON steps (run_shard(run_id), start_deadline_at) WHERE status = 'READY' AND start_deadline_at IS NOT
NULL`, and `steps_close_deadline_shard_idx ON steps (run_shard(run_id), close_deadline_at) WHERE status IN ('READY', 'RETRY_WAIT',
'WAITING') AND close_deadline_at IS NOT NULL` (a `WAITING` row carries a close deadline only at a gate). Building them reads the table
once at the upgrade; the upgrade notes say so (Risks).

In the domain, `StepState` gains `close_deadline: Option<OffsetDateTime>`, set from `StepReleased` and kept across retries, because
the retry decision needs it (Technical Plan 6). `StepReleased` and `StepRetryDue` gain optional `start_deadline` and
`close_deadline` fields, which the reducer applies and the store writes; replay restores them; an event written before this design
has none.

### 3. The claim and the start deadline

The claim's condition (the root claim and the path claim; `crates/infra-postgres/src/execution.rs` and `execution/path.rs`) adds,
on the row it already selects: `(s.start_deadline_at IS NULL OR s.start_deadline_at > now())`, `(s.close_deadline_at IS NULL OR
s.close_deadline_at > now())` and `s.min_protocol_minor <= $session_minor`. No statement is added (Non-functional requirement
045-001). A step skipped by the first two conditions is ended by the pass.

The pass (`crates/scheduler/src/passes.rs`, beside the lost-lease pass) reads, per shard and window, the rows of
`steps_start_deadline_shard_idx` whose deadline is at or before `now()`, and for each opens the run's transaction (chain lock, then
run lock, as every run transaction does) with a condition `StartDeadlinePassed {instance, attempt, generation}` checked by `holds`:
the row is still `READY`, its generation is the one read, its start deadline is still at or before `tx.now()`. If it holds, the
domain command `time_out_start(def, state, instance, now)` emits `StepFailed {code: SCHEDULE_TO_START_TIMEOUT, fence: None}` and
settles (the failure route, a saga's compensation); if not, the pass counts a no-op. A run with a stop reason is left to the stop
path, which cancels the step.

A lease withdrawal (Architecture decision 0098 decision 3, Architecture decision 0099 decision 5) sets the step back to `READY`
without spending an attempt. Its statement MUST NOT write `start_deadline_at`: the withdrawn lease never started, so the attempt is
the same attempt and keeps its deadline. If that deadline has passed, the pass ends the step: nothing ran, so the failure is known.

### 4. The close deadline

Written at the release. While no attempt is live, the pass reads `steps_close_deadline_shard_idx` and ends the step under
`CloseDeadlinePassed {instance, generation}` with `time_out_close`, which emits `StepFailed {code: SCHEDULE_TO_CLOSE_TIMEOUT, fence:
None}`:

| Status when it passes | What happens |
|---|---|
| `READY` | as above; the claim already refuses it |
| `RETRY_WAIT` | as above; the retry is never due |
| `WAITING` at a gate of feature 022 | as above, through the gate's exit: the waiter row of `concurrency_waiters` or the rate-limit queue entry is removed in the same transaction, the way the gate's own wait timeout removes it (`specs/022-flow-control-step-policies/data-model.md`) |
| `LEASED` or `RUNNING` | nothing here: the attempt deadline is the close deadline, and the lost-lease pass ends the attempt `ATTEMPT_TIMEOUT` with `deadline: scheduleToClose` |

The grant routine of Architecture decision 0089 adds the condition `close_deadline_at > now()` to the step it would make `READY`;
a step whose deadline passed is left to the pass. Whichever of feature 022's gates and this design is built second adds the gate
rows of this table and their tests (Task 045-11).

The ending events of a live attempt that a deadline ended (`StepFailed`, `ReconciliationRequested`, `StepNeedsReview` with
`ATTEMPT_TIMEOUT`) gain an optional `deadline` field (`startToClose`, `scheduleToClose`), read by the lost-lease pass from
`attempt_deadline_cause` and carried by `Expiry::AttemptTimeout {cause}`.

### 5. The heartbeat timeout and its source

The lease's time-to-live becomes per node: `LeaseGrant.ttl` and `LeaseRenewal.ttl` (`crates/application/src/worker.rs`, the claim
and `heartbeat`) take the node's `heartbeatTimeoutMs` when it is declared and `settings.lease_ttl` otherwise; the statement already
caps the expiry at the attempt deadline. The lost-lease pass maps an expiry of a node with the field to `Expiry::HeartbeatTimeout`,
whose code is `HEARTBEAT_TIMEOUT`. A renewal that commits before the pass's transaction saves the attempt, as today: the pass
re-checks the expiry under the run lock.

`HEARTBEAT_TIMEOUT` joins `FailureCode::outcome_unknown_codes` (seven codes become eight) and its two copies move with it: the SQL
literal of `crates/infra-postgres/src/chain_progress.rs` and `OUTCOME_UNKNOWN` of `frontend/apps/studio/src/features/runs/ops.ts`;
`crates/domain/tests/outcome_unknown_parity.rs` fails when they drift.

With delivery acknowledgement (Architecture decision 0099 decision 2 and decision 6) a lease is marked provisional only when its
earlier deadline is at least 18 s after the claim; a node with `heartbeatTimeoutMs` under 18,000 is therefore never provisional and
runs with today's meaning, which is correct and needs no rule here.

**The source.** `heartbeatFrom: "host"` keeps today's host loop: it beats while the handler runs. A host older than 1.9 computes its
interval as a third of the lease's time left at receipt, clamped to 500 ms and 10 s (`sdks/SPEC.md` §6.4), so it already beats in
time for any `heartbeatTimeoutMs` from 3,000 ms (a third is 1,000 ms, three beats per window). `heartbeatFrom: "handler"` needs a
host that knows it: the release writes `min_protocol_minor = 9` and the claim leases the step only to a session at 1.9 or later
(Technical Plan 3). Such a host sends the first beat itself when the handler starts, and afterwards sends a beat at its interval
only if the handler called `heartbeat()` since the last beat sent; calls between beats are coalesced. The handler can therefore
call it as often as it likes; at most one beat per interval reaches the server.

**Feature 019's heartbeat details.** Feature 019 gives `heartbeat(details)` an argument whose details ride on the next scheduled
beat (its checkpoints, `HeartbeatRequest` fields 9 and 10 at its own minor). It is the same call: a call with details is a handler
heartbeat, and the coalescing above keeps the latest details, as 019 says ("latest wins"). 019's detached heartbeat timeout is its
own; a detached attempt's maximum duration is also bounded by this design's close deadline.

### 6. The retry decision

`end_attempt` (`crates/domain/src/execution/command.rs`) keeps its first two rules (unknown outcome; a stop) and replaces the
third with this table, evaluated top to bottom; the first row that applies decides, and the event records which (`retryRule`):

| Row | When | Decision | Journal |
|---|---|---|---|
| 1 | the code leaves the outcome unknown | reconciliation or review (feature 037) | as today |
| 2 | the step was asked to stop | `step.cancelled` | as today |
| 3 | the worker said `retryable = false` | final | `step.failed`, `retryRule: worker` |
| 4 | the code is in `nonRetryableErrorCodes` | final | `step.failed`, `retryRule: nonRetryableCode` |
| 5 | the attempt count is at or above the effective `maxAttempts` (base overlaid by the code's `byErrorCode` entry) | final | `step.failed`, `retryRule: attemptsSpent` |
| 6 | the next attempt's time (the worker's `next_retry_delay_ms`, else the effective policy's delay) is at or after the close deadline | final, code `SCHEDULE_TO_CLOSE_TIMEOUT` | `step.failed`, `lastAttemptCode` |
| 7 | otherwise | retry | `step.retry_scheduled`, `retryRule: base` or `byErrorCode` |

The effective policy takes each field from the code's entry when the entry names it and from the base policy otherwise.
`maxAttempts` counts every attempt of the step whatever its code: an entry with `maxAttempts: 10` lets a step that has already had
four attempts (three of them with other codes) have six more if each fails with that code. A compensation's failure policy
(Architecture decision 0076) is applied after row 5 or 4 exactly as after a known non-retryable failure today. The unhandled
exception of an SDK is `ACTIVITY_ERROR`, which an author may name; Requirement 045-015 lets a worker map its own exception classes
to codes at registration (for example `CardDeclinedError` to `CARD_DECLINED`), which is how a Temporal user's
`nonRetryableErrorTypes` of class names is written here.

Row 6 is the only place the close deadline enters the domain; it compares two database-clock times (the failure's `now` and the
stored deadline), so a replay decides the same.

### 7. The races (AGENTS.md rule 9)

Every row is decided under the run lock, which every writer of a step takes (constitution principle I); the loser commits nothing
or a counted no-op.

| Race | Resolution | Test (barrier, never a sleep) |
|---|---|---|
| a claim and the start deadline | the claim's condition on `now()`: before the deadline the lease commits and the pass's `holds` finds `LEASED`; at or after, the claim skips it and the pass ends it | hold the run lock in the pass, claim at the deadline's either side |
| a claim and the close deadline | the same, with `close_deadline_at` | the same |
| a completion and the attempt deadline (either cause) | first commit wins, as today; a late report is `STALE_ATTEMPT` | existing test extended with `scheduleToClose` |
| a renewal and the heartbeat timeout | first commit wins, as today | the lost-lease test with a per-node time-to-live |
| a failure's retry and the close deadline | decided in the failure's transaction (row 6); a retry that is due is claimable only before the deadline | unit test at deadline minus one microsecond, at it, after it |
| a gate's grant and the close deadline | the grant's condition; the pass re-checks | held grant routine against the pass |
| a lease withdrawal and the start deadline | the withdrawal keeps the deadline; the pass ends the step if it passed | withdrawal at the deadline's either side |
| a stop and a deadline | the first commit; both end the step without an effect | stop held against the pass |
| a signal and a wait's entry | either order delivers: entered first, the signal finds it waiting; signalled first, the entry consumes it | held signal transaction against the entry |
| a buffered signal and a wait's timeout | the entry consumes at once, so the wait has no timer to race; a signal that arrives while the wait is open is today's race | existing test kept |
| two waits and one buffered signal | the first entry to commit consumes it; the second waits | two entries in one run, held |
| a signal and the run's end | before: buffered, then discarded and counted; after: refused as today | held terminal transition |
| a signal and a continuation | before the handoff: moved or discarded in the handoff; after: refused as today (a run's command never forwards; a chain's goes to the head) | held handoff |
| a crash of the pass, two passes | nothing committed is repeated: `holds` re-checks status, generation and deadline | kill between read and commit; two passes on one shard |

A lost broker hint does not delay a deadline: the pass reads the database, not the broker (constitution principle VII). A restart
loses nothing: every deadline is a column.

### 8. Protocol 1.9 and the field register

The register of `sdks/SPEC.md` §9: 1.7 is design 041, 1.8 is delivery acknowledgement (Owner decision 2026-11-21-6). This design
names 1.9, the next free minor, and takes it when its contract merges (Owner decision 2026-11-15-8). Minors are cumulative: a 1.9
host also carries 1.7 and the 1.8 host procedure, so the SDK work of 1.9 follows the SDK work of 1.8 (Technical Plan 14).

```protobuf
message TaskLease {
  // fields 1 to 18 unchanged; 19 to 22 named by draft 019; 23 and 24 by 1.8; 25 by draft 031
  // Added in 1.9. The node's heartbeatTimeoutMs, or 0 for the platform's lease time-to-live.
  uint32 heartbeat_timeout_ms = 26;
  // Added in 1.9. The time left before the attempt's deadline at the claim's clock sample, when it has one; never below 0.
  optional int64 attempt_time_left_ms = 27;
  // Added in 1.9. True when the node declares heartbeatFrom: "handler".
  bool heartbeat_from_handler = 28;
}
message FailStepRequest {
  // fields 1 to 12 unchanged; 13 named by draft 019
  // Added in 1.9. The handler's delay before the next attempt, 1 to 86,400,000; absent: the policy decides.
  optional uint32 next_retry_delay_ms = 14;
}
```

The field register across the drafts, so that no two take one number (recorded in drafts 019 and 031 the same day):

| Message | Numbers taken |
|---|---|
| `TaskLease` | 1 to 18 today; 19 to 22 draft 019; 23 and 24 protocol 1.8; 25 draft 031 (moved from 19); 26 to 28 this design |
| `WorkerCapabilities` | 1 to 5 today; 6 draft 019; 7 draft 031 (moved from 6); 10 protocol 1.8 |
| `RegisterWorkerResponse` | 1 to 3 today; 4 design 041; 5 draft 019 (moved from 4) |
| `HeartbeatRequest` | 1 to 8 today; 9 and 10 draft 019 |
| `HeartbeatResponse` | 1 and 2 today; 3 draft 019 |
| `FailStepRequest` | 1 to 12 today; 13 draft 019; 14 this design |

A server offers 1.9 on its build's proofs (the `protocol_ceiling` pattern of `crates/application/src/worker.rs`) and only when it
offers 1.8. A 1.9 field is sent only to a session accepted at 1.9; a session below it gets today's lease. The server accepts
`next_retry_delay_ms` only from a 1.9 session and refuses one out of range as `INVALID_ARGUMENT`.

### 9. The SDKs

In all six (TypeScript, Go, Python, C#, Java, Rust), with idiomatic names and one meaning (principle IV):

- **Builders.** `activity(...)` and `script(...)` options `scheduleToStart`, `scheduleToClose`, `heartbeatTimeout`, `heartbeatFrom`;
  retry options `nonRetryableErrorCodes`, `byErrorCode`; workflow `signals`. Each validates what the server validates (Requirement
  045-001, 045-002, 045-018) and raises the same reason before anything is sent.
- **Worker registration.** A map from exception class to failure code (TypeScript: a constructor; Go: an `errors.As` target;
  Python: a class; C#: a `Type`; Java: a `Class`; Rust: a function on the error), consulted before an unhandled error becomes
  `ACTIVITY_ERROR`. Local only: nothing on the wire changes.
- **Host at 1.9.** The interval is a third of `heartbeat_timeout_ms` when it is set, within 500 ms and 10 s; `heartbeat_from_handler`
  switches to the handler source of Technical Plan 5; `ctx.attemptDeadline` is the receipt time on the monotonic clock plus
  `attempt_time_left_ms` (the 1.8 acknowledgement's own `attempt_time_left_ms` replaces it for a flagged lease); `ActivityFailure`
  takes an optional `nextRetryDelay`, sent as `next_retry_delay_ms` (raised locally as `VALIDATION_FAILED` out of range).
- **Client.** `signal(...)` returns `buffered`; `listSignals(runId, {state})` reads `GET /v1/runs/{runId}/signals`.
- **Conformance** (`tests/sdk-conformance/`): `activity-timeouts` (the three new timeouts, at 1.8 and 1.9), `retry-by-code`,
  `heartbeat-from-handler` (1.9 only), `buffered-signals`; the same scenarios in six languages (AGENTS.md rule 8).

### 10. REST and the Studio

`specs/001-platform-foundation/contracts/openapi.yaml`, additive: the step view gains `startDeadlineAt`, `closeDeadlineAt`,
`attemptDeadlineAt`, `attemptDeadlineCause`; `SignalResult` gains `buffered` and `sequence`; `GET /v1/runs/{runId}/signals`
(permission `run:read`, paged, filter by state and name) returns `{sequence, name, state, receivedAt, settledAt, consumedBy,
bytes}`; the problem reasons of Technical Plan 1 and `SIGNAL_INBOX_FULL` are added. The description of `signalRun` changes from "a
signal that no step is waiting for is refused with 409, never buffered" to the rule for undeclared and declared names.

The Studio (`frontend/apps/studio/src/features/workflows/fields.tsx` and the run view in `features/runs/`): the four fields and the
retry lists in the inspector with the server's validation; on a step of a run, its deadlines with the time left (the projection's
lag shown, principle VI) and, for each timeout code, one sentence of what happened and what the platform does next ("No worker took
the task in time: the step failed and its failure route ran"; "The attempt lost its heartbeat: the outcome is unknown, so the
platform is asking `findExport`"); the run header counts pending signals and opens their list. Types follow `openapi.yaml`
(`npm run api && npm run check:api`).

### 11. Telemetry and audit

- Metrics, closed labels only: `step_timeouts_total{kind="schedule_to_start|schedule_to_close|start_to_close|heartbeat|lease"}`,
  `step_retry_decisions_total{rule="worker|non_retryable_code|attempts_spent|close_deadline|base|by_error_code"}`,
  `step_deadline_lag_seconds` (histogram: from the deadline to the pass's commit), `step_deadline_pass_noops_total{kind}`,
  `signal_inbox_total{outcome="buffered|consumed|moved|discarded|refused_full"}`.
- Spans: the pass's transaction carries `sagat.deadline.kind`; the claim's span carries how many rows its new conditions skipped.
- Audit: a buffered signal is audited as `run.signalled` with the reason `buffered`; publication of a version with these fields is
  audited as every publication. A timeout is a system transition, journaled and not audited, as a lease expiry is today.
- Logs: no payload, no signal payload, no code that the worker sent unless it passed the code pattern.

### 12. Buffered signals

One table, written only under the run's lock (the `/migration` checklist, storage class "run content"):

```sql
CREATE TABLE run_signal_inbox (
  tenant_id uuid NOT NULL, run_id uuid NOT NULL, sequence bigint NOT NULL,
  name text NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 128),
  payload_id uuid NOT NULL,              -- a payloads row, as a step output's is
  bytes integer NOT NULL CHECK (bytes BETWEEN 0 AND 262144),
  state text NOT NULL CHECK (state IN ('PENDING', 'CONSUMED', 'MOVED', 'DISCARDED')),
  received_at timestamptz NOT NULL, settled_at timestamptz NULL,
  consumed_by_node text NULL, consumed_by_path text NULL, moved_to_run uuid NULL,
  PRIMARY KEY (tenant_id, run_id, sequence)
);
CREATE INDEX run_signal_inbox_pending_idx ON run_signal_inbox (tenant_id, run_id, name, sequence) WHERE state = 'PENDING';
```

`runs` gains `pending_signals integer NOT NULL DEFAULT 0` (kept by the same transactions), so that a continuation and a terminal
transition with an empty inbox read nothing more. The sequence is the run's journal sequence of the `signal.buffered` event, so
the inbox's order is the journal's.

- **Signal** (`deliver_signal`, `crates/application/src/runs.rs`): unchanged while steps wait; with none, a declared name is
  buffered (Requirement 045-019) after the bounds of Requirement 045-020: the count per name and per run, then the retained bytes,
  whose bound (4 MiB, `MAX_RUN_OUTPUT_BYTES`) holds step outputs and pending payloads together, so at most 16 pending signals of
  256 KiB fit (4,096 KiB divided by 256 KiB) whatever `maxPending` says; undeclared names keep the `409`. The idempotency of
  `run.signal:{id}` is unchanged: a repeated call answers its first answer, `buffered` included.
- **Wait entry** (the planner's release of a `waitEvent`, `crates/domain/src/execution/plan.rs`, root and scoped): the domain asks
  for the oldest pending signal of the event type that the store loaded with the run (the pending rows, at most 1,000, are part of
  the run's state when `pending_signals > 0`); if one exists, the release emits `WaitSatisfied {node, signal: sequence}` instead of
  waiting, and the store marks the row consumed.
- **Continue-as-new** (Architecture decision 0030; feature 014's continuation): in the handoff's transaction, pending rows whose
  name the successor's pinned version declares buffered move to the successor (new sequences in its journal, `signal.buffered` with
  `movedFrom`), the rest are marked discarded; the predecessor journals `signal.discarded {count}` when any were.
- **Terminal transition:** every pending row is marked discarded and `signal.discarded {count}` journaled, in the terminal
  transaction. Payloads follow the run's retention and purge (feature 018's batch purge), as step outputs do.
- **Reset reapply** (FR-018-039): a `signal.buffered` after the reset point is a recorded signal `via: signal`; with reapply
  `SIGNALS`, it is delivered, or buffered when the new run's version declares it, in journal order; only a signal that can be
  neither ends reapply.
- **Signal-with-Start** (FR-018-024): for a declared name, the created run receives the signal (delivered or buffered) in its
  creation transaction; an open holder that does not wait buffers it. The binding-then-run lock order of Architecture decision 0050
  is unchanged (the inbox is written under the run lock the path already holds).
- **Visibility:** Requirement 045-023; the Studio shows the inbox; feature 020's list query may count `pending_signals`.

### 13. Upgrade, rollback and runs in flight

- A version published before this design has none of the fields: every run of it behaves as today, at every minor, through every
  rolling upgrade (principle IV).
- The operator records `activity-timeouts-v1` and `signal-inbox-v1` (`platform_requirement_activate`, Architecture decision 0082)
  once every engine and control plane runs a build with the capability; until then publication of a version using a field answers
  `422` with the reason `CAPABILITY_NOT_ACTIVE`. Once recorded, a binary without the capability serves nothing (the closed gate).
- Removing a requirement is refused while a published, non-retired version uses its fields.
- The migration adds columns and a table; its down migration refuses to run while a column holds a value or the table a row
  (the `/migration` checklist).
- Released 0.2 scenario (`scripts/test-release-upgrade-profile.py`): runs started on 0.2 finish on 0.3 unchanged; a 0.2 SDK host is
  served at its minor; a version with `heartbeatFrom: "handler"` is never leased to it.

### 14. Other features

| Feature | What this design asks of it, or gives it |
|---|---|
| 006 (1.8, Architecture decision 0099) | 1.9 follows 1.8 in the server and in each SDK; a withdrawal keeps the start deadline (Technical Plan 3) |
| 019 (worker deployments) | its `heartbeat(details)` is a handler heartbeat; its detached maximum duration is bounded by the close deadline; its priority order is unchanged by the start deadline (Gap 045-002); field numbers above |
| 020 (visibility) | the new failure codes, `attemptDeadlineCause` and `pending_signals` are additive facts it may list |
| 022 (flow control) | the close deadline ends a gated step through the gate's exit; the grant refuses a step past it |
| 030 (cross-namespace services) | its operation timeouts use the same names and clock; a cross-namespace signal uses the same inbox |
| 031 (client-side encryption) | `error_code` and `safe_message` are never encoded; `min_protocol_minor` is the column its payload-plan gating can use |
| 037 (compensation safety) | unchanged; `HEARTBEAT_TIMEOUT` is an outcome-unknown code |
| 043 (run input validation) | unchanged; a buffered signal is not a run input |
| 044 (workflow as code) | the options of an activity call map to these fields; its signal channel reads the inbox |

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A Temporal user expects a timed-out attempt to retry | The Studio's sentence, the guide's comparison table and `reconcileWith` as the documented way to go on automatically |
| Building two partial indexes on a large `steps` table holds a lock that blocks writes while it reads | The upgrade notes give the measured time on the scale fixture (Task 045-20); an operator with a very large table builds them `CONCURRENTLY` by hand before the upgrade, which the migration detects and skips |
| A short `heartbeatTimeoutMs` on a slow network loses leases | The minimum is 3,000 ms (three beats of 1,000 ms per window, against a ready-to-handler 99th percentile of 751 to 889 ms measured in `tests/performance/reports/sdk-streaming.md`) |
| The deadline pass falls behind under load | Its lag is a histogram with an alert in the runbook; it shares the shard's window and is measured (Gap 045-001) |
| A buffered signal is never consumed | Bounded by count and bytes, visible in the run view, discarded and counted at the run's end |
| Superseding Architecture decision 0050 decision 4 reopens its deadlock analysis | The inbox is written only under the run lock; no run-driven transition writes an aggregate; the slot-order test of feature 018 runs with a buffered Signal-with-Start |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Every deadline is a column written by the transition that starts it and acted on in one run transaction under the run lock; the claim's condition and the pass's re-check make a lease and a timeout exclusive; the retry decision is journaled with its rule; the inbox changes commit with the run's transitions |
| II. Modular domain, ports and adapters | Pass | New types in `crates/domain` without `serde_json`; parsing in `infra-ir`; SQL in `infra-postgres`; the pass in `scheduler`; no new crate edge (`scripts/check-architecture.py`) |
| III. External effects are at-least-once; sagas compensate | Pass | A deadline that ends a live attempt is an unknown outcome (reconciliation or review); only a deadline with no live attempt is a known failure; compensation and reconciliation nodes are untouched |
| IV. Compatibility is versioned and pinned | Pass with conditions (Complexity Tracking) | Optional IR fields, protocol 1.9 additive, optional journal fields, a fleet requirement; versions published before are unchanged; buffering is opt-in so `signalRun`'s `409` keeps its meaning for every undeclared name |
| V. Code, tenant boundary and secrets are isolated | Pass | No code runs on the server; inbox rows carry the tenant and are read under it; payloads classified as step outputs; no new grant beyond the table (Non-functional requirement 045-003) |
| VI. Observability and audit are part of the feature | Pass | Closed-label metrics, spans, `run.signalled` audits, deadlines and their cause in the projection with its lag |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Deadlines do not depend on a broker hint; a repeated signal answers its first answer through the idempotency record |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Barrier tests per race, crash and duplicate pass tests, restart, lost hint, conformance in six languages, contract tests; gaps registered |
| Delivery Constraints | Pass | No new technology; single region; one migration; the registry of Architecture decision 0082 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| A buffered signal changes what `signalRun` answers (principle IV) | The owner put buffered signals in 0.3.0 (Owner decision 2026-11-24-1); opt-in per version keeps every existing caller's answer | Buffer every signal: changes runs in flight; keep refusing: leaves the gap |
| Four columns and two indexes on `steps` | A deadline must be a durable fact the claim and a pass can read on the database clock | Timer rows per deadline: one more row and statement per attempt on the hot path |
| `min_protocol_minor` on `steps` | A handler-sourced heartbeat must never reach a host that cannot honour it | Refuse to publish until every host is at 1.9: the server cannot know the hosts |
| A new table and a counter on `runs` | A buffered payload must outlive its call and be bounded and charged | Hold pending signals in the run's state blob: unbounded rewrite on every transition |
| `byErrorCode` beside `nonRetryableErrorCodes` | A rate limit and a bug need different patience; the short list is the Temporal-familiar form | Only the list: forces an author to choose between giving up and hammering a provider |

## Alternatives Considered

- **Temporal's semantics (retry after every timeout).** Rejected by principle III (ADR-0110, option 2).
- **Timer rows for deadlines.** Rejected for the hot-path cost (ADR-0110, option 3).
- **A new error-type field.** Rejected: `error_code` is the type (ADR-0110, option 4).
- **Patterns in the retry lists.** Left out: exact codes keep the decision auditable; addable later without breaking a version.
- **Schedule-to-start counted from the release, gates included.** Rejected: a gate is a deliberate hold by the platform, and
  schedule-to-start exists to detect a missing worker; the close deadline covers the gate's time.
- **Heartbeat timeout as a second clock beside the lease.** Rejected: two expiries for one attempt would race each other; the
  lease expiry is the heartbeat timeout.
- **Buffered signals in design 044 or in a design of their own.** Weighed in The Problem; here for the reasons given there.
- **A pending signal satisfies every wait on its name, as a live signal does.** Rejected: a buffered signal would then be delivered
  to waits that did not exist when it was sent, an unbounded fan-out; one consumer per signal is a queue, which is what an early
  signal means.

## Non-Functional Requirements

Non-functional requirements 045-001 to 045-004 are stated under Requirements. Their tests: the budget lines of
`integration_claim_transition_efficiency` (unchanged) for 045-001; the lag measurement of Task 045-19 for 045-002; the privilege
audit of `integration_platform_foundation` and the suites as the runtime role for 045-003; the released 0.2 scenario and the
protocol baseline test for 045-004.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Test first: each task that changes behaviour adds its tests and sees them fail for the right reason before the code
(AGENTS.md rule 18). Documentation first: Task 045-1 changes the contracts and documents before any code (Owner decision
2026-11-20-8). The tasks are ordered into slices for two builders whose lanes never touch the same crate at once:

| Slice | Lane | Tasks | Crates and folders | Depends on |
|---|---|---|---|---|
| 0 | either | 045-1 | `specs/001-platform-foundation/contracts/`, `sdks/SPEC.md`, `docs/` | this design approved |
| 1 | server | 045-2, 045-3, 045-4 | `crates/domain` | slice 0 |
| 2 | server | 045-5, 045-6, 045-7 | `crates/infra-ir`, `crates/infra-postgres` | slice 1 |
| 3 | server | 045-8, 045-9, 045-10, 045-12 | `crates/scheduler`, `crates/application`, `crates/api-http`, `crates/api-grpc`, `apps/` | slice 2 |
| 4 | server | 045-13, 045-14 | `crates/domain`, `crates/infra-postgres`, `crates/application`, `crates/api-http` | slice 3 |
| 5 | server | 045-11, 045-18, 045-20 | `crates/application`, `crates/infra-postgres`, `tests/integration/activity-timeouts/`, `tests/fault/activity-timeouts/` | slice 4; 045-11 also on feature 022's gates |
| 6 | edges | 045-15 | `sdks/<language>` builders (six), `tests/sdk-conformance/` scenarios | slice 0; its conformance run needs slice 3 |
| 7 | edges | 045-17 | `frontend/apps/studio` | slice 0 (types from `openapi.yaml`); its journeys need slice 4 |
| 8 | edges | 045-16 | `sdks/<language>` hosts (six) | slice 6, the 1.8 host work of feature 006 in each SDK, and 045-9 for the server's 1.9 |
| 9 | either | 045-19, 045-21 | `tests/performance/`, `docs/` | slices 5 and 8 |

- [ ] T045-1 Write the contracts and documents first: the IR fields and their reasons (`workflow.schema.json`, `contracts/errors.md`
  of feature 001), protocol 1.9 (`worker.proto`, `worker.baseline.json` re-pinned after reading its diff), `openapi.yaml` (step
  deadlines, `SignalResult`, the signals list, the reasons), the journal fields (`docs/event-schema-evolution.md`), `sdks/SPEC.md`
  (§6.4 the heartbeat source and interval, §6.5 `nextRetryDelay` and the exception map, §9 the register row for 1.9 and the field
  register), `docs/architecture/execution-semantics.md` (the deadlines and the retry table), and the "superseded in part" pointers
  in Architecture decision 0030, Architecture decision 0050 and `specs/018-schedules-bulk-operations/spec.md` (FR-018-024,
  FR-018-039, Out of scope). The contract suites fail on the new baselines until slices 2 and 3.
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-014, Requirement 045-018, Requirement 045-023, Non-functional requirement 045-004
- [ ] T045-2 Domain types: `StepTimeouts`, the heartbeat source, `RetryPolicy`'s final codes and per-code overrides with
  `for_code`, the buffered-signal declaration, and their validation (bounds, contradictions, reserved and duplicated codes). Unit
  tests first, one per refusal and per boundary value (1,000 and 999; 31,622,400,000 and one more; 3,000 and 2,999; 86,400,000 and
  one more; 32 and 33 codes; 64 and 65 in all).
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-018
- [ ] T045-3 The retry decision table of Technical Plan 6 in `end_attempt`, with `next_retry_delay` on `Failure`, the close deadline
  in `StepState`, `retryRule` and `lastAttemptCode` on the events; `HEARTBEAT_TIMEOUT` in `outcome_unknown_codes`, with the SQL and
  Studio copies and the parity test. A unit test per row, first; the existing retry tests unchanged.
  Refs: Requirement 045-005, Requirement 045-007, Requirement 045-008
- [ ] T045-4 Events and commands: optional `start_deadline` and `close_deadline` on `StepReleased` and `StepRetryDue`, `deadline`
  on the ending events, `Expiry::AttemptTimeout {cause}` and `Expiry::HeartbeatTimeout`, `time_out_start` and `time_out_close` with
  their fences; replay equals projection with and without the fields (a corpus of journals written before this design).
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-013
- [ ] T045-5 `infra-ir`: parse the fields into the domain types and refuse with pointers and the closed reasons; an IR fixture per
  reason; publication refuses a version using a field until the requirement is recorded (`CAPABILITY_NOT_ACTIVE`).
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-017, Requirement 045-018
- [ ] T045-6 The migrations of Technical Plan 2 and Technical Plan 12 (columns, `NOT VALID` checks and their validation, indexes, the
  table, `runs.pending_signals`, grants for `sagat_app`, storage classification, a down migration that refuses data loss), and the
  event codec of the new fields. `integration_platform_foundation` (migrations as the runtime role, privilege audit) first.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-013, Non-functional requirement 045-003
- [ ] T045-7 The step statements write the deadlines and `min_protocol_minor`; the claim's three conditions; the lease and the
  renewal take the node's time-to-live; a withdrawal does not write the start deadline; the deadline scans of the store. The budget
  lines of `integration_claim_transition_efficiency` must pass unchanged (seen passing before and after); a store test per column.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-005, Requirement 045-006, Non-functional requirement 045-001
- [ ] T045-8 The deadline pass in `crates/scheduler` (both kinds, `holds` conditions, the shard window, the metrics and the no-op
  counter); the lost-lease pass maps the cause and the heartbeat timeout. Crash and duplicate tests in `fault_activity_timeouts`
  first.
  Refs: Requirement 045-009, Requirement 045-024
- [ ] T045-9 `crates/application` and `crates/api-grpc`: the per-node time-to-live in the claim and `heartbeat`, protocol 1.9
  negotiation (offered only with 1.8), the 1.9 lease fields, `next_retry_delay_ms` validated and passed to the domain. Contract test
  of the descriptor first (`contract_grpc_durable_execution`).
  Refs: Requirement 045-006, Requirement 045-008, Requirement 045-016, Non-functional requirement 045-004
- [ ] T045-10 `crates/api-http`: the step view's deadlines and cause, the publication reasons in the problem body. Contract test
  first (`contract_rest_durable_execution`).
  Refs: Requirement 045-014
- [ ] T045-11 The close deadline at feature 022's gates: the gate's exit on `time_out_close`, the grant's condition, and the two
  barrier tests of Technical Plan 7; built by whichever of 022's gate and this design lands second.
  Refs: Requirement 045-011
- [ ] T045-12 The fleet requirements `activity-timeouts-v1` and `signal-inbox-v1`: the capability in both process kinds, the
  publication gate, the refusal to remove one in use; registry tests first (`apps/control-plane`, `apps/execution-engine`).
  Refs: Requirement 045-017
- [ ] T045-13 Buffered signals in the domain and the store: the inbox in the run's state when `pending_signals > 0`, the wait entry
  that consumes, `signal.buffered`, `signal.discarded`, the bounds; unit tests and the replay corpus first.
  Refs: Requirement 045-019, Requirement 045-020
- [ ] T045-14 Buffered signals in the application and REST: `signalRun`, Signal-with-Start and its holder, the chain's signal, the
  continuation's move, the terminal discard, reset reapply, `GET /v1/runs/{runId}/signals`; integration tests per path first, the
  slot-order test of feature 018 with a buffered Signal-with-Start.
  Refs: Requirement 045-019, Requirement 045-021, Requirement 045-022, Requirement 045-023
- [ ] T045-15 The six SDK builders, validation and the exception-to-code map, with the conformance scenarios `activity-timeouts`
  (at 1.8), `retry-by-code` and `buffered-signals`; each SDK's checks of `sdks/AGENTS.md`.
  Refs: Requirement 045-015, Requirement 045-023
- [ ] T045-16 The six SDK hosts at 1.9: the interval from `heartbeat_timeout_ms`, the handler source with coalescing, the attempt
  deadline, `nextRetryDelay`; the conformance scenario `heartbeat-from-handler` and `activity-timeouts` at 1.9.
  Refs: Requirement 045-006, Requirement 045-016
- [ ] T045-17 The Studio: the inspector's fields and validation, the deadlines and the timeout sentences on a run's step, the
  pending-signal count and list; component tests first, then the journeys.
  Refs: Requirement 045-014, Requirement 045-023
- [ ] T045-18 The races of Technical Plan 7, each a barrier test, and the crash, duplicate, restart and lost-hint cases, in
  `fault_activity_timeouts` and `integration_activity_timeouts`; every suite also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  Refs: Requirement 045-010, Requirement 045-012
- [ ] T045-19 Measure Non-functional requirement 045-002 (1,000 deadlines in one second) and the index build on the scale fixture;
  record them with their environment in `tests/performance/reports/`, and decide Gap 045-001.
  Refs: Non-functional requirement 045-001, Non-functional requirement 045-002, Success criterion 045-006
- [ ] T045-20 Released 0.2 and mixed-fleet scenarios: runs from 0.2 finish unchanged, a 0.2 host is served at its minor and never
  leased a handler-sourced step, the requirement gate, rollback before and after the requirement.
  Refs: Requirement 045-017, Non-functional requirement 045-004
- [ ] T045-21 Documents for people: a guide page on timeouts and retries with the Temporal comparison and the unknown-outcome rule,
  a page on buffered signals, the runbook `docs/runbooks/step-timeouts.md` (the lag alert, the no-op counter, the index build),
  `docs/sdk-compatibility.md`, the release notes; `contract_developer_experience` regenerated only after reading its diff.
  Refs: Requirement 045-014, Requirement 045-024

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py`, `python3 scripts/check-agent-config.py`, `python3 scripts/check-design-docs.py`,
`python3 scripts/check-plain-words.py --base origin/develop`, the unit tests of the crates it changes and of those that depend on
them, and the suites below that it changes or that read what it changes (`scripts/select-test-targets.py`):

- `integration_activity_timeouts` and `fault_activity_timeouts` (new; every race, crash, duplicate, restart and lost-hint case),
  also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_durable_execution`, `integration_sagas_compensations` (the retry table, compensation untouched),
  `integration_workflow_primitives` (scoped waits consuming a buffered signal, continuation), `integration_schedules_bulk_operations`
  and `fault_schedules_bulk_operations` (Signal-with-Start, reset reapply, the slot order);
- `integration_claim_transition_efficiency` (the budget lines unchanged);
- `integration_platform_foundation` (migrations as the runtime role, the privilege audit);
- `integration_gitops_versioning` (publication refusals and the requirement gate);
- `contract_rest_durable_execution`, `contract_grpc_durable_execution`, the IR schema and protocol baseline tests;
- `sdk_conformance` for `activity-timeouts`, `retry-by-code`, `heartbeat-from-handler` and `buffered-signals` in six languages, and the
  per-SDK checks of `sdks/AGENTS.md`;
- `(cd frontend/apps/studio && npm run api && npm run check:api && npm test)`;
- the released 0.2 scenario of Task 045-20.

Failure cases (principle VIII): a crash of the pass between its read and its commit; two passes on one shard; a claim at the start
deadline's either side; a completion at the attempt deadline of each cause; a renewal against the heartbeat timeout; a withdrawal at
the start deadline; a gate's grant against the close deadline; a stop against each deadline; a signal against a wait's entry, a
wait's timeout, the run's end and a continuation; two waits on one buffered signal; a restart with deadlines pending; a lost
`task.ready` hint before a start deadline. A test that fails is a defect to explain, not to retry.
