# Activity timeouts and retry per failure code

*   **Author:** Claude Code, for the owner (under Owner decision 2026-11-20-6 and Owner decision 2026-11-24-4)
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

Release 0.3.0 gives a leased step the four timeouts a Temporal activity has and a retry policy that decides per failure code.
The owner put them in 0.3.0 (Owner decision 2026-11-24-1: "activity timeouts (schedule-to-start, schedule-to-close, heartbeat
timeout per activity) and non-retryable error types").

What a workflow author can write, on an `activity` or a `script` node (in the root graph and in the body of a collection or a
loop):

| Field | Meaning | Temporal's name |
|---|---|---|
| `timeoutMs` (exists) | One attempt, from its claim to its report | start-to-close |
| `scheduleToStartMs` (new) | One attempt, from the moment it can be claimed to its lease | schedule-to-start |
| `scheduleToCloseMs` (new) | The whole step, from its first release to its end: every attempt, retry wait and gate wait | schedule-to-close |
| `heartbeatTimeoutMs` (new) | The lease of one attempt lapses after this long without a renewal | heartbeat timeout |
| `heartbeatFrom` (new) | `host` (the host renews while the handler runs, today's behaviour) or `handler` (only a handler's `heartbeat()` renews) | the activity heartbeats |
| `retry.nonRetryableErrorCodes` (new) | Failure codes that end the step at once | `nonRetryableErrorTypes` |
| `retry.byErrorCode` (new) | A retry policy for named failure codes, over the node's base policy | none |

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
- **The journal says which deadline ended an attempt** (`deadline: startToClose`, `scheduleToClose` or `heartbeat`), and the Studio
  shows the deadlines of a step before they pass.

The rules are [ADR-0110](../adr/ADR-0110-step-timeouts-are-deadlines-on-the-database-clock-and-retry-is-decided-per-failure-code.md),
proposed with this design. The register of decisions, gaps and questions is `specs/045-activity-timeouts/research.md` (Decision
045-001 onward). The principles cited by numeral are those of [the constitution](../constitution.md).

### Terms

- **Attempt.** One lease of a step and what follows it, numbered from 1 (`steps.attempt`). A lease withdrawn because it never
  reached or never started on a host does not spend an attempt (Architecture decision 0098, on the branch of pull request #260,
  and Architecture decision 0099, on the branch of pull request #264, accepted by the owner on 2026-11-21; neither is merged yet).
- **Release.** The transition that first makes a step due to run: `step.released`, or, for a node with a concurrency or rate gate
  of feature 022, the moment it starts waiting at the gate.
- **Claimable.** A step is claimable when it is `READY`: released and past any gate (Architecture decision 0089), or a retry
  whose backoff has passed (`step.retry_due`).
- **Live attempt.** An attempt whose lease is committed: `LEASED` or `RUNNING`.
- **The database clock.** The transition's time (the database time the run transaction reads) when a deadline is written, and the
  database's `now()` when it is compared. No process clock decides a deadline.
- **Start deadline** (`steps.start_deadline_at`): claimable time plus `scheduleToStartMs`. **Close deadline**
  (`steps.close_deadline_at`): release time plus `scheduleToCloseMs`. **Attempt deadline** (`steps.attempt_deadline_at`, exists):
  the earlier of the claim's time plus `timeoutMs` and the close deadline. **Lease expiry** (`steps.lease_expires_at`, exists): the
  claim's or the last renewal's time plus the node's `heartbeatTimeoutMs` (or the platform's 30 s), never after the attempt deadline.
- **Outcome-unknown codes.** The seven codes for which `FailureCode::leaves_outcome_unknown` holds (`crates/domain/src/execution/event.rs`):
  `LEASE_EXPIRED`, `ATTEMPT_TIMEOUT`, `OUTCOME_UNKNOWN`, `STORAGE_QUOTA_EXCEEDED`, `RECONCILIATION_FAILED`,
  `RECONCILIATION_UNKNOWN`, `RECONCILIATION_NOT_FINAL`. This design adds none.

## The Problem

A step that nobody claims waits until its run's deadline: there is no way to say "if no worker takes this within five minutes, go to
the fallback". A step whose retries back off exponentially can outlive the business window it serves, because there is no budget
for the whole step. The lease time-to-live is one value for the platform (`lease_ttl`, 30 s, `crates/application/src/worker.rs`),
so a slow external call that blocks a host's event loop for 40 s loses its lease, and a worker process that dies holds a fast step
for 30 s however short the step is. A handler stuck in a deadlock keeps its lease forever, because the host renews it while the
handler runs. And the retry policy (`RetryPolicy` in `crates/domain/src/workflow.rs`) treats every failure alike: an author cannot
say "a declined card is final, a rate limit is retried ten times".

The roadmap assessment of 2026-11-11 (`docs/roadmap-assessment-2026-11-11.md`, Temporal-parity table) names these gaps:
"Schedule-to-start and schedule-to-close timeouts, per-activity heartbeat timeout, non-retryable error types: no spec found".

### Buffered signals are not in this design

The same table names "buffered signals (an early signal is refused today)". Feature 018 does not cover them: its spec lists them
under Out of scope and states "No signal is ever buffered" (FR-018-024), on Architecture decision 0050 decision 4 and Architecture
decision 0030 decision 17. They belong to a design of their own, which design 044 (workflow as code) will depend on, because a
workflow function reads signals from a channel when it chooses. [ADR-0111](../adr/ADR-0111-a-version-may-declare-buffered-signals-kept-in-a-bounded-per-run-inbox.md)
is its proposal: opt-in per version and signal name, a bounded inbox of the run written under the run lock, consumed once by a
later wait, moved across continue-as-new, and weighed against feature 004's `lookbackMs` path for events that arrive before a
wait. It shares no mechanism with this design (no timer, no lease, its own fleet requirement), so it is designed apart
(Decision 045-009, revised after the zoom-out review of 2026-11-24).

## Requirements

### Functional

