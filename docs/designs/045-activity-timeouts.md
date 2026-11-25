# Activity timeouts and retry per failure code

*   **Author:** Claude Code, for the owner (under Owner decision 2026-11-20-6 and Owner decision 2026-11-24-4)
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

Release 0.3.0 gives a leased step the four timeouts a Temporal activity has and a retry policy that decides per failure code.
The owner put them in 0.3.0 (Owner decision 2026-11-24-1: "activity timeouts (schedule-to-start, schedule-to-close, heartbeat
timeout per activity) and non-retryable error types").

What a workflow author can write. The timeouts go on an `activity` or a `script` node, in the root graph and in the body of a
collection or a loop; the retry fields go on every `retry` object, `compensationRetry` included:

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
  a deadline that ends a live attempt leaves its outcome unknown, so the step is reconciled (`reconcileWith`, on an activity) or
  reviewed, as feature 037 decided for every lost attempt (constitution principle III). A deadline that ends no live attempt (a task
  nobody claimed, a retry that would start too late) is a known failure and takes the node's failure route.
- **One clock decides.** Every deadline is computed by the domain from the database clock when the transition that starts it
  happens, journaled, and compared only with the database clock; the lease decision refuses an attempt whose deadline has passed,
  so a lease and a timeout of the same attempt can never both commit.
- **A retry policy per failure code**, not only a list of codes that are never retried: `RATE_LIMITED` can get ten attempts
  spaced a minute apart while `ACTIVITY_ERROR` keeps three. A worker can also ask for a longer wait before the next attempt
  (`nextRetryDelay`, for a provider's `Retry-After`), as Temporal's `nextRetryDelay` does, never shorter than the author's.
- **Two kinds of heartbeat.** `host` detects a lost process without one line of handler code; `handler` detects a stuck handler,
  which is Temporal's meaning.
- **The journal says which deadline ended an attempt** (`deadline: startToClose`, `scheduleToClose` or `heartbeat`), and the Studio
  shows the deadlines of a step before they pass.

The rules are [ADR-0110](../adr/ADR-0110-step-timeouts-are-deadlines-on-the-database-clock-and-retry-is-decided-per-failure-code.md),
proposed with this design. The register of decisions, gaps and questions is `specs/045-activity-timeouts/research.md` (Decision
045-001 onward). The principles cited by numeral are those of [the constitution](../constitution.md).

### Terms

- **Attempt; fence; generation.** An attempt is one lease of a step and what follows it, numbered from 1 (`steps.attempt`). Each
  lease also raises the step's generation (`steps.generation`); the pair (attempt, generation) is the lease's fence, which every
  report must carry, so that a stale report is refused (constitution principle I).
- **Withdrawal; provisional lease.** A lease the server never handed to the transport is withdrawn: the step goes back to `READY`
  and no attempt is spent ([Architecture decision 0098](https://github.com/ortisun/sagat-octopus-orchestrator/blob/bugfix/withdraw-undelivered-leases/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md),
  pull request #260, not merged). At protocol 1.8 a lease is provisional until its host acknowledges it, and a provisional lease
  that reaches its earlier deadline is withdrawn the same way; a lease is made provisional only when its earlier deadline (expiry or
  attempt deadline) is at least 18 s after the claim, three receive bounds of 6 s, the longest a conformant host may take to read a
  lease ([Architecture decision 0099](https://github.com/ortisun/sagat-octopus-orchestrator/blob/docs/delivery-acknowledgement/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md),
  pull request #264, accepted by the owner on 2026-11-21, not merged). `TASK_NOT_STARTED` is the code with which a 1.8 host hands
  back a lease it did not start.
- **Release.** The transition that first makes a step due to run: `step.released`, or, for a node with a concurrency or rate gate
  of feature 022, the moment it starts waiting at the gate.
- **Claimable.** A step is claimable when it is `READY`: released and past any gate (Architecture decision 0089), a retry whose
  backoff has passed (`step.retry_due`), or a step an operator asked to retry (`step.retry_requested`).
- **Live attempt.** An attempt whose lease is committed: `LEASED` or `RUNNING` (the first heartbeat moves `LEASED` to `RUNNING`
  and journals `step.started`).
- **The database clock; `db_now`.** A run transaction reads the database's time once, under the run lock (`db_now`,
  `clock_timestamp()` in `crates/infra-postgres/src/execution.rs`); the domain uses it as the transition's time. Scans compare with
  the database's `now()`. No process clock decides a deadline.
- **The run lock, the chain lock.** A run transaction locks the run's continuation chain, then the run's row, and holds both until
  it commits; every transition of the run's steps (a lease, a report, a timeout, a retry) happens under them. A delivery
  acknowledgement and a binding revocation write lease rows without the run lock, under their own row and advisory locks.
- **Shard owner; window; pass.** Each run belongs to a shard; the scheduler process that holds a shard's lease (its owner) runs
  passes over that shard, each reading a bounded window of candidates fairly across tenants (`pick` in
  `crates/scheduler/src/passes.rs`) and re-checking each under the run lock before acting.
- **Root, scoped and path steps.** A root step is in the run's top graph; a scoped or path step is an instance inside a collection
  or loop body, written by the scoped statement of a full-run transaction or by the path statement `upsert_path_instance`.
- **Start deadline** (`steps.start_deadline_at`): claimable time plus `scheduleToStartMs`. **Close deadline**
  (`steps.close_deadline_at`): release time plus `scheduleToCloseMs`. **Close floor**: the earlier of the node's `timeoutMs` and
  6,000 ms (one receive bound); an attempt is not started with less than this left before the close deadline. **Attempt deadline**
  (`steps.attempt_deadline_at`, exists): the earlier of the claim's time plus `timeoutMs` and the close deadline. **Lease expiry**
  (`steps.lease_expires_at`, exists): the claim's or the last renewal's time plus the node's `heartbeatTimeoutMs` (or the platform's
  30 s); a renewal never moves it past the attempt deadline.
- **Outcome-unknown codes.** The seven codes for which `FailureCode::leaves_outcome_unknown` holds (`crates/domain/src/execution/event.rs`):
  `LEASE_EXPIRED`, `ATTEMPT_TIMEOUT`, `OUTCOME_UNKNOWN`, `STORAGE_QUOTA_EXCEEDED`, `RECONCILIATION_FAILED`,
  `RECONCILIATION_UNKNOWN`, `RECONCILIATION_NOT_FINAL`. This design adds none. It adds two known-failure codes,
  `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT` (Technical Plan 12).

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

## Requirements

### Functional

- **Requirement 045-001 — Timeout fields.** An `activity` or `script` node, in the root graph (`$defs/activity`, `$defs/script`)
  and in a body (`$defs/bodyActivity`, `$defs/bodyScript`), MAY declare `scheduleToStartMs` and `scheduleToCloseMs` (each an
  integer from 1,000 to 31,622,400,000, that is one second to 366 days), `heartbeatTimeoutMs` (10,000 to 86,400,000, ten seconds
  to 24 hours) and `heartbeatFrom` (`host`, the default, or `handler`). `timeoutMs` stays optional. Publication MUST refuse, with
  the JSON pointer of the field: a value out of range; `heartbeatFrom: "handler"` without both `heartbeatTimeoutMs` and
  `scheduleToStartMs`; and, between fields that are both declared, `heartbeatTimeoutMs` greater than `timeoutMs` or than
  `scheduleToCloseMs`, `timeoutMs` greater than `scheduleToCloseMs`, `scheduleToStartMs` greater than `scheduleToCloseMs`. A
  compensation, a reconciliation or any other kind of node MUST NOT declare them. *Acceptance:* a publication test per refusal and
  per accepted combination (`integration_gitops_versioning`), the IR schema test, and the Studio's form test.
- **Requirement 045-002 — Retry fields.** A `retry` (`$defs/retry`, `$defs/bodyRetry`, and so a `compensationRetry`) MAY declare
  `nonRetryableErrorCodes` (1 to 32 distinct codes) and `byErrorCode` (1 to 16 entries, each with `codes`, 1 to 32 distinct codes,
  and any of `maxAttempts`, `initialDelayMs`, `maxDelayMs`, `strategy`, `jitter` with the meaning and bounds they have in `retry`).
  A code matches `^[A-Z0-9_]{1,64}$`, the grammar every SDK host sends (`sdks/SPEC.md` §6.5); it appears at most once across both
  lists; the lists name at most 64 codes in all; none is one the platform decides elsewhere: the seven outcome-unknown codes,
  `CANCELLED`, `TASK_NOT_STARTED`, `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT`; and the effective policy of each
  entry (the base overlaid by the entry) MUST be a valid policy (its initial delay not above its maximum delay). *Acceptance:* a
  publication test per refusal (`RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`, `RETRY_CODES_TOO_MANY`, `RETRY_OVERRIDE_INVALID`).
- **Requirement 045-003 — Schedule-to-start.** When an attempt of a node with `scheduleToStartMs` becomes claimable, the domain
  MUST compute its start deadline from `db_now` and record it in the event that makes it claimable. The deadline MUST stay until the
  next attempt becomes claimable or the step ends: a lease and a withdrawal do not change it. The lease decision MUST NOT lease an
  attempt whose start deadline is at or before `db_now`. Once it has passed, the step MUST end `FAILED` with
  `SCHEDULE_TO_START_TIMEOUT`, which is not retried (no lease of the attempt ever started work: either none was committed, or each
  was withdrawn as never started) and takes the node's failure route. *Acceptance:* the claim tests of Technical Plan 7 and the
  withdrawal test.
- **Requirement 045-004 — Schedule-to-close.** The close deadline MUST be computed and recorded at the step's release and kept by
  every later attempt. An attempt MUST NOT be leased, and a retry MUST NOT be scheduled, with less than the close floor left before
  it. Reached while no attempt is live, it MUST end the step `FAILED` with `SCHEDULE_TO_CLOSE_TIMEOUT`, a known failure: by the
  deadline pass for a `READY` step, by the retries pass for a `RETRY_WAIT` step, and by feature 022's timeout routine for a step
  waiting at a gate. An attempt's deadline MUST be the earlier of its claim time plus `timeoutMs` and the close deadline; when the
  close deadline ends a live attempt, the attempt ends `ATTEMPT_TIMEOUT` with `deadline: scheduleToClose`, and the outcome-unknown
  rule applies. The close deadline MUST be cleared when the step goes to reconciliation, to review or to an operator hold, and by an
  operator's retry: from then on a person or a reconciliation decides, not the budget. *Acceptance:* the tests of Technical Plan 7,
  one per status in which the deadline can be reached, and one per transition that clears it.
- **Requirement 045-005 — Heartbeat timeout.** A lease of a node with `heartbeatTimeoutMs` MUST be granted and renewed with that
  time-to-live instead of the platform's; a renewal never moves the expiry past the attempt deadline. An attempt that lapses MUST end
  with `LEASE_EXPIRED`, as today, and its event MUST record `deadline: heartbeat` when the node declares the field. *Acceptance:*
  the lapse test per node kind, with and without the field.
- **Requirement 045-006 — The handler as the heartbeat's source.** A node with `heartbeatFrom: "handler"` MUST be leased only to a
  session accepted at protocol 1.9 or later. Such a host MUST send the first beat itself when the handler starts and afterwards
  renew the lease only when the handler called `heartbeat()` since its last renewal. *Acceptance:* the claim test with a 1.8 and a
  1.9 session, and the conformance scenario `heartbeat-from-handler` in six languages.
- **Requirement 045-007 — The retry decision.** When an attempt of a root, scoped or path step ends with a report, one decision
  function MUST decide, in this order: an outcome-unknown code goes to reconciliation or review (feature 037, unchanged); a step
  asked to stop is cancelled (unchanged); `retryable = false` from the worker is final; a code in `nonRetryableErrorCodes` is final;
  the effective policy (the base overlaid by the `byErrorCode` entry that names the code, if any) is spent when the step's attempt
  count is at its `maxAttempts`; another attempt is scheduled only when its time is before the close deadline less the close floor,
  and otherwise the step fails `SCHEDULE_TO_CLOSE_TIMEOUT` with the last attempt's code recorded. *Acceptance:* the decision-table
  unit test (every row of Technical Plan 6), for a root step, a scoped instance and a path instance, and the conformance scenario
  `retry-by-code`.
- **Requirement 045-008 — A worker's delay.** `FailStep.next_retry_delay_ms` MUST replace the computed delay of the next attempt,
  without jitter, clamped to at least the effective policy's initial delay and at most 86,400,000 ms, when the decision of
  Requirement 045-007 schedules one; the event MUST record `delaySource: worker`; it MUST NOT add an attempt or pass the close
  deadline; it is part of the report's idempotency fingerprint; a session below 1.9 that sends it is refused `INVALID_ARGUMENT`.
  *Acceptance:* unit tests and the conformance scenario `retry-by-code`.
- **Requirement 045-009 — Deadlines are acted on by a pass on the database clock.** A scheduler pass of its own, `step_deadlines`,
  MUST end `READY` steps whose start deadline, or close deadline less the close floor, has passed: on its shard, through `pick` with
  the tenant exclusion and its own window, skipping paused and stopping runs, and re-checking under the run lock both the shard
  fence and the step (status, generation, deadline) in the transaction that ends it. A pass that crashes, or two passes, MUST end a
  step at most once. *Acceptance:* the crash and duplicate tests of `fault_activity_timeouts`.
- **Requirement 045-010 — Races.** Each race of Technical Plan 7 MUST resolve as its row says, and the loser MUST be a no-op that
  leaves evidence (a refusal for a caller, a counted no-op for a pass). *Acceptance:* one barrier test per row, with held locks,
  never with sleeps (AGENTS.md rule 18).
- **Requirement 045-011 — Gates.** A step waiting at a gate of feature 022 MUST have as its waiter deadline the earlier of the gate's
  queue deadline and the close deadline, so that 022's one timeout routine (Architecture decision 0089 decision 5) ends it, with
  `SCHEDULE_TO_CLOSE_TIMEOUT` when the close deadline was the earlier; a waiter past its close deadline is not eligible for a grant;
  a granted step ended by either deadline before its lease MUST return its slot and permit as feature 022 specifies.
  *Acceptance:* the gate tests of Technical Plan 7, built with whichever of 022's gate and this design lands second.
- **Requirement 045-012 — Compensation and reconciliation keep their rules.** The new timeouts MUST NOT apply to a compensation or
  a reconciliation node, and the close deadline does not bound a reconciliation; `compensationRetry` gains only the retry fields.
  *Acceptance:* a test that a saga's compensation and a reconciliation are unaffected by a forward node's `scheduleToCloseMs`.
- **Requirement 045-013 — Journal and replay floor.** The fields this design adds to events are optional; an event written before it
  decodes as before; a run's replay MUST equal its projection with and without them; the domain computes each deadline (truncated to
  microseconds) and the store writes exactly the event's value. A build without the capability MUST NOT load a journal that holds
  this design's transitions (Requirement 045-017). *Acceptance:* the replay-equals-projection corpus of
  `integration_activity_timeouts`, comparing the columns themselves, and the registry test of the replay floor.
- **Requirement 045-014 — Visible deadlines.** The REST view of a step and the Studio MUST show its start, close and attempt deadlines
  and the cause of the attempt deadline, and a timeout's code with its meaning; the time left is computed against the server time
  the view carries, never the browser's clock. *Acceptance:* `contract_rest_durable_execution` and the Studio's run-view tests.
- **Requirement 045-015 — SDK builders.** The six SDKs' workflow builders MUST offer the fields of Requirements 045-001 and 045-002,
  validated as the server validates them, and the SDKs MUST let a worker map exception classes to failure codes at registration,
  refusing a code outside the list grammar. *Acceptance:* each SDK's unit tests and the IR conformance scenarios.
- **Requirement 045-016 — SDK hosts at 1.9.** A host accepted at 1.9 MUST beat at a third of `heartbeat_timeout_ms`, within today's
  bounds of 500 ms and 10 s, honour `heartbeat_from_handler`, and send `next_retry_delay_ms` when the handler asks. *Acceptance:*
  each SDK's unit tests and the conformance scenarios `activity-timeouts` and `heartbeat-from-handler`.
- **Requirement 045-017 — Fleet switches and runs in flight.** A version that uses any field of this design except the handler
  source MUST publish only once the operator recorded the requirement `activity-timeouts-v1`; one that uses `heartbeatFrom:
  "handler"` also needs `activity-timeouts-handler-v1`, which only a build that offers protocol 1.9 lists. Publication holds the
  requirement for its transaction, so a removal cannot slip between. A requirement MUST NOT be removed while any run journal that is
  not purged holds a transition of this design (the replay floor). A version published before MUST behave for every run exactly as
  today. *Acceptance:* the registry tests and the released 0.2 scenario of Technical Plan 13.
- **Requirement 045-018 — Telemetry and audit.** The metrics, spans and audit records of Technical Plan 11, with closed labels and no
  tenant, namespace, run identifier or worker-sent code as a label or in a log. *Acceptance:* the metrics text test of
  `integration_activity_timeouts`.
- **Requirement 045-019 — Only the platform reports a timeout.** A worker's report with `SCHEDULE_TO_START_TIMEOUT` or
  `SCHEDULE_TO_CLOSE_TIMEOUT` MUST be recorded as `WORKER_ERROR`, and timeouts MUST be counted from the passes, never from a code.
  *Acceptance:* a worker-report test per code.
- **Requirement 045-020 — Reset and operator actions.** A reset (feature 018) MUST recompute, from the reset's own time, the deadlines
  of the steps it makes claimable or releases, instead of copying the source's instants; an operator's retry MUST write a new start
  deadline and clear the close deadline. *Acceptance:* a reset-registry test and an operator-retry test past the close deadline.

### Non-functional

- **Non-functional requirement 045-001 — No cost on the hot path.** A lease stays 38 statements in 2 transactions and a completion 50
  statements in 1 (the lines of `tests/integration/claim-transition-efficiency/work_budget.rs`); the new columns are written by
  the statements that write the step today; the claim's new conditions are on the rows its candidate query already reads; a budget
  line covers a poll with rows it cannot claim ahead of claimable work.
- **Non-functional requirement 045-002 — Lag.** A deadline is acted on by a pass of its shard owner. The bound follows from the
  scheduler's defaults (`crates/scheduler/src/passes.rs`: 200 candidates per pass and tick, a pause of 500 ms between ticks only when
  a tick found nothing): 1,000 deadlines due in one second need at most 1,000 / 200 = 5 ticks, so at most 5 × 500 ms = 2,500 ms of
  pauses plus the work of the 1,000 run transactions. Criterion, measured on the CI machine and reported with its environment
  (constitution, Delivery Constraints): every one of 1,000 steps ended within 5 s of its deadline, the 99th percentile under 3 s; and,
  while one tenant floods 100,000 such deadlines, another tenant's 1,000 keep that criterion.
- **Non-functional requirement 045-003 — Isolation.** No new table; the new columns are on `steps`, read and written under the run's
  tenant as every other column of the row; one new grant, `EXECUTE` on the function that holds a requirement for a publication
  (Technical Plan 13).
- **Non-functional requirement 045-004 — Compatibility.** Protocol 1.9 is additive; a host at 1.0 to 1.8 is served as today; the
  IR grows by optional fields only; the journal's new fields are optional.

### Success criteria

- **Success criterion 045-001.** A step with `scheduleToStartMs: 5000` and no worker registered ends `SCHEDULE_TO_START_TIMEOUT` and
  takes its `onError` route, and a worker that registers after the deadline is never leased it.
- **Success criterion 045-002.** A step with `scheduleToCloseMs: 60000` whose attempts fail with a retryable code never starts an
  attempt with less than its close floor left and ends `SCHEDULE_TO_CLOSE_TIMEOUT` with the last code recorded.
- **Success criterion 045-003.** A handler that stops calling `heartbeat()` under `heartbeatFrom: "handler"` loses its lease within
  `heartbeatTimeoutMs` plus one beat interval of its last call, plus one pass, and its step waits for reconciliation or review, in
  six languages.
- **Success criterion 045-004.** A failure whose code is in `nonRetryableErrorCodes` is never retried; a step whose failures carry a
  code with a `byErrorCode` entry gets attempts until the step's count reaches that entry's `maxAttempts`.
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

- Automatic retry after a deadline that ends a live attempt. Constitution principle III forbids it; `reconcileWith` (on an activity)
  is the way to make such a step go on by itself, and a qualified safe-repeat (Architecture decision 0076) is not in this design.
  A `script` node cannot declare `reconcileWith`, so its timed-out attempt goes to review.
- Timeouts on compensation, reconciliation, child, wait or approval nodes (each has its own deadline today), and on the managed
  database and lake nodes, whose runners are platform processes with their own limits (Gap 045-003).
- Patterns or prefixes in the retry lists (Alternatives Considered).
- **Buffered signals.** The roadmap assessment also names "buffered signals (an early signal is refused today)". Feature 018 does
  not cover them (its spec lists them under Out of scope, on Architecture decision 0050 decision 4 and Architecture decision 0030
  decision 17). They share no mechanism with this design (no timer, no lease, a fleet switch of their own) and belong to a design of
  their own, which design 044 (workflow as code) will depend on. [ADR-0111](../adr/ADR-0111-a-version-may-declare-buffered-signals-kept-in-a-bounded-per-run-inbox.md)
  is its proposal (Decision 045-009).
- Workflow as code: design 044 decides how its activity calls carry these options.
- Heartbeat details: feature 019 (Technical Plan 5 says how they fit).

## Open Questions

- Whether a step's start deadline should also be offered to 019's priority order as an urgency hint (Gap 045-002); not in 0.3.0.
- Whether the minimum heartbeat timeout should be 18,000 ms, so that every lease can be provisional under protocol 1.8 (Gap 045-004);
  the measurement of Task 045-17 decides.

## Owner decisions needed

None open. The owner's decisions this design rests on: Owner decision 2026-11-24-1 (the scope), Owner decision 2026-11-20-6 and
Owner decision 2026-11-24-4 (the delegation of its questions, of its architecture decision and of its approval), Owner decision
2026-11-15-8 (the register of protocol minors). Every question below was decided under that delegation, delegate (Owner decision
2026-11-24-4), and is recorded with its options and reasoning in `specs/045-activity-timeouts/research.md`:

- Owner question 045-001: does a timeout that ends a live attempt retry, as in Temporal? No: outcome unknown (Decision 045-001).
- Owner question 045-002: is `SCHEDULE_TO_START_TIMEOUT` retried? No: final, failure route (Decision 045-002).
- Owner question 045-003: a new error-type field, or the existing `error_code`? `error_code` (Decision 045-003).
- Owner question 045-004: heartbeat from the host, from the handler, or both? Both, `host` by default (Decision 045-004).
- Owner question 045-005: a retry policy per code, or only a non-retryable list? Both (Decision 045-005).
- Owner question 045-006: which node kinds? `activity` and `script`, root and body (Decision 045-006).
- Owner questions 045-007 and 045-008 (buffered signals across continuation; opt-in buffering) moved with buffered signals to
  their own design (Decisions 045-007 and 045-008).
- Owner question 045-009: where do buffered signals belong? A design of their own (Decision 045-009).
- Decided during the review: an operator's retry clears the close deadline (Decision 045-018); a pause suspends the start deadline,
  not the close deadline (Decision 045-019); a reset recomputes the deadlines (Decision 045-020).

## Critical User Journeys (CUJs)

### CUJ 1: No worker takes the task

*   **Given**: a `chargeCard` activity with `scheduleToStartMs: 30000` and `onError: notifyOps`, and every payments worker down.
*   **When**: the step becomes `READY` and 30 s pass on the database clock.
*   **Then**: the next pass ends it `SCHEDULE_TO_START_TIMEOUT`, the run goes to `notifyOps`; a worker that comes back at second 31
    is not leased the attempt; the Studio showed the start deadline while it waited.

### CUJ 2: Retries that outlive their window

*   **Given**: `reserveSeat` with `scheduleToCloseMs: 120000`, `retry: {maxAttempts: 10, initialDelayMs: 5000, jitter: false}`
    and no `timeoutMs` (so its close floor is 6 s), and attempts failing with `UPSTREAM_BUSY`.
*   **When**: the fourth failure, at second 95, would schedule its retry 40 s later (5 s doubled three times), at second 135, which is
    after second 114 (the close deadline, 120 s, less the 6 s floor).
*   **Then**: no retry is scheduled; the step fails `SCHEDULE_TO_CLOSE_TIMEOUT` with `lastAttemptCode: UPSTREAM_BUSY`, and a saga
    around it compensates what completed.

### CUJ 3: A declined card is final, a rate limit is patient

*   **Given**: on a 1.9 host, `retry: {maxAttempts: 3, nonRetryableErrorCodes: ["CARD_DECLINED"], byErrorCode: [{codes:
    ["RATE_LIMITED"], maxAttempts: 10, initialDelayMs: 60000, maxDelayMs: 600000, strategy: "fixed"}]}`.
*   **When**: an attempt fails `RATE_LIMITED` with `nextRetryDelay` of 90 s, later another fails `CARD_DECLINED`.
*   **Then**: the first is retried after 90 s (the worker's delay, above the entry's 60 s), the second ends the step at once; the
    journal records which rule decided each and that the first delay came from the worker.

### CUJ 4: A stuck handler

*   **Given**: `exportReport` with `heartbeatTimeoutMs: 20000, heartbeatFrom: "handler", scheduleToStartMs: 300000`,
    `reconcileWith: findExport`, on a 1.9 host.
*   **When**: the handler deadlocks after its last `heartbeat()`.
*   **Then**: the host stops renewing; the lease lapses at most 20 s plus one beat interval after that call (`LEASE_EXPIRED`,
    `deadline: heartbeat`); `findExport` asks the provider what happened; the operator sees "outcome unknown: reconciling" and the
    deadline that passed.

## Technical Plan (Architecture)

### 1. The IR

Additive to `specs/001-platform-foundation/contracts/workflow.schema.json` (Architecture decision 0032). Properties added to
`$defs/activity`, `$defs/script`, `$defs/bodyActivity` and `$defs/bodyScript` (a fragment of their `properties`):

```json
"scheduleToStartMs": {"type": "integer", "minimum": 1000, "maximum": 31622400000},
"scheduleToCloseMs": {"type": "integer", "minimum": 1000, "maximum": 31622400000},
"heartbeatTimeoutMs": {"type": "integer", "minimum": 10000, "maximum": 86400000},
"heartbeatFrom": {"enum": ["host", "handler"]}
```

Properties added to `$defs/retry` and `$defs/bodyRetry`, which keep `additionalProperties: false` (a fragment):

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

The schema cannot express the cross-field rules of Requirements 045-001 and 045-002; `crates/infra-ir` checks them and refuses
with the field's pointer and a closed reason (`TIMEOUT_OUT_OF_RANGE`, `TIMEOUTS_CONTRADICT`, `HEARTBEAT_SOURCE_NEEDS_TIMEOUTS`,
`TIMEOUT_NOT_ALLOWED_ON_NODE`, `RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`, `RETRY_CODES_TOO_MANY`, `RETRY_OVERRIDE_INVALID`),
listed in `contracts/errors.md` of feature 001 by Task 045-1. Each effective policy is built through `RetryPolicy::new`, so the
domain's own invariant refuses an invalid one.