- **Requirement 045-001 — Timeout fields.** An `activity` or `script` node, in the root graph (`$defs/activity`, `$defs/script`)
  and in a body (`$defs/bodyActivity`, `$defs/bodyScript`), MAY declare `scheduleToStartMs` and `scheduleToCloseMs` (each an
  integer from 1,000 to 31,622,400,000, that is one second to 366 days), `heartbeatTimeoutMs` (3,000 to 86,400,000, three seconds
  to 24 hours) and `heartbeatFrom` (`host`, the default, or `handler`). Publication MUST refuse, with the JSON pointer of the
  field: a value out of range; `heartbeatFrom: "handler"` without `heartbeatTimeoutMs`; `heartbeatTimeoutMs` greater than
  `timeoutMs` or than `scheduleToCloseMs`; `timeoutMs` greater than `scheduleToCloseMs`; `scheduleToStartMs` greater than
  `scheduleToCloseMs`. A compensation, a reconciliation or any other kind of node MUST NOT declare them. *Acceptance:* a
  publication test per refusal and per accepted combination (`integration_gitops_versioning`), the IR schema test, and the
  Studio's form test.
- **Requirement 045-002 — Retry fields.** A `retry` (`$defs/retry`, `$defs/bodyRetry`, and so a `compensationRetry`) MAY declare
  `nonRetryableErrorCodes` (1 to 32 distinct codes) and `byErrorCode` (1 to 16 entries, each with `codes`, 1 to 32 distinct codes,
  and any of `maxAttempts`, `initialDelayMs`, `maxDelayMs`, `strategy`, `jitter` with the meaning and bounds they have in `retry`).
  A code matches `^[A-Z0-9_]{1,64}$`, appears at most once across both lists, the lists name at most 64 codes in all, and none is
  one the platform decides elsewhere: the seven outcome-unknown codes, `CANCELLED`, `TASK_NOT_STARTED`, `SCHEDULE_TO_START_TIMEOUT`
  and `SCHEDULE_TO_CLOSE_TIMEOUT`. *Acceptance:* a publication test per refusal (`RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`,
  `RETRY_CODES_TOO_MANY`).
- **Requirement 045-003 — Schedule-to-start.** When an attempt of a node with `scheduleToStartMs` becomes claimable, its start
  deadline MUST be written in the same statement. A claim MUST NOT lease an attempt whose start deadline is at or before the
  database's `now()`. Once it has passed, the step MUST end `FAILED` with `SCHEDULE_TO_START_TIMEOUT`, which is not retried (no lease
  was committed for the attempt, so nothing ran) and takes the node's failure route. A lease withdrawal MUST NOT move the start
  deadline. *Acceptance:* the barrier test of a claim against the deadline, in both orders, and the withdrawal test (Technical
  Plan 7).
- **Requirement 045-004 — Schedule-to-close.** The close deadline MUST be written at the step's release and kept by every later
  attempt. Reached while no attempt is live, it MUST end the step `FAILED` with `SCHEDULE_TO_CLOSE_TIMEOUT`, a known failure: by the
  deadline pass for a `READY` step or a step waiting at a gate, and by the retries pass for a `RETRY_WAIT` step that it finds past
  the deadline. An attempt's deadline MUST be the earlier of its claim time plus `timeoutMs` and the close deadline; when the close
  deadline ends a live attempt, the attempt ends with `ATTEMPT_TIMEOUT` and `deadline: scheduleToClose`, and the outcome-unknown
  rule applies. *Acceptance:* the tests of Technical Plan 4, one per status in which the deadline can be reached.
- **Requirement 045-005 — Heartbeat timeout.** A lease of a node with `heartbeatTimeoutMs` MUST be written and renewed with that
  time-to-live instead of the platform's, never past the attempt deadline. An attempt that lapses MUST end with `LEASE_EXPIRED`, as
  today, and its event MUST record `deadline: heartbeat`, so the journal says that the author's bound, not the platform's, passed.
  *Acceptance:* the lapse test per node kind, with and without the field.
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
  leaves evidence (a refusal for a caller, a counted no-op for a pass). *Acceptance:* one barrier test per row, with held locks,
  never with sleeps (AGENTS.md rule 18).
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
- **Requirement 045-015 — SDK builders.** The six SDKs' workflow builders MUST offer the fields of Requirements 045-001 and 045-002,
  validated as the server validates them, and the SDKs MUST let a worker map exception classes to failure codes at registration
  (so that an author can name them in a retry list). *Acceptance:* each SDK's unit tests and the IR conformance scenarios.
- **Requirement 045-016 — SDK hosts at 1.9.** A host accepted at 1.9 MUST beat at a third of `heartbeat_timeout_ms` (within today's
  bounds of 500 ms and 10 s), honour `heartbeat_from_handler`, and send `next_retry_delay_ms` when the handler asks. *Acceptance:*
  each SDK's unit tests and the conformance scenarios `activity-timeouts` and `heartbeat-from-handler`.
- **Requirement 045-017 — Fleet switch and runs in flight.** A version that uses any field of this design MUST publish only once the
  operator recorded the requirement `activity-timeouts-v1`; a version published before MUST behave for every run exactly as today.
  *Acceptance:* the registry tests and the released 0.2 scenario of Technical Plan 13.
- **Requirement 045-018 — Telemetry and audit.** The metrics, spans and audit records of Technical Plan 11, with closed labels and no
  tenant, namespace or run identifier as a label. *Acceptance:* the metrics text test of `integration_activity_timeouts`.

### Non-functional

- **Non-functional requirement 045-001 — No cost on the hot path.** A lease stays 38 statements in 2 transactions and a completion 50
  statements in 1 (the lines of `tests/integration/claim-transition-efficiency/work_budget.rs`); the new columns are written by
  the statements that write the step today, and the claim's new conditions are on the row it already reads.
- **Non-functional requirement 045-002 — Lag.** A deadline is acted on by the next pass of its shard owner. Criterion, measured on the
  CI machine and reported with its environment (constitution, Delivery Constraints): with 1,000 start deadlines passing in the same
  second, every step ended within 5 s of its deadline, the 99th percentile under 2 s.