In the domain (`crates/domain/src/workflow.rs`), a leased node gains its timeouts (the two schedule durations, and the heartbeat
timeout with its source) and `RetryPolicy` gains its final codes and its overrides per code, with one function that answers, for a
code, "final" or the effective policy. No `serde_json` enters the domain (principle II); `infra-ir` builds the types.

### 2. The deadlines, their events and their columns

**One source.** The domain computes each deadline as `db_now` plus the node's duration, truncated to microseconds, and puts it in
the event of the transition that starts it; the reducer applies it; the store writes exactly the event's value into the column
(Requirement 045-013). A static check keeps the store from computing a deadline itself (Task 045-20).

| Deadline | Set by the events | Kept through | Cleared by |
|---|---|---|---|
| start (`steps.start_deadline_at`, `timestamptz NULL`) | `step.released` past its gates; `step.retry_due`; `step.retry_requested`; feature 022's grant events | the lease, a withdrawal, the attempt | the next attempt's event above (which replaces it); the end of the step |
| close (`steps.close_deadline_at`, `timestamptz NULL`) | `step.released`, or the start of its gate wait | every retry | the end of the step; `step.reconciliation_requested`; `step.needs_review`; an operator's hold; `step.retry_requested` |

Two more columns: `attempt_deadline_cause` (`text NULL`, `CHECK` in `START_TO_CLOSE`, `SCHEDULE_TO_CLOSE`), written by the lease
with `attempt_deadline_at` (the earlier of `db_now` plus `timeoutMs` and the close deadline; a tie goes to `START_TO_CLOSE`); and
`min_protocol_minor` (`smallint NOT NULL DEFAULT 0`), 9 for `heartbeatFrom: "handler"`, written at the release, this design's own
need (Technical Plan 5). Each is written by the three statements that write a step today (the root statement, the scoped statement
and `upsert_path_instance` in `crates/infra-postgres/src/execution.rs`), so no statement is added.