- **Non-functional requirement 045-003 — Isolation.** No new table and no new grant; the new columns are on `steps`, read and written
  under the run's tenant as every other column of the row.
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
- **Success criterion 045-005.** Non-functional requirements 045-001 and 045-002 measured and reported.

### User stories

- **User story 045-01.** As a workflow author, I bound how long a step may wait for a worker and how long it may take in all, and
  route to a fallback when it cannot (CUJs 1 and 2).
- **User story 045-02.** As a workflow author, I say which failures are final and how each other failure is retried (CUJ 3).
- **User story 045-03.** As an operator, I see why an attempt ended and what the platform does next (CUJ 4).

## Goals

- The four Temporal activity timeouts, with one meaning each and one clock.
- A retry decision per failure code that a reader of the version can audit.
- No automatic repetition of an effect whose outcome is unknown, whatever timeout ended it.
- No new statement on the claim or the completion, and no new table.

## Non-Goals

- Automatic retry after a deadline that ends a live attempt. Constitution principle III forbids it; `reconcileWith` is the way to
  make such a step go on by itself, and a qualified safe-repeat (Architecture decision 0076) is not in this design.
- Timeouts on compensation, reconciliation, managed database, lake, child, wait or approval nodes. Each has its own deadline today
  (`timeoutMs`, the wait's timeout, the child's run deadline); the managed runner nodes are Gap 045-003.
- Patterns or prefixes in the retry lists (Alternatives Considered).
- Buffered signals (The Problem, last section).
- Workflow as code: design 044 decides how its activity calls carry these options.
- Heartbeat details: feature 019 (Technical Plan 5 says how they fit).

## Open Questions

- Whether the deadline pass needs its own work budget per tick or shares the lost-lease pass's (Gap 045-001); the measurement of
  Task 045-17 decides.
- Whether a step's start deadline should also be offered to 019's priority order as an urgency hint (Gap 045-002); not in 0.3.0.

## Owner decisions needed

The owner's decisions this design rests on: Owner decision 2026-11-24-1 (the scope), Owner decision 2026-11-20-6 and Owner decision
2026-11-24-4 (the delegation of its questions, of its architecture decision and of its approval), Owner decision 2026-11-15-8 (the
register of protocol minors). Every question below was decided under that delegation and is recorded with its options and reasoning
in `specs/045-activity-timeouts/research.md`:

- Owner question 045-001: does a timeout that ends a live attempt retry, as in Temporal? No: outcome unknown, reconciliation or
  review (Decision 045-001, delegate (Owner decision 2026-11-24-4)); the constitution leaves no other choice, and the design says so
  to Temporal users.
- Owner question 045-002: is `SCHEDULE_TO_START_TIMEOUT` retried? No: final, failure route (Decision 045-002).
- Owner question 045-003: a new error-type field, or the existing `error_code`? `error_code` (Decision 045-003).
- Owner question 045-004: heartbeat from the host, from the handler, or both? Both, `host` by default (Decision 045-004).
- Owner question 045-005: a retry policy per code, or only a non-retryable list? Both (Decision 045-005).
- Owner question 045-006: which node kinds? `activity` and `script`, root and body (Decision 045-006).
- Owner question 045-009: where do buffered signals belong? A design of their own (Decision 045-009, revised).

## Critical User Journeys (CUJs)

### CUJ 1: No worker takes the task

*   **Given**: a `chargeCard` activity with `scheduleToStartMs: 30000` and `onError: notifyOps`, and every payments worker down.
*   **When**: the step becomes `READY` and 30 s pass on the database clock.
*   **Then**: the next pass ends it `SCHEDULE_TO_START_TIMEOUT`, the run goes to `notifyOps`; a worker that comes back at second 31
    is not leased the attempt; the Studio showed the start deadline while it waited.

### CUJ 2: Retries that outlive their window

*   **Given**: `reserveSeat` with `scheduleToCloseMs: 120000`, `retry: {maxAttempts: 10, initialDelayMs: 5000}` and attempts failing
    with `UPSTREAM_BUSY`.
*   **When**: the fourth failure, at second 95, would schedule its retry 40 s later (5 s doubled three times), at second 135.
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
*   **Then**: the host stops renewing; 20 s later the lease lapses (`LEASE_EXPIRED`, `deadline: heartbeat`); `findExport` asks the
    provider what happened; the operator sees "outcome unknown: reconciling" and the deadline that passed.

## Technical Plan (Architecture)

### 1. The IR

Additive to `specs/001-platform-foundation/contracts/workflow.schema.json` (Architecture decision 0032), on `$defs/activity`,
`$defs/script`, `$defs/bodyActivity` and `$defs/bodyScript`:

```json
"scheduleToStartMs": {"type": "integer", "minimum": 1000, "maximum": 31622400000},
"scheduleToCloseMs": {"type": "integer", "minimum": 1000, "maximum": 31622400000},
"heartbeatTimeoutMs": {"type": "integer", "minimum": 3000, "maximum": 86400000},
"heartbeatFrom": {"enum": ["host", "handler"]}
```

On `$defs/retry` and `$defs/bodyRetry` (and so on `compensationRetry`), which keep `additionalProperties: false`:

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

The schema cannot express the cross-field rules of Requirements 045-001 and 045-002 (a timeout greater than another, a code in
both lists, a reserved code); `crates/infra-ir` checks them and refuses with the field's pointer and a closed reason
(`TIMEOUT_OUT_OF_RANGE`, `TIMEOUTS_CONTRADICT`, `HEARTBEAT_SOURCE_NEEDS_TIMEOUT`, `TIMEOUT_NOT_ALLOWED_ON_NODE`,
`RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`, `RETRY_CODES_TOO_MANY`), listed in `contracts/errors.md` of feature 001 by Task
045-1.

In the domain (`crates/domain/src/workflow.rs`), a leased node gains its timeouts (the two schedule durations and the heartbeat
timeout with its source) and `RetryPolicy` gains its final codes and its overrides per code, with one function that answers, for a
code, "final" or the effective policy. No `serde_json` enters the domain (principle II); `infra-ir` builds the types.

### 2. The deadlines and their columns

One migration in `crates/infra-postgres/migrations/` (the `/migration` checklist), additive:

| Column of `steps` | Type | Written by | Cleared by |
|---|---|---|---|
| `start_deadline_at` | `timestamptz NULL` | the statement that makes the attempt `READY` (release past its gates, retry due, gate grant) | the claim, any end of the step |
| `close_deadline_at` | `timestamptz NULL` | the statement that releases the step (or starts its gate wait) | the end of the step |
| `attempt_deadline_cause` | `text NULL`, `CHECK` in (`START_TO_CLOSE`, `SCHEDULE_TO_CLOSE`) | the claim, with `attempt_deadline_at` | with `attempt_deadline_at` |
| `min_protocol_minor` | `smallint NOT NULL DEFAULT 0` | the release: 9 for `heartbeatFrom: "handler"`, 0 otherwise | never |

Each is computed in SQL from the transition's database time and the node's durations, bound as microseconds, as
`attempt_deadline_at` is today, in the three statements that write a step (`crates/infra-postgres/src/execution.rs`: the root
statement, the scoped statement of a full-run transaction, the path statement `upsert_path_instance`). The claim writes
`attempt_deadline_at` as the earlier of the claim time plus `timeoutMs` and `close_deadline_at`, and the cause of the earlier one;
a tie goes to `START_TO_CLOSE`. `CHECK` constraints are added `NOT VALID` and validated in a second migration, as design 043 did.
`min_protocol_minor` is this design's own need (Technical Plan 5); draft 031 may later use it for its payload-plan gating, which
would be its own decision.

Two partial indexes, shaped like the lost-lease indexes of `20261017000000_scheduler_shards.up.sql` and added to that migration's
list of indexes refused when a concurrent build left them invalid:
`steps_start_deadline_shard_idx ON steps (run_shard(run_id), start_deadline_at) WHERE status = 'READY' AND start_deadline_at IS NOT
NULL`, and `steps_close_deadline_shard_idx ON steps (run_shard(run_id), close_deadline_at) WHERE status IN ('READY', 'WAITING') AND
close_deadline_at IS NOT NULL` (a `WAITING` row carries a close deadline only at a gate). A `RETRY_WAIT` row is not in the index:
the retries pass, which already reads every due retry, ends it when it finds it past its close deadline (Technical Plan 4).

In the domain, the step's state gains its close deadline, set from `step.released` and kept across retries, because the retry
decision needs it (Technical Plan 6). `step.released` and `step.retry_due` gain optional start and close deadlines, which the
reducer applies and the store writes; replay restores them; an event written before this design has none.

### 3. The claim and the start deadline

The claim's condition (the root claim and the path claim, `crates/infra-postgres/src/execution.rs` and `execution/path.rs`) adds,
on the row it already selects: the start deadline is absent or after `now()`, the close deadline is absent or after `now()`, and
`min_protocol_minor` is at most the session's accepted minor. No statement is added (Non-functional requirement 045-001). A step
skipped by the first two conditions is ended by the pass.

The pass, named `step_deadlines` (beside `leases` and the run-level `deadlines` of `crates/scheduler/src/passes.rs`), reads, per
shard and window, the rows of `steps_start_deadline_shard_idx` whose deadline is at or before `now()`, and for each opens the
run's transaction (chain lock, then run lock, as every run transaction does) with a condition checked under the lock, as the
lost-lease pass's `holds` checks its own: the row is still `READY`, its generation is the one read, its start deadline is still at
or before the transaction's time. If it holds, the domain ends the step `FAILED` with `SCHEDULE_TO_START_TIMEOUT` and no fence, and
settles (the failure route, a saga's compensation); if not, the pass counts a no-op. A run with a stop reason is left to the stop
path, which cancels the step.

A lease withdrawal (Architecture decision 0098 decision 3, Architecture decision 0099 decision 5) sets the step back to `READY`
without spending an attempt. Its statement MUST NOT write `start_deadline_at`: the withdrawn lease never started, so the attempt is
the same attempt and keeps its deadline. If that deadline has passed, the pass ends the step: nothing ran, so the failure is known.

### 4. The close deadline

Written at the release. While no attempt is live, the step is ended `FAILED` with `SCHEDULE_TO_CLOSE_TIMEOUT` and no fence:

| Status when it passes | What happens |
|---|---|
| `READY` | the `step_deadlines` pass ends it; the claim already refuses it |
| `RETRY_WAIT` | the retries pass, when it finds the retry due, ends it instead of releasing it; a retry is never scheduled at or after the deadline in the first place (Technical Plan 6, row 6), so this happens only when the retries pass lags |
| `WAITING` at a gate of feature 022 | the `step_deadlines` pass ends it through the gate's exit: the waiter row of `concurrency_waiters` or the rate-limit queue entry is removed in the same transaction, the way the gate's own wait timeout removes it (`specs/022-flow-control-step-policies/data-model.md`) |
| `LEASED` or `RUNNING` | nothing here: the attempt deadline is the close deadline, and the lost-lease pass ends the attempt `ATTEMPT_TIMEOUT` with `deadline: scheduleToClose` |

The grant routine of Architecture decision 0089 adds the condition "close deadline absent or after `now()`" to the step it would
make `READY`; a step whose deadline passed is left to the pass. Feature 022's gates are specified and being built
(`concurrency_waiters` is not in `crates/` yet); whichever of them and this design is built second adds the gate rows of this table
and their tests (Task 045-10).

The ending events of a live attempt that a deadline ended (`step.failed`, `step.reconciliation_requested`, `step.needs_review`)
gain an optional `deadline` field (`startToClose`, `scheduleToClose`, `heartbeat`), read by the lost-lease pass from
`attempt_deadline_cause` or from the node's heartbeat timeout.

### 5. The heartbeat timeout and its source

The lease's time-to-live becomes per node: the lease grant and the renewal (`crates/application/src/worker.rs`, the claim and
`heartbeat`) take the node's `heartbeatTimeoutMs` when it is declared and `settings.lease_ttl` otherwise; the statement already
caps the expiry at the attempt deadline. The lost-lease pass ends a lapsed attempt with `LEASE_EXPIRED`, as today, and records
`deadline: heartbeat` when the node declares the field. A renewal that commits before the pass's transaction saves the attempt, as
today: the pass re-checks the expiry under the run lock. No failure code is added, so the outcome-unknown list and its two copies
(`crates/infra-postgres/src/chain_progress.rs`, `frontend/apps/studio/src/features/runs/ops.ts`) do not change.

With delivery acknowledgement (Architecture decision 0099 decisions 2 and 6) a lease is marked provisional only when its earlier
deadline is at least 18 s after the claim; a node with `heartbeatTimeoutMs` under 18,000 is therefore never provisional and runs
with today's meaning, which is correct and needs no rule here.

**The source.** `heartbeatFrom: "host"` keeps today's host loop: it beats while the handler runs. A host older than 1.9 computes its
interval as a third of the lease's time left at receipt, clamped to 500 ms and 10 s (`sdks/SPEC.md` §6.4), so it beats in time for
any `heartbeatTimeoutMs` from 3,000 ms (a third is 1,000 ms, three beats per window), as long as its clock agrees with the
database's; a 1.9 host reads the duration itself (Technical Plan 8) and does not depend on its clock. `heartbeatFrom: "handler"`
needs a host that knows it: the release writes `min_protocol_minor = 9` and the claim leases the step only to a session at 1.9 or
later (Technical Plan 3). Such a host sends the first beat itself when the handler starts, and afterwards sends a beat at its
interval only if the handler called `heartbeat()` since the last beat sent; calls between beats are coalesced. The handler can
therefore call it as often as it likes; at most one beat per interval reaches the server.

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
(Architecture decision 0076) is applied after row 4 or row 5 exactly as after a known non-retryable failure today. The unhandled
exception of an SDK is `ACTIVITY_ERROR`, which an author may name; Requirement 045-015 lets a worker map its own exception classes
to codes at registration (for example `CardDeclinedError` to `CARD_DECLINED`), so that an exception the handler did not catch still
reaches the policy with its own code instead of `ACTIVITY_ERROR`, which is how a Temporal user's `nonRetryableErrorTypes` of class
names is written here.

Row 6 is the only place the close deadline enters the domain; it compares two database-clock times (the failure's time and the
stored deadline), so a replay decides the same.

### 7. The races (AGENTS.md rule 9)

Every row is decided under the run lock, which every writer of a step takes (constitution principle I); the loser commits nothing
or a counted no-op.

| Race | Resolution | Test (barrier, never a sleep) |
|---|---|---|
| a claim and the start deadline | the claim's condition on `now()`: before the deadline the lease commits and the pass's check finds `LEASED`; at or after, the claim skips it and the pass ends it | hold the run lock in the pass, claim at the deadline's either side |
| a claim and the close deadline | the same, with the close deadline | the same |
| a completion and the attempt deadline (either cause) | first commit wins, as today; a late report is `STALE_ATTEMPT` | existing test extended with `scheduleToClose` |
| a renewal and the heartbeat timeout | first commit wins, as today | the lost-lease test with a per-node time-to-live |
| a failure's retry and the close deadline | decided in the failure's transaction (row 6); a retry that is due is claimable only before the deadline | unit test at deadline minus one microsecond, at it, after it |
| the retries pass and the close deadline | the retries pass ends a `RETRY_WAIT` step past its deadline instead of releasing it | lagging retries pass, held |
| a gate's grant and the close deadline | the grant's condition; the pass re-checks | held grant routine against the pass |
| a lease withdrawal and the start deadline | the withdrawal keeps the deadline; the pass ends the step if it passed | withdrawal at the deadline's either side |
| a stop and a deadline | the first commit; both end the step without an effect | stop held against the pass |
| a crash of the pass, two passes | nothing committed is repeated: the check re-reads status, generation and deadline | kill between read and commit; two passes on one shard |

A lost broker hint does not delay a deadline: the pass reads the database, not the broker (constitution principle VII). A restart
loses nothing: every deadline is a column.

### 8. Protocol 1.9 and the field register

The register of `sdks/SPEC.md` §9 on `develop` stops at 1.7 (design 041); 1.8 is reserved for delivery acknowledgement by Owner
decision 2026-11-21-6, on the branch of pull request #264, not merged. This design names 1.9, the next free minor, and takes it
when its contract merges (Owner decision 2026-11-15-8). Minors are cumulative: a 1.9 host also carries 1.7 and the 1.8 host
procedure, so the SDK work of 1.9 follows the SDK work of 1.8 (Technical Plan 14, Risks).