Two migrations in `crates/infra-postgres/migrations/` (the checklist of `.claude/skills/migration`: additive, grants for the
runtime role, a down migration that refuses data loss, every variable-size column classified in
`crates/infra-postgres/storage-classification.json`, the privilege audit): the first adds the columns, the `CHECK` constraints
`NOT VALID`, the two capabilities to the registry's closed capability lists, the publication-hold function, and the two indexes;
the second validates the constraints, as design 043 did. The down migration refuses while a column holds a non-default value. The
indexes, shaped like the lost-lease indexes, with the new migration's own check that refuses an index a failed concurrent build left
invalid (the applied migration `20261017000000_scheduler_shards.up.sql` is not edited):

```sql
CREATE INDEX steps_start_deadline_shard_idx ON steps (run_shard(run_id), start_deadline_at)
    WHERE status = 'READY' AND start_deadline_at IS NOT NULL;
CREATE INDEX steps_close_deadline_shard_idx ON steps (run_shard(run_id), close_deadline_at)
    WHERE status = 'READY' AND close_deadline_at IS NOT NULL;
```

Only `READY` rows are indexed: a `RETRY_WAIT` step is ended by the retries pass, a gated step by feature 022's timeout routine, and
a step waiting for reconciliation, review or an operator has no close deadline.

### 3. The claim and the start deadline

**The lease decision is the domain's.** The lease commands (`lease_step`, `lease_instance`, `lease_path_instance` in
`crates/domain/src/execution/command.rs`) take `db_now` and the session's accepted minor, and skip, without aborting the run's other
leases, a step whose start deadline is at or before `db_now`, whose close deadline less its close floor is, or whose
`min_protocol_minor` is above the session's. The claim under the run lock (`claim_root_run` and the path claim in
`crates/application/src/worker.rs`) leases only what the domain allows. **The candidate query carries the same conditions as a
hint**: the four branches of `claimable_query!` (`crates/infra-postgres/src/workers.rs`), `CLAIMABLE`, `CLAIMABLE_PATHS` and
`RUN_FOR_CLAIM` add them on the rows they already read, with the database's `now()` (the precedent is the `$5 >= 5` branch of
Architecture decision 0032 decision 5), so unclaimable rows do not fill the candidate window of a poll. A parity test runs the
candidate conditions and the domain's decision over the same rows (Task 045-20). No statement is added (Non-functional requirement
045-001).