```protobuf
message TaskLease {
  // fields 1 to 18 unchanged; 19 to 22 named by draft 019; 23 and 24 by 1.8; 25 by draft 031
  // Added in 1.9. The node's heartbeatTimeoutMs, or 0 for the platform's lease time-to-live: a duration, so that the
  // host's interval does not depend on its clock agreeing with the database's.
  uint32 heartbeat_timeout_ms = 26;
  // Added in 1.9. True when the node declares heartbeatFrom: "handler".
  bool heartbeat_from_handler = 27;
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
| `TaskLease` | 1 to 18 today; 19 to 22 draft 019; 23 and 24 protocol 1.8; 25 draft 031 (moved from 19); 26 and 27 this design |
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
  retry options `nonRetryableErrorCodes`, `byErrorCode`. Each validates what the server validates (Requirements 045-001 and
  045-002) and raises the same reason before anything is sent.
- **Worker registration.** A map from exception class to failure code (TypeScript: a constructor; Go: an `errors.As` target;
  Python: a class; C#: a `Type`; Java: a `Class`; Rust: a function on the error), consulted before an unhandled error becomes
  `ACTIVITY_ERROR`. Local only: nothing on the wire changes.
- **Host at 1.9.** The interval is a third of `heartbeat_timeout_ms` when it is set, within 500 ms and 10 s;
  `heartbeat_from_handler` switches to the handler source of Technical Plan 5; `ActivityFailure` takes an optional
  `nextRetryDelay`, sent as `next_retry_delay_ms` (raised locally as `VALIDATION_FAILED` out of range).
- **Conformance** (`tests/sdk-conformance/`): `activity-timeouts` (the three new timeouts, at 1.8 and at 1.9), `retry-by-code`,
  `heartbeat-from-handler` (1.9 only); the same scenarios in six languages (AGENTS.md rule 8).

### 10. REST and the Studio

`specs/001-platform-foundation/contracts/openapi.yaml`, additive: the step view gains `startDeadlineAt`, `closeDeadlineAt`,
`attemptDeadlineAt` and `attemptDeadlineCause`; the publication reasons of Technical Plan 1 are added to the problem body.

The Studio (`frontend/apps/studio/src/features/workflows/fields.tsx` and the run view in `features/runs/`): the four fields and the
retry lists in the inspector with the server's validation; on a step of a run, its deadlines with the time left (the projection's
lag shown, principle VI) and, for each timeout, one sentence of what happened and what the platform does next ("No worker took the
task in time: the step failed and its failure route ran"; "The attempt lost its heartbeat: the outcome is unknown, so the platform
is asking `findExport`"). Types follow `openapi.yaml` (`npm run api && npm run check:api`).

### 11. Telemetry and audit

- Metrics, closed labels only: `step_timeouts_total{kind="schedule_to_start|schedule_to_close|start_to_close|heartbeat|lease"}`,
  `step_retry_decisions_total{rule="worker|non_retryable_code|attempts_spent|close_deadline|base|by_error_code"}`,
  `step_deadline_lag_seconds` (histogram: from the deadline to the pass's commit), `step_deadline_pass_noops_total{kind}`.
- Spans: the pass's transaction carries `sagat.deadline.kind`; the claim's span carries how many rows its new conditions skipped.
- Audit: publication of a version with these fields is audited as every publication. A timeout is a system transition, journaled and
  not audited, as a lease expiry is today.
- Logs: no payload, and no code that the worker sent unless it passed the code pattern.

### 12. What this design does not change

The journal's event types, the failure codes a worker may send, the outcome-unknown list, the run-level deadline, the wait and
approval timeouts, and every compensation and reconciliation rule.

### 13. Upgrade, rollback and runs in flight

- A version published before this design has none of the fields: every run of it behaves as today, at every minor, through every
  rolling upgrade (principle IV).
- The operator records `activity-timeouts-v1` (`platform_requirement_activate`, Architecture decision 0082) once every engine and
  control plane runs a build with the capability; until then publication of a version using a field answers `422` with the reason
  `CAPABILITY_NOT_ACTIVE`. Once recorded, a binary without the capability serves nothing (the closed gate).
- Removing the requirement is refused while a published, non-retired version uses its fields.
- The migration adds columns; its down migration refuses to run while a column holds a value (the `/migration` checklist).
- Released 0.2 scenario (`scripts/test-release-upgrade-profile.py`): runs started on 0.2 finish on 0.3 unchanged; a 0.2 SDK host is
  served at its minor; a version with `heartbeatFrom: "handler"` is never leased to it.

### 14. Other features

| Feature | Status on 2026-11-24 | What this design asks of it, or gives it |
|---|---|---|
| 006, protocol 1.8 (Architecture decision 0099) | accepted, on the branch of pull request #264, not merged | 1.9 follows 1.8 in the server and in each SDK; a withdrawal keeps the start deadline (Technical Plan 3) |
| lease withdrawal (Architecture decision 0098) | on the branch of pull request #260, not merged | the same withdrawal rule |
| 019 (worker deployments) | approved draft | its `heartbeat(details)` is a handler heartbeat; its detached maximum duration is bounded by the close deadline; its priority order is unchanged by the start deadline (Gap 045-002); field numbers above |
| 020 (visibility) | approved draft | `attemptDeadlineCause` and the deadline causes are additive facts it may list |
| 022 (flow control) | specified, being built | the close deadline ends a gated step through the gate's exit; the grant refuses a step past it |
| 030 (cross-namespace services) | approved draft | its operation deadlines use the same names and clock (`scheduleToCloseMs`, `scheduleToStartMs`) |
| 031 (client-side encryption) | approved draft | `error_code` and `safe_message` are never encoded |
| 037 (compensation safety) | built | unchanged |
| 044 (workflow as code) | being designed | the options of an activity call map to these fields |

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A Temporal user expects a timed-out attempt to retry | The Studio's sentence, the guide's comparison table and `reconcileWith` as the documented way to go on automatically |
| Protocol 1.8 slips, so 1.9 cannot ship in the SDKs | Slices 1 to 6 ship without it: the three timeouts in host mode, the retry table and the Studio work at 1.8 and below (old hosts beat from the lease's expiry). Only `heartbeatFrom: "handler"` and `nextRetryDelay` wait, and publication refuses `heartbeatFrom: "handler"` until a server offers 1.9 (`CAPABILITY_NOT_ACTIVE`) |
| Building two partial indexes on a large `steps` table holds a lock that blocks writes while it reads | The upgrade notes give the measured time on the scale fixture (Task 045-17); an operator with a very large table builds them `CONCURRENTLY` by hand before the upgrade, which the migration detects and skips, and a failed concurrent build is refused as the existing indexes' is |
| A short `heartbeatTimeoutMs` on a slow network loses leases | The minimum is 3,000 ms (three beats of 1,000 ms per window, against a ready-to-handler 99th percentile of 751 to 889 ms measured in `tests/performance/reports/sdk-streaming.md`) |
| A host clock far from the database's distorts an old host's interval | A 1.9 host uses the duration; an older host's interval is clamped as today, and `sdks/SPEC.md` §6.4 tells it to set `heartbeatIntervalMs` |
| The deadline pass falls behind under load | Its lag is a histogram with an alert in the runbook; it shares the shard's window and is measured (Gap 045-001) |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Every deadline is a column written by the transition that starts it and acted on in one run transaction under the run lock; the claim's condition and the pass's re-check make a lease and a timeout exclusive; the retry decision is journaled with its rule |
| II. Modular domain, ports and adapters | Pass | New types in `crates/domain` without `serde_json`; parsing in `infra-ir`; SQL in `infra-postgres`; the pass in `scheduler`; no new crate edge (`scripts/check-architecture.py`) |
| III. External effects are at-least-once; sagas compensate | Pass | A deadline that ends a live attempt is an unknown outcome (reconciliation or review); only a deadline with no live attempt is a known failure; compensation and reconciliation nodes are untouched |
| IV. Compatibility is versioned and pinned | Pass | Optional IR fields, protocol 1.9 additive, optional journal fields, a fleet requirement; versions published before are unchanged; no failure code changes meaning |
| V. Code, tenant boundary and secrets are isolated | Pass | No code runs on the server; no new table or grant; a worker's delay is bounded to 24 hours and spends an attempt like any retry |
| VI. Observability and audit are part of the feature | Pass | Closed-label metrics, spans, deadlines and their cause in the projection with its lag |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Deadlines do not depend on a broker hint |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Barrier tests per race, crash and duplicate pass tests, restart, lost hint, conformance in six languages, contract tests; gaps registered |
| Delivery Constraints | Pass | No new technology; single region; one migration; the registry of Architecture decision 0082 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Four columns and two indexes on `steps` | A deadline must be a durable fact the claim and a pass can read on the database clock | Timer rows per deadline: one more row and statement per attempt on the hot path |
| `min_protocol_minor` on `steps` | A handler-sourced heartbeat must never reach a host that cannot honour it | Refuse to publish until every host is at 1.9: the server cannot know the hosts |
| `byErrorCode` beside `nonRetryableErrorCodes` | The owner's brief asks for a retry policy per error type; a rate limit and a bug need different patience, and `nextRetryDelay` alone cannot give a rate limit more attempts than a bug | Only the list: forces an author to choose between giving up and hammering a provider |
| `heartbeat_timeout_ms` on the lease although an old host derives its interval from the expiry | The expiry is an instant on the database's clock; a duration lets a host beat in time whatever its clock says, as protocol 1.8 chose for its own fields | The instant alone: a host whose clock is off by more than the window beats too late |

## Alternatives Considered

- **Temporal's semantics (retry after every timeout).** Rejected by principle III (ADR-0110, option 2).
- **Timer rows for deadlines.** Rejected for the hot-path cost (ADR-0110, option 3).
- **A new error-type field.** Rejected: `error_code` is the type (ADR-0110, option 4).
- **A new `HEARTBEAT_TIMEOUT` code.** Rejected after the zoom-out review: it would mean what `LEASE_EXPIRED` means, add an eighth
  outcome-unknown code and its copies, and give every consumer two codes to match; the `deadline` field says which bound passed.
- **Patterns in the retry lists.** Left out: exact codes keep the decision auditable; addable later without breaking a version.
- **Schedule-to-start counted from the release, gates included.** Rejected: a gate is a deliberate hold by the platform, and
  schedule-to-start exists to detect a missing worker; the close deadline covers the gate's time.
- **Heartbeat timeout as a second clock beside the lease.** Rejected: two expiries for one attempt would race each other; the
  lease expiry is the heartbeat timeout.
- **An attempt's time left on every lease (`attempt_time_left_ms`).** Dropped after the zoom-out review: no requirement needs it;
  protocol 1.8 already gives it to a flagged lease.

## Non-Functional Requirements

Non-functional requirements 045-001 to 045-004 are stated under Requirements. Their tests: the budget lines of
`integration_claim_transition_efficiency` (unchanged) for 045-001; the lag measurement of Task 045-17 for 045-002; the privilege
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
| 3 | server | 045-8, 045-9, 045-11 | `crates/scheduler`, `crates/application`, `crates/api-http`, `crates/api-grpc`, `apps/` | slice 2; for 1.9 in 045-9, the server side of 1.8 |
| 4 | server | 045-10, 045-16, 045-18 | `crates/application`, `crates/infra-postgres`, `tests/integration/activity-timeouts/`, `tests/fault/activity-timeouts/` | slice 3; 045-10 also on feature 022's gates |
| 5 | edges | 045-12 | `sdks/<language>` builders (six), `tests/sdk-conformance/` scenarios | slice 0; its conformance run needs slice 3 |
| 6 | edges | 045-14 | `frontend/apps/studio` | slice 0 (types from `openapi.yaml`); its journeys need slice 3 |
| 7 | edges | 045-13 | `sdks/<language>` hosts (six) | slice 5, the 1.8 host work of feature 006 in each SDK, and 045-9 for the server's 1.9 |
| 8 | either | 045-15, 045-17 | `docs/`, `tests/performance/` | slices 4 and 7 |

- [ ] T045-1 Write the contracts and documents first: the IR fields and their reasons on `activity`, `script`, `bodyActivity`,
  `bodyScript`, `retry` and `bodyRetry` (`workflow.schema.json`, `contracts/errors.md` of feature 001), protocol 1.9
  (`worker.proto`, `worker.baseline.json` re-pinned after reading its diff), `openapi.yaml` (step deadlines, the reasons), the
  journal fields (`docs/event-schema-evolution.md`), `sdks/SPEC.md` (§6.4 the heartbeat source and interval, §6.5 `nextRetryDelay`
  and the exception map, §9 the register row for 1.9 and the field register) and `docs/architecture/execution-semantics.md` (the
  deadlines and the retry table). The contract suites fail on the new baselines until slices 2 and 3.
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-014, Non-functional requirement 045-004
- [ ] T045-2 Domain types: the timeouts of a leased node with the heartbeat source, `RetryPolicy`'s final codes and overrides per
  code, and their validation (bounds, contradictions, reserved and duplicated codes). Unit tests first, one per refusal and per
  boundary value (1,000 and 999; 31,622,400,000 and one more; 3,000 and 2,999; 86,400,000 and one more; 32 and 33 codes; 64 and
  65 in all).
  Refs: Requirement 045-001, Requirement 045-002
- [ ] T045-3 The retry decision table of Technical Plan 6 in `end_attempt`, with the worker's delay on the failure, the close
  deadline in the step's state, and `retryRule` and `lastAttemptCode` on the events. A unit test per row, first; the existing retry
  tests unchanged.
  Refs: Requirement 045-007, Requirement 045-008
- [ ] T045-4 Events and commands: optional start and close deadlines on `step.released` and `step.retry_due`, `deadline` on the
  ending events (`startToClose`, `scheduleToClose`, `heartbeat`), the two timeouts that end a step with no live attempt, each under
  its fence; replay equals projection with and without the fields (a corpus of journals written before this design).
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-005, Requirement 045-013
- [ ] T045-5 `infra-ir`: parse the fields into the domain types and refuse with pointers and the closed reasons; an IR fixture per
  reason; publication refuses a version using a field until the requirement is recorded (`CAPABILITY_NOT_ACTIVE`).
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-017
- [ ] T045-6 The migrations of Technical Plan 2 (columns, `NOT VALID` checks and their validation, the two indexes and their entry
  in the invalid-index check, a down migration that refuses data loss, the storage classification) and the event codec of the new
  fields. `integration_platform_foundation` (migrations as the runtime role, privilege audit) first.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-013, Non-functional requirement 045-003
- [ ] T045-7 The step statements write the deadlines and `min_protocol_minor`; the claim's three conditions; the lease and the
  renewal take the node's time-to-live; a withdrawal does not write the start deadline; the deadline scans of the store. The budget
  lines of `integration_claim_transition_efficiency` must pass unchanged (seen passing before and after); a store test per column.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-005, Requirement 045-006, Non-functional requirement 045-001
- [ ] T045-8 The `step_deadlines` pass in `crates/scheduler` (both kinds, the checks under the lock, the shard window, the metrics and
  the no-op counter); the retries pass ends a `RETRY_WAIT` step past its close deadline; the lost-lease pass records the deadline
  cause. Crash and duplicate tests in `fault_activity_timeouts` first.
  Refs: Requirement 045-004, Requirement 045-009, Requirement 045-018
- [ ] T045-9 `crates/application` and `crates/api-grpc`: the per-node time-to-live in the claim and `heartbeat`, protocol 1.9
  negotiation (offered only with 1.8), the 1.9 lease fields, `next_retry_delay_ms` validated and passed to the domain. Contract test
  of the descriptor first (`contract_grpc_durable_execution`).
  Refs: Requirement 045-006, Requirement 045-008, Requirement 045-016, Non-functional requirement 045-004
- [ ] T045-10 The close deadline at feature 022's gates: the gate's exit, the grant's condition, and the two barrier tests of
  Technical Plan 7; built by whichever of 022's gate and this design lands second.
  Refs: Requirement 045-011
- [ ] T045-11 `crates/api-http` and the fleet requirement: the step view's deadlines and cause, the publication reasons in the
  problem body (contract test first, `contract_rest_durable_execution`); the capability `activity-timeouts-v1` in both process
  kinds, the publication gate and the refusal to remove it while in use (registry tests first, `apps/control-plane`,
  `apps/execution-engine`).
  Refs: Requirement 045-014, Requirement 045-017
- [ ] T045-12 The six SDK builders, validation and the exception-to-code map, with the conformance scenarios `activity-timeouts` (at
  1.8) and `retry-by-code`; each SDK's checks of `sdks/AGENTS.md`.
  Refs: Requirement 045-015
- [ ] T045-13 The six SDK hosts at 1.9: the interval from `heartbeat_timeout_ms`, the handler source with coalescing,
  `nextRetryDelay`; the conformance scenario `heartbeat-from-handler` and `activity-timeouts` at 1.9.
  Refs: Requirement 045-006, Requirement 045-016
- [ ] T045-14 The Studio: the inspector's fields and validation, the deadlines and the timeout sentences on a run's step; component
  tests first, then the journeys.
  Refs: Requirement 045-014
- [ ] T045-15 Documents for people: a guide page on timeouts and retries with the Temporal comparison and the unknown-outcome rule,
  the runbook `docs/runbooks/step-timeouts.md` (the lag alert, the no-op counter, the index build), `docs/sdk-compatibility.md`, the
  release notes; `contract_developer_experience` regenerated only after reading its diff.
  Refs: Requirement 045-014, Requirement 045-018
- [ ] T045-16 The races of Technical Plan 7, each a barrier test, and the crash, duplicate, restart and lost-hint cases, in
  `fault_activity_timeouts` and `integration_activity_timeouts`; every suite also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  Refs: Requirement 045-010, Requirement 045-012
- [ ] T045-17 Measure Non-functional requirement 045-002 (1,000 deadlines in one second) and the index build on the scale fixture;
  record them with their environment in `tests/performance/reports/`, and decide Gap 045-001.
  Refs: Non-functional requirement 045-001, Non-functional requirement 045-002, Success criterion 045-005
- [ ] T045-18 Released 0.2 and mixed-fleet scenarios: runs from 0.2 finish unchanged, a 0.2 host is served at its minor and never
  leased a handler-sourced step, the requirement gate, rollback before and after the requirement.
  Refs: Requirement 045-017, Non-functional requirement 045-004

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
  `integration_workflow_primitives` (deadlines of steps in bodies);
- `integration_claim_transition_efficiency` (the budget lines unchanged);
- `integration_platform_foundation` (migrations as the runtime role, the privilege audit);
- `integration_gitops_versioning` (publication refusals and the requirement gate);
- `contract_rest_durable_execution`, `contract_grpc_durable_execution`, the IR schema and protocol baseline tests;
- `sdk_conformance` for `activity-timeouts`, `retry-by-code` and `heartbeat-from-handler` in six languages, and the per-SDK checks of
  `sdks/AGENTS.md`;
- `(cd frontend/apps/studio && npm run api && npm run check:api && npm test)`;
- the released 0.2 scenario of Task 045-18.

Failure cases (principle VIII): a crash of the pass between its read and its commit; two passes on one shard; a claim at the start
deadline's either side; a completion at the attempt deadline of each cause; a renewal against the heartbeat timeout; a withdrawal at
the start deadline; a gate's grant against the close deadline; a lagging retries pass against the close deadline; a stop against
each deadline; a restart with deadlines pending; a lost `task.ready` hint before a start deadline. A test that fails is a defect to
explain, not to retry.