**The pass.** `step_deadlines` (its own pass, beside `leases` and the run-level `deadlines` of `crates/scheduler/src/passes.rs`) reads
through `pick`, with the tenant exclusion, the per-shard limit and its own window, the rows of `steps_start_deadline_shard_idx` whose
deadline is at or before `now()` and of `steps_close_deadline_shard_idx` whose deadline is within 6,000 ms of `now()` (the largest
close floor), excluding paused and stopping runs as `due_start_releases` does. For each it opens the run's transaction and checks
two things under the lock: the shard fence (`View::holds`) and the step (still `READY`, the generation read, the deadline passed
against `db_now`, with the node's exact floor). If both hold, the domain ends the step `FAILED` with no lease fence, guarded by the
generation; if the start and the close deadline have both passed, the earlier one names the code, and a tie goes to
`SCHEDULE_TO_CLOSE_TIMEOUT`. If not, the pass counts a no-op. A scoped or path step is ended in its path transaction with the
scoped-protocol filter the `leases` pass uses. A run with a stop reason is left to the stop path, which cancels the step.

**A withdrawal** (Architecture decision 0098 decision 3, Architecture decision 0099 decision 5) sets the step back to `READY` without
spending an attempt and keeps its start deadline: the withdrawn lease never started, so the attempt is the same attempt. If the
deadline has passed, the pass ends the step: nothing ran, so the failure is known.

**A pause** (Decision 045-019). The pass skips a paused run. The resume (`run.resumed`) moves each claimable step's start deadline
later by the time the run was paused (from the `run.paused` event's time to `db_now`), recorded in the resume event: a pause is an
operator's hold, not a missing worker. The close deadline is not moved, because it is the author's business window; it is enforced
again from the resume.

### 4. The close deadline

| Status when it is reached | What happens |
|---|---|
| `READY` | the lease decision refuses the step once less than the close floor is left; the `step_deadlines` pass ends it `SCHEDULE_TO_CLOSE_TIMEOUT` |
| `RETRY_WAIT` | a retry is never scheduled with less than the floor left (Technical Plan 6, row 6); if the retries pass finds a due retry past that point (when it lags), it ends the step `SCHEDULE_TO_CLOSE_TIMEOUT` instead of making it claimable |
| `WAITING` at a gate of feature 022 | the waiter's deadline (`deadline_at` of `concurrency_waiters` and of the rate-limit queue, `specs/022-flow-control-step-policies/data-model.md`) is written as the earlier of the gate's queue deadline and the close deadline, with which one it is; 022's one timeout routine (Architecture decision 0089 decision 5) ends the step, with `SCHEDULE_TO_CLOSE_TIMEOUT` when the close deadline was the earlier; a waiter past its close deadline is not eligible under decision 4 |
| `READY` after a gate's grant, not yet leased | the pass ends it, and the transaction returns its slot (`rate_limit.returned`) and its permit (`concurrency.released`) in feature 022's lock order, as 022 specifies for a granted step that leaves `READY` other than by a lease |
| `LEASED` or `RUNNING` | the attempt deadline is the close deadline: the lost-lease pass ends the attempt `ATTEMPT_TIMEOUT` with `deadline: scheduleToClose`; a provisional lease at that deadline is withdrawn (Architecture decision 0099 decision 5) and the step, now `READY` past its close deadline, fails `SCHEDULE_TO_CLOSE_TIMEOUT` |
| `WAITING` for reconciliation, review or an operator | not reached: the transition into it cleared the close deadline (Requirement 045-004) |

What follows a live attempt's timeout is feature 037's, unchanged: the reconciliation's answer `applied` completes the step;
`not_applied` and `unknown` are not final and wait for a person; a person may resolve the step or retry it, and an operator's retry
opens a new attempt whose deadline is its claim time plus `timeoutMs` only (Decision 045-018).

Whichever of feature 022's gates (specified and being built; `concurrency_waiters` is not in `crates/` yet) and this design is built
second adds the gate rows of this table and their tests (Task 045-10). This refines Architecture decision 0089 decisions 4 and 5
(ADR-0110).

The ending events of a live attempt that a deadline ended (`step.failed`, `step.reconciliation_requested`, `step.needs_review`)
gain an optional `deadline` field (`startToClose`, `scheduleToClose`, `heartbeat`), written only when the node declares a field of
this design: from `attempt_deadline_cause` for an attempt timeout, `heartbeat` for a lapse of a node with `heartbeatTimeoutMs`.

### 5. The heartbeat timeout and its source

The lease's time-to-live becomes per node: the lease grant and the renewal (`crates/application/src/worker.rs`, the claim and
`heartbeat`) take the node's `heartbeatTimeoutMs` when it is declared and `settings.lease_ttl` otherwise; a renewal is capped at the
attempt deadline, as today (a fresh grant is not; the attempt-deadline row of the lost-lease pass ends the attempt first). The
lost-lease pass ends a lapsed attempt with `LEASE_EXPIRED`, as today, and records `deadline: heartbeat` when the node declares the
field; when the expiry and the attempt deadline have both passed, the attempt timeout is reported, as today
(`crates/infra-postgres/src/maintenance.rs` reads it first). A renewal that commits before the pass's transaction saves the attempt, as today: the pass re-checks the expiry under the
run lock. No outcome-unknown code is added, so the list and its two copies (`crates/infra-postgres/src/chain_progress.rs`,
`frontend/apps/studio/src/features/runs/ops.ts`) do not change.

**The minimum, 10,000 ms.** A host may take up to one receive bound (6,000 ms) to read a lease and must commit its first heartbeat
within one heartbeat bound (2,000 ms) after that (`sdks/SPEC.md` §10, Architecture decision 0099 decision 6): 8,000 ms, below the
minimum. The cost: each beat is one run transaction; today a lease beats every 10,000 ms (a third of 30,000 ms, the cap), 0.1 beats
per second; at the minimum a 1.9 host beats every 3,333 ms, 0.3 beats per second, three times today's rate per lease at most, and
the number of leases is bounded by the activity's concurrency quotas. A lease under 18,000 ms is never provisional under protocol
1.8 (Architecture decision 0099 decision 2), so it does not get delivery acknowledgement's protection against a lease lost on the
wire; ADR-0110 states it, the Studio's form warns, and Gap 045-004 records the choice. At the other end, a `heartbeatTimeoutMs` of
24 hours lets a dead worker's attempt hold its step, and its concurrency slot, for that long: the author's choice, stated in the
Studio's form.

**The source.** `heartbeatFrom: "host"` keeps today's host loop: it beats while the handler runs. A host older than 1.9 computes its
interval as a third of the lease's time left at receipt, clamped to 500 ms and 10 s (`sdks/SPEC.md` §6.4), so it beats in time for
any `heartbeatTimeoutMs` from 10,000 ms, as long as its clock agrees with the database's (a host whose clock does not sets
`heartbeatIntervalMs`, the host setting §6.4 defines); a 1.9 host reads the duration itself (Technical Plan 8). `heartbeatFrom:
"handler"` needs a host that knows it: the release writes `min_protocol_minor = 9`, the lease decision leases the step only to a
session at 1.9 or later, and the required `scheduleToStartMs` ends the step if no such host comes. Such a host sends the first beat
itself when the handler starts, and afterwards sends a beat at its interval only if the handler called `heartbeat()` since the last
beat sent; calls between beats are coalesced, so at most one beat per interval reaches the server however often the handler calls.
A call can wait up to one interval (a third of the timeout) before its beat goes out, so a handler should call `heartbeat()` at least
every two thirds of `heartbeatTimeoutMs`, less a round trip; `sdks/SPEC.md` §6.4 says so (Task 045-1).
A host that stops beating also stops learning of a cancellation (a heartbeat's answer carries `cancel_requested`, §6.4); a
handler-sourced handler that wants to be cancellable calls `heartbeat()`.

**Feature 019's heartbeat details.** Feature 019 gives `heartbeat(details)` an argument whose details ride on the next scheduled
beat, from its own minor (its checkpoints, `HeartbeatRequest` fields 9 and 10). It is the same call: a call with details is a handler
heartbeat, and the coalescing keeps the latest details, as 019 says ("latest wins"). 019's detached heartbeat timeout is its own;
a detached attempt ignores `heartbeatTimeoutMs`, and its maximum duration is bounded by this design's attempt deadline.

### 6. The retry decision

One decision function, called by `end_attempt` (root steps) and `fail_scoped_attempt` (scoped and path instances) in
`crates/domain/src/execution/command.rs`, keeps their first two rules (unknown outcome; a stop) and replaces the third with this
table, evaluated top to bottom; the first row that applies decides, and the event records which (`retryRule`):

| Row | When | Decision | Journal |
|---|---|---|---|
| 1 | the code leaves the outcome unknown | reconciliation or review (feature 037) | as today |
| 2 | the step was asked to stop | `step.cancelled` | as today |
| 3 | the worker said `retryable = false` | final | `step.failed`, `retryRule: worker` |
| 4 | the code is in `nonRetryableErrorCodes` | final | `step.failed`, `retryRule: nonRetryableCode` |
| 5 | the step's attempt count is at or above the effective `maxAttempts` | final | `step.failed`, `retryRule: attemptsSpent` |
| 6 | the next attempt's time (`db_now` plus the delay) is at or after the close deadline less the close floor | final, code `SCHEDULE_TO_CLOSE_TIMEOUT` | `step.failed`, `retryRule: closeDeadline`, `lastAttemptCode` |
| 7 | otherwise | retry | `step.retry_scheduled`, `retryRule: base` or `byErrorCode`, `delaySource: policy` or `worker` |

The effective policy takes each field from the code's entry when the entry names it and from the base policy otherwise. The step's
attempt count is shared by every code, and the exponent of an exponential delay is the step's attempt number: with a base
`maxAttempts` of 4 and an entry `{codes: ["RATE_LIMITED"], maxAttempts: 10}`, a step whose first three attempts failed with other
codes and whose fourth failed `RATE_LIMITED` is retried (4 is below 10), and has up to six more attempts while each fails with that
code. The delay is the worker's `next_retry_delay_ms` when it sent one, clamped to at least the effective initial delay and at most
86,400,000 ms, without jitter; otherwise the effective policy's delay with its jitter sample. A code outside the list grammar, which
only a client outside the six SDKs can send, matches no entry, and the base policy decides. A compensation's failure policy
(Architecture decision 0076) is applied after rows 3, 4 and 5, exactly as after a known non-retryable failure today.

The unhandled exception of an SDK is `ACTIVITY_ERROR`, which an author may name; Requirement 045-015 lets a worker map its own
exception classes to codes at registration (for example `CardDeclinedError` to `CARD_DECLINED`), so that an exception the handler
did not catch still reaches the policy with its own code; that is how a Temporal user's `nonRetryableErrorTypes` of class names is
written here.

The close deadline enters the domain at row 6, in the lease decision, and in the commands the two passes call; each compares
`db_now` with a deadline recorded in an event. A replay applies the journaled decisions; it never decides again, so the jitter
sample and the clock do not need to be reproduced.

### 7. The races (AGENTS.md rule 9)

Every row is decided under the run lock, which every transition of a step's status takes (constitution principle I); the loser
commits nothing or a counted no-op. A claim locks the run with `SKIP LOCKED`, so its tests are not "hold the lock and claim": each
claim row has three tests (no lock, deadline at `db_now` less and plus a margin; the claim's transaction held while the pass runs,
which then finds `LEASED`; the pass's transaction held while the claim runs, which then finds the step ended).

| Race | Resolution |
|---|---|
| a claim and the start deadline | the lease decision on `db_now`: before the deadline the lease commits and the pass's check finds `LEASED`; at or after, the decision skips it and the pass ends it |
| a claim and the close deadline | the same, with the close deadline less the close floor |
| two `READY` steps of one run, one of them past its deadline or needing minor 9 | the lease decision leases the other only |
| a completion and the attempt deadline (either cause) | first commit wins, as today; a late report is `STALE_ATTEMPT` |
| a renewal and the heartbeat timeout | first commit wins, as today |
| a failure's retry and the close deadline | decided in the failure's transaction (row 6), at the floor less one microsecond, at it, after it |
| the retries pass and the close deadline | the retries pass ends a `RETRY_WAIT` step past the floor instead of making it claimable |
| a gate's grant, the gate's queue timeout and the close deadline | one deadline per waiter, the earlier, in 022's one timeout routine; the grant refuses a waiter past its close deadline |
| a lease withdrawal and the start deadline | the withdrawal keeps the deadline; the pass ends the step if it passed |
| a pause and each deadline | the pass skips a paused run; the resume moves the start deadline; the close deadline is enforced from the resume |
| a stop and a deadline | the first commit; both end the step without an effect |
| a step in reconciliation past its close deadline | never ended by the pass: its close deadline was cleared |
| an operator's retry past the close deadline | the retry clears the close deadline and writes a start deadline; the attempt is leased |
| a crash of the pass, two passes | nothing committed is repeated: the check re-reads the shard fence, status, generation and deadline |

A lost broker hint does not delay a deadline: the pass reads the database, not the broker (constitution principle VII). A restart
loses nothing: every deadline is in the journal and in a column.

### 8. Protocol 1.9 and the field register

The register of `sdks/SPEC.md` §9 on `develop` stops at 1.7 (design 041); 1.8 is reserved for delivery acknowledgement by Owner
decision 2026-11-21-6, on the branch of pull request #264, not merged. This design names 1.9, the next free minor, and takes it
when its contract merges (Owner decision 2026-11-15-8). Minors are cumulative: a 1.9 host also carries 1.7 and the 1.8 host
procedure, so the SDK work of 1.9 follows the SDK work of 1.8 (Risks).

```protobuf
message TaskLease {
  // fields 1 to 18 unchanged; 19 to 22 named by draft 019; 23 and 24 by 1.8; 25 by draft 031
  // Added in 1.9. The lease's time-to-live that applies, the node's heartbeatTimeoutMs or the platform's: a duration, so that
  // the host's interval does not depend on its clock agreeing with the database's.
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

A server offers 1.9 on its build's proofs (the `protocol_ceiling` pattern of `crates/application/src/worker.rs`: a minor is offered
only when the build has the code and the evidence for it) and only when it offers 1.8. A 1.9 field is sent only to a session
accepted at 1.9; a session below it gets today's lease, and its `FailStep` with field 14 is refused `INVALID_ARGUMENT`.

### 9. The SDKs

In all six (TypeScript, Go, Python, C#, Java, Rust), with idiomatic names and one meaning (principle IV):

- **Builders.** `activity(...)` and `script(...)` options `scheduleToStart`, `scheduleToClose`, `heartbeatTimeout`, `heartbeatFrom`;
  retry options `nonRetryableErrorCodes`, `byErrorCode`. Each validates what the server validates (Requirements 045-001 and
  045-002) and raises the same reason before anything is sent.
- **Worker registration.** A map from exception class to failure code (TypeScript: a constructor; Go: an `errors.As` target;
  Python: a class; C#: a `Type`; Java: a `Class`; Rust: a function on the error), consulted before an unhandled error becomes
  `ACTIVITY_ERROR`; a code outside `[A-Z0-9_]{1,64}` is refused at registration. Local only: nothing on the wire changes.
- **Host at 1.9.** The interval is a third of `heartbeat_timeout_ms`, within 500 ms and 10 s; `heartbeat_from_handler` switches to
  the handler source of Technical Plan 5; `ActivityFailure` takes an optional `nextRetryDelay`, sent as `next_retry_delay_ms`
  (raised locally as `VALIDATION_FAILED` out of range).
- **Conformance** (`tests/contract/sdk-conformance/`): `activity-timeouts` (the three new timeouts, at 1.8 and at 1.9),
  `retry-by-code`, `heartbeat-from-handler` (1.9 only); the same scenarios in six languages (AGENTS.md rule 8).

### 10. REST and the Studio

`specs/001-platform-foundation/contracts/openapi.yaml`, additive: the step view gains `startDeadlineAt`, `closeDeadlineAt`,
`attemptDeadlineAt` and `attemptDeadlineCause`, and the run view's server time (`serverTime`), against which a client computes the
time left; the publication reasons of Technical Plan 1 are added to the problem body.

The Studio (`frontend/apps/studio/src/features/workflows/fields.tsx` and the run view in `features/runs/`): the four fields and the
retry lists in the inspector with the server's validation, and the two warnings of Technical Plan 5; on a step of a run, its
deadlines with the time left against the view's server time (the projection's lag shown, principle VI) and, for each timeout, one
sentence of what happened and what the platform does next ("No worker took the task in time: the step failed and its failure route
ran"; "The attempt lost its heartbeat: the outcome is unknown, so the platform is asking `findExport`"). Types follow `openapi.yaml`
(`npm run api && npm run check:api`).

### 11. Telemetry and audit

- Metrics, closed labels only: `step_timeouts_total{kind}`, counted by the passes, where `kind` is `schedule_to_start`,
  `schedule_to_close`, `start_to_close` (an attempt deadline), `heartbeat` (the lapse of a node's own `heartbeatTimeoutMs`) or `lease`
  (the lapse of the platform's time-to-live); `step_retry_decisions_total{rule}` with the seven rows' rules;
  `step_deadline_lag_seconds` (histogram: from the deadline to the pass's commit); `step_deadline_pass_noops_total{kind}`.
- Spans: the pass's transaction carries `sagat.deadline.kind`.
- Audit: publication of a version with these fields is audited as every publication. A timeout is a system transition, journaled and
  not audited, as a lease expiry is today.
- Logs: closed values only (a rule, a kind); a worker's code stays in the journal.

### 12. What this design adds to the journal and the codes

- **Two known-failure codes**, `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT`, in `FailureCode`, in
  `specs/001-platform-foundation/contracts/errors.md`, in `sdks/error-catalog.json` and in the Studio's sentences (Task 045-1). A
  worker cannot send them (Requirement 045-019).
- **Optional event fields**: the deadlines on `step.released`, `step.retry_due`, `step.retry_requested`, 022's grant events and
  `run.resumed`; `deadline`, `retryRule`, `lastAttemptCode` and `delaySource` on the events of Technical Plans 4 and 6.
- **New transitions of the reducer**: `step.failed` from `READY` and from `RETRY_WAIT` (no fence, guarded by the generation). An
  older build refuses them, which is why the requirement is a replay floor (Requirement 045-017).
- **Reset** (feature 018): the reset-rule registry (`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`) gains the
  rule that the deadline fields of a copied event are recomputed from the reset's time for the steps the new run makes claimable or
  releases (Decision 045-020).
- Unchanged: the outcome-unknown list, the run-level deadline, the wait and approval timeouts, and every compensation and
  reconciliation rule.

### 13. Upgrade, rollback and runs in flight

- A version published before this design has none of the fields: every run of it behaves as today, at every minor, through every
  rolling upgrade (principle IV); no `deadline` field is written for it.
- The operator records `activity-timeouts-v1`, and `activity-timeouts-handler-v1` once every engine offers 1.9
  (`platform_requirement_activate`, Architecture decision 0082); until then publication of a version using a field answers `422`
  with the reason `CAPABILITY_NOT_ACTIVE`. Once recorded, a binary without the capability serves nothing (the closed gate).
- **The publication hold.** The runtime role cannot lock the requirement epoch, so a `SECURITY DEFINER` function,
  `platform_requirement_hold(capability text)`, with `SET search_path = pg_catalog, pg_temp` and `EXECUTE` granted to `sagat_app`,
  takes the epoch `FOR SHARE` in the publication transaction; a removal waits for it. Feature 031 may use the same function.
- **The replay floor.** Removing either requirement is refused while any run journal that is not purged holds a transition of this
  design, as Architecture decision 0098 decision 8 does for its own.
- Released 0.2 scenario (`scripts/test-release-upgrade-profile.py`): runs started on 0.2 finish on 0.3 unchanged; a 0.2 SDK host is
  served at its minor; a version with `heartbeatFrom: "handler"` is never leased to it.

### 14. Other features

| Feature | Status on 2026-11-24 | What this design asks of it, or gives it |
|---|---|---|
| 006, protocol 1.8 (Architecture decision 0099) | accepted, on the branch of pull request #264, not merged | 1.9 follows 1.8 in the server and in each SDK; a withdrawal keeps the start deadline |
| lease withdrawal (Architecture decision 0098) | on the branch of pull request #260, not merged | the same withdrawal rule; its replay floor is the pattern of Technical Plan 13 |
| 018 (schedules, resets) | built | the reset-rule registry gains the deadline rule (Technical Plan 12) |
| 019 (worker deployments) | approved draft | its `heartbeat(details)` is a handler heartbeat; a detached attempt ignores `heartbeatTimeoutMs`; its priority order is unchanged by the start deadline (Gap 045-002); field numbers above |
| 020 (visibility) | approved draft | the deadlines, `attemptDeadlineCause` and the deadline causes are additive facts it may list |
| 022 (flow control) | specified, being built | the close deadline is one of a waiter's deadlines in its one timeout routine; a granted step ended by a deadline returns its slot and permit |
| 030 (cross-namespace services) | approved draft | its operation deadlines use the same names and clock (`scheduleToCloseMs`, `scheduleToStartMs`) |
| 031 (client-side encryption) | approved draft | `error_code` and `safe_message` are never encoded; it may use `platform_requirement_hold` |
| 037 (compensation safety) | built | unchanged |
| 044 (workflow as code) | being designed | the options of an activity call map to these fields |

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A Temporal user expects a timed-out attempt to retry | The Studio's sentence, the guide's comparison table and `reconcileWith` as the documented way to go on automatically |
| Protocol 1.8 slips, so 1.9 cannot ship | Everything but the handler source and the worker's delay works at 1.8 and below (an older host beats from the lease's expiry); `activity-timeouts-handler-v1` is listed only by a build that offers 1.9, so publication refuses `heartbeatFrom: "handler"` until then |
| Building two partial indexes on a large `steps` table holds a lock that blocks writes while it reads | The upgrade notes give the measured time on the scale fixture (Task 045-17); an operator with a very large table builds them `CONCURRENTLY` by hand before the upgrade, which the migration detects and skips, and refuses one left invalid |
| Security: a worker asks for a short delay to hammer a provider | The delay is clamped to at least the author's initial delay, and the journal says it came from the worker |
| Security: a worker reports a platform timeout code to claim that nothing ran | The server records it as `WORKER_ERROR`; timeouts are counted from the passes |
| Security: one tenant floods deadlines or short heartbeats | The pass is fair across tenants with its own window; the minimum heartbeat timeout bounds a lease to three times today's beat rate |
| Security: a requirement is removed while a publication that needs it commits | The publication holds the requirement in its transaction |
| A host clock far from the database's distorts an older host's interval | A 1.9 host uses the duration; an older host's interval is clamped as today |
| The deadline pass falls behind under load | Its lag is a histogram with an alert in the runbook, and its criterion includes a one-tenant flood |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Every deadline is computed by the domain, journaled, written to a column and acted on in one run transaction under the run lock; the lease decision and the pass's re-check make a lease and a timeout exclusive; the retry decision is journaled with its rule. The principle's audited no-op is for a timeout racing a signal, which this design does not change; here a late worker report loses as today (`STALE_ATTEMPT`, recorded with its trace) and a pass that loses is counted (`step_deadline_pass_noops_total`), since no caller made it |
| II. Modular domain, ports and adapters | Pass | New types in `crates/domain` without `serde_json`; parsing in `infra-ir`; SQL in `infra-postgres`; the pass in `scheduler`; no new crate edge (`scripts/check-architecture.py`) |
| III. External effects are at-least-once; sagas compensate | Pass | A deadline that ends a live attempt is an unknown outcome (reconciliation or review); only a deadline with no live attempt is a known failure; compensation and reconciliation nodes are untouched, and the close deadline is cleared when a person or a reconciliation takes over |
| IV. Compatibility is versioned and pinned | Pass | Optional IR fields, protocol 1.9 additive, optional journal fields, two fleet requirements with a replay floor; versions published before are unchanged; no code changes meaning |
| V. Code, tenant boundary and secrets are isolated | Pass | No code runs on the server; no new table; one grant of a definer function with a fixed search path; a worker's delay and codes are bounded and cannot claim a timeout |
| VI. Observability and audit are part of the feature | Pass | Closed-label metrics counted by the passes, spans, deadlines and their cause in the projection with its lag |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Deadlines do not depend on a broker hint |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Barrier tests per race, crash and duplicate pass tests, restart, lost hint, pause, conformance in six languages, contract tests, a parity test and a static check; gaps registered |
| Delivery Constraints | Pass | No new technology; single region; two migrations; the registry of Architecture decision 0082 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Four columns and two indexes on `steps` | A deadline must be a durable fact the claim and a pass can read on the database clock | Timer rows per deadline: one more row and statement per attempt on the hot path |
| `min_protocol_minor` on `steps` and in the candidate query | A handler-sourced heartbeat must never reach a host that cannot honour it, and such rows must not fill an older host's candidate window | Refuse to publish until every host is at 1.9: the server cannot know the hosts |
| `byErrorCode` beside `nonRetryableErrorCodes` | The owner's brief asks for a retry policy per error type; a rate limit and a bug need different patience, and a worker's delay cannot give a rate limit more attempts than a bug | Only the list: forces an author to choose between giving up and hammering a provider |
| `heartbeat_timeout_ms` on the lease although an older host derives its interval from the expiry | The expiry is an instant on the database's clock; a duration lets a host beat in time whatever its clock says, as protocol 1.8 chose for its own fields | The instant alone: a host whose clock is off by more than the window beats too late |
| A second requirement for the handler source | Its steps can only run on 1.9 hosts, which need 1.8 first | One requirement: a handler-sourced version could publish before any server offers 1.9 and never be leased |
| A definer function for the publication hold | The runtime role cannot lock the requirement epoch, and without the hold a removal can slip between a publication's read and its commit | No hold: an old binary could serve a version it cannot honour |

## Alternatives Considered

- **Temporal's semantics (retry after every timeout).** Rejected: it repeats an effect whose outcome is unknown, which principle III
  and FR-037-009 forbid (ADR-0110, option 2).
- **Timer rows for deadlines.** Rejected: one more row and one more statement per attempt on a hot path that design 038 measured
  (ADR-0110, option 3).
- **A new `HEARTBEAT_TIMEOUT` code.** Rejected: it would mean what `LEASE_EXPIRED` means, grow the outcome-unknown list and its copies,
  and give every consumer two codes to match; the `deadline` field says which bound passed (ADR-0110, option 4).
- **A new error-type field.** Rejected: `error_code` already is the type; two names for one fact would fork what six SDKs send
  (ADR-0110, option 5).
- **Patterns in the retry lists.** Left out: exact codes keep the decision auditable; addable later without breaking a version
  (ADR-0110, option 6).
- **Only a non-retryable list, or only per-code policies.** Rejected: the list is what a Temporal user knows; the per-code policy is
  what a rate limit needs (Decision 045-005).
- **A heartbeat from the host only, or from the handler only.** Rejected: the host alone cannot see a stuck handler; the handler
  alone would break every existing handler that never calls `heartbeat()` (Decision 045-004).
- **Schedule-to-start counted from the release, gates included.** Rejected: a gate is a deliberate hold by the platform, and
  schedule-to-start exists to detect a missing worker; the close deadline covers the gate's time.
- **Heartbeat timeout as a second clock beside the lease.** Rejected: two expiries for one attempt would race each other; the lease
  expiry is the heartbeat timeout.
- **The close deadline as a second exit for a gate waiter.** Rejected after the architecture review: Architecture decision 0089 has
  one timeout routine per waiter, and two exits orphan waiter rows; the close deadline joins the waiter's deadline instead.
- **An attempt's time left on every lease (`attempt_time_left_ms`).** Dropped: no requirement needs it; protocol 1.8 already gives
  it to a flagged lease.

## Non-Functional Requirements

Non-functional requirements 045-001 to 045-004 are stated under Requirements. Their tests: the budget lines of
`integration_claim_transition_efficiency` (unchanged, plus the line with unclaimable rows) for 045-001; the lag measurement of
Task 045-17 for 045-002; the privilege audit of `integration_platform_foundation` and the suites as the runtime role for 045-003; the
released 0.2 scenario and the protocol baseline test for 045-004.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Test first: each task that changes behaviour adds its tests and sees them fail for the right reason before the code
(AGENTS.md rule 18). Documentation first: Task 045-1 changes the contracts and documents before any code (Owner decision
2026-11-20-8). The tasks are ordered into slices for two builders whose lanes never touch the same crate at once:

| Slice | Lane | Tasks | Crates and folders | Depends on |
|---|---|---|---|---|
| 0 | either | 045-1 | `specs/001-platform-foundation/contracts/`, `specs/018-schedules-bulk-operations/contracts/`, `sdks/SPEC.md`, `sdks/error-catalog.json`, `docs/` | this design approved |
| 1 | server | 045-2, 045-3, 045-4 | `crates/domain` | slice 0 |
| 2 | server | 045-5, 045-6, 045-7 | `crates/infra-ir`, `crates/infra-postgres` | slice 1 |
| 3 | server | 045-8, 045-9, 045-11, 045-20 | `crates/scheduler`, `crates/application`, `crates/api-http`, `apps/`, `scripts/` | slice 2 |
| 4 | server | 045-19 | `crates/api-grpc`, `crates/application` | slice 3; the server side of protocol 1.8 |
| 5 | server | 045-10, 045-16, 045-18 | `crates/application`, `crates/infra-postgres`, `tests/integration/activity-timeouts/`, `tests/fault/activity-timeouts/` | slice 3; 045-10 also on feature 022's gates |
| 6 | edges | 045-12 | `sdks/<language>` builders (six), `tests/contract/sdk-conformance/` scenarios | slice 0; its conformance run needs slice 3 |
| 7 | edges | 045-14 | `frontend/apps/studio` | slice 0 (types from `openapi.yaml`); its journeys need slice 3 |
| 8 | edges | 045-13 | `sdks/<language>` hosts (six) | slice 6, the 1.8 host work of feature 006 in each SDK, and slice 4 |
| 9 | either | 045-15, 045-17 | `docs/`, `tests/performance/` | slices 5 and 8 |

- [ ] T045-1 Write the contracts and documents first: the IR fields and their reasons on `activity`, `script`, `bodyActivity`,
  `bodyScript`, `retry` and `bodyRetry` (`workflow.schema.json`, `contracts/errors.md` of feature 001), the two codes
  (`contracts/errors.md`, `sdks/error-catalog.json`), protocol 1.9 (`worker.proto`, `worker.baseline.json` re-pinned after reading
  its diff), `openapi.yaml` (step deadlines, `serverTime`, the reasons), the journal fields and transitions (the event list of
  `specs/001-platform-foundation/contracts/`), the reset rule (`specs/018-schedules-bulk-operations/contracts/journal-and-events.md`),
  `sdks/SPEC.md` (§6.4 the heartbeat source and interval, §6.5 `nextRetryDelay`, the exception map and the two codes a worker
  cannot send, §9 the register row for 1.9 and the field register), `docs/architecture/execution-semantics.md` (the deadlines and
  the retry table), the "Refined by" pointer in Architecture decision 0089, and the rows of
  `docs/engineering/architecture-principles.md` for the checks of Task 045-20. The contract suites fail on the new baselines until
  slices 2 and 3.
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-014, Requirement 045-020, Non-functional requirement 045-004
- [ ] T045-2 Domain types: the timeouts of a leased node with the heartbeat source, `RetryPolicy`'s final codes and overrides per
  code built through `RetryPolicy::new`, and their validation (bounds, contradictions, reserved and duplicated codes, invalid
  overrides). Unit tests first, one per refusal and per boundary value (1,000 and 999; 31,622,400,000 and one more; 10,000 and
  9,999; 86,400,000 and one more; 32 and 33 codes; 64 and 65 in all).
  Refs: Requirement 045-001, Requirement 045-002
- [ ] T045-3 The retry decision function of Technical Plan 6, called by `end_attempt` and `fail_scoped_attempt`, with the worker's
  clamped delay, the close floor, and `retryRule`, `lastAttemptCode` and `delaySource` on the events. A unit test per row for a root
  step, a scoped instance and a path instance, first; the existing retry tests unchanged.
  Refs: Requirement 045-007, Requirement 045-008
- [ ] T045-4 Events and commands: the deadline fields on `step.released`, `step.retry_due`, `step.retry_requested` and `run.resumed`,
  computed by the domain from `db_now` and truncated to microseconds; the clearing of the close deadline by the transitions of
  Requirement 045-004; `deadline` on the ending events; the lease decision with `db_now`, the session's minor and the floor; the two
  timeouts that end a step with no live attempt, guarded by the generation; `step.failed` from `READY` and `RETRY_WAIT` in the
  reducer; replay equals projection with and without the fields (a corpus of journals written before this design).
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-005, Requirement 045-013
- [ ] T045-5 `infra-ir`: parse the fields into the domain types and refuse with pointers and the closed reasons; an IR fixture per
  reason; publication refuses a version using a field until its requirement is recorded (`CAPABILITY_NOT_ACTIVE`), and holds the
  requirement through `platform_requirement_hold` in its transaction.
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-017
- [ ] T045-6 The two migrations of Technical Plan 2 (the columns, the `NOT VALID` checks and their validation, the indexes with their
  own invalid-index check, the two capabilities in the registry's closed lists, the definer function and its grant, a down migration
  that refuses a non-default value, the storage classification) and the event codec of the new fields.
  `integration_platform_foundation` (migrations as the runtime role, privilege audit) first.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-013, Non-functional requirement 045-003
- [ ] T045-7 The step statements write the event's deadlines and `min_protocol_minor`; the candidate query's four branches,
  `CLAIMABLE`, `CLAIMABLE_PATHS` and `RUN_FOR_CLAIM` carry the conditions; the lease and the renewal take the node's time-to-live; a
  withdrawal does not write the start deadline; the deadline scans of the store, excluding paused and stopping runs; the row-shape
  predicates at the top of `crates/infra-postgres/src/execution.rs` (the conditions that say a row holds no work) include the new
  columns. The budget lines
  of `integration_claim_transition_efficiency` must pass unchanged (seen passing before and after), plus a new line for a poll with
  unclaimable rows ahead of claimable work; `scripts/check-capacity-activation.py` passes; a store test per column.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-005, Requirement 045-006, Non-functional requirement 045-001
- [ ] T045-8 The `step_deadlines` pass in `crates/scheduler` (its own `pick` window, the shard fence and the step check, root and
  path transactions, the metrics and the no-op counter); the retries pass ends a `RETRY_WAIT` step past its floor; the lost-lease
  pass records the deadline cause; the resume moves the start deadlines. Crash, duplicate and pause tests in
  `fault_activity_timeouts` first.
  Refs: Requirement 045-004, Requirement 045-009, Requirement 045-018
- [ ] T045-9 `crates/application`: the per-node time-to-live in the claim and `heartbeat`; `fail()` records a worker's
  `SCHEDULE_TO_START_TIMEOUT` or `SCHEDULE_TO_CLOSE_TIMEOUT` as `WORKER_ERROR`; the delay joins the report's fingerprint; the operator's
  retry writes a start deadline and clears the close deadline. Tests per rule first.
  Refs: Requirement 045-005, Requirement 045-008, Requirement 045-019, Requirement 045-020
- [ ] T045-10 The close deadline at feature 022's gates: the waiter's deadline as the earlier of the queue deadline and the close
  deadline in 022's one timeout routine, the eligibility rule, the return of a granted step's slot and permit in 022's lock order,
  the start deadline on the grant events, and the races of Technical Plan 7; built by whichever of 022's gate and this design lands
  second; the runtime role's grants on 022's tables checked.
  Refs: Requirement 045-011
  Owner-gate: OD-2026-11-24-4
- [ ] T045-11 `crates/api-http` and the fleet requirements: the step view's deadlines and cause and the view's server time, the
  publication reasons in the problem body (contract test first, `contract_rest_durable_execution`); the capabilities
  `activity-timeouts-v1` and `activity-timeouts-handler-v1` in both process kinds, the publication gate and the replay floor
  (registry tests first, `apps/control-plane`, `apps/execution-engine`).
  Refs: Requirement 045-014, Requirement 045-017
- [ ] T045-12 The six SDK builders, validation and the exception-to-code map, with the conformance scenarios `activity-timeouts` (at
  1.8) and `retry-by-code`; each SDK's checks of `sdks/AGENTS.md`.
  Refs: Requirement 045-015
- [ ] T045-13 The six SDK hosts at 1.9: the interval from `heartbeat_timeout_ms`, the handler source with coalescing,
  `nextRetryDelay`; the conformance scenario `heartbeat-from-handler` and `activity-timeouts` at 1.9.
  Refs: Requirement 045-006, Requirement 045-016
  Owner-gate: OD-2026-11-24-4
- [ ] T045-14 The Studio: the inspector's fields, validation and warnings, the deadlines and the timeout sentences on a run's step,
  against the view's server time; component tests first, then the journeys.
  Refs: Requirement 045-014
- [ ] T045-15 Documents for people: a guide page on timeouts and retries with the Temporal comparison and the unknown-outcome rule,
  the runbook `docs/runbooks/step-timeouts.md` (the lag alert, the no-op counter, the index build, the two requirements and the
  replay floor), `docs/sdk-compatibility.md`, the release notes; `contract_developer_experience` regenerated only after reading its
  diff.
  Refs: Requirement 045-014, Requirement 045-018
- [ ] T045-16 The races of Technical Plan 7, each with the tests its row names, and the crash, duplicate, restart and lost-hint cases,
  in `fault_activity_timeouts` and `integration_activity_timeouts`; every suite also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.
  Refs: Requirement 045-010, Requirement 045-012
- [ ] T045-17 Measure Non-functional requirement 045-002 (1,000 deadlines in one second, and a one-tenant flood of 100,000), the beat
  rate at the minimum heartbeat timeout, and the index build on the scale fixture; record them with their environment in
  `tests/performance/reports/`, and decide Gap 045-004.
  Refs: Non-functional requirement 045-001, Non-functional requirement 045-002, Success criterion 045-005
- [ ] T045-18 Released 0.2 and mixed-fleet scenarios: runs from 0.2 finish unchanged, a 0.2 host is served at its minor and never
  leased a handler-sourced step, the requirement gates, the replay floor, rollback before and after the requirements.
  Refs: Requirement 045-017, Non-functional requirement 045-004
- [ ] T045-19 Protocol 1.9 on the server (`crates/api-grpc`, `crates/application`): negotiation offered only with 1.8, the lease
  fields, `next_retry_delay_ms` validated, refused below 1.9 and passed to the domain. Contract test of the descriptor first
  (`contract_grpc_durable_execution`).
  Refs: Requirement 045-006, Requirement 045-008, Requirement 045-016, Non-functional requirement 045-004
  Owner-gate: OD-2026-11-24-4
- [ ] T045-20 The fitness checks: a parity test of claim eligibility (the candidate conditions against the domain's lease decision
  over the same rows); a rule in `scripts/check-architecture.py`, with its fixtures, that the deadline columns are assigned only from
  event values and never by a withdrawal; a reset-registry test of the deadline rule.
  Refs: Requirement 045-003, Requirement 045-013, Requirement 045-020

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py` with its fixtures, `python3 scripts/check-agent-config.py`,
`python3 scripts/check-design-docs.py`, `python3 scripts/check-plain-words.py --base origin/develop`, the unit tests of the crates
it changes and of those that depend on them, and the suites below that it changes or that read what it changes
(`scripts/select-test-targets.py`):

- `integration_activity_timeouts` and `fault_activity_timeouts` (new; every race, crash, duplicate, restart, pause and lost-hint
  case), also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_durable_execution`, `integration_sagas_compensations` (the retry table, compensation untouched),
  `integration_workflow_primitives` (deadlines of steps in bodies), `integration_schedules_bulk_operations` (the reset rule);
- `integration_claim_transition_efficiency` (the budget lines unchanged, and the new line);
- `integration_platform_foundation` (migrations as the runtime role, the privilege audit);
- `integration_gitops_versioning` (publication refusals, the requirement gates and the hold);
- `contract_rest_durable_execution`, `contract_grpc_durable_execution`, the IR schema and protocol baseline tests;
- `sdk_conformance` for `activity-timeouts`, `retry-by-code` and `heartbeat-from-handler` in six languages, and the per-SDK checks of
  `sdks/AGENTS.md`;
- `(cd frontend/apps/studio && npm run api && npm run check:api && npm test)`;
- the released 0.2 scenario of Task 045-18.

Failure cases (principle VIII): a crash of the pass between its read and its commit; two passes on one shard; a claim at the start
deadline's either side, in the three forms of Technical Plan 7; two `READY` steps of one run; a completion at the attempt deadline of
each cause; a renewal against the heartbeat timeout; a withdrawal at the start deadline; a gate's grant and queue timeout against the
close deadline; a lagging retries pass against the close deadline; a pause against each deadline; a step in reconciliation past its
close deadline; an operator's retry past it; a stop against each deadline; a restart with deadlines pending; a lost `task.ready` hint
before a start deadline. A test that fails is a defect to explain, not to retry.
