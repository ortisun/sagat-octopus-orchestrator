# Activity timeouts and retry per failure code

*   **Author:** Claude Code, for the owner (under Owner decision 2026-11-20-6 and Owner decision 2026-11-24-4)
*   **Status:** Draft
*   **Date:** 2026-11-24

--------------------------------------------------------------------------------

## Overview

Release 0.3.0 gives a leased step the four timeouts a Temporal activity has and a retry policy that decides per failure code.
The owner put them in 0.3.0 (Owner decision 2026-11-24-1: "activity timeouts (schedule-to-start, schedule-to-close, heartbeat
timeout per activity) and non-retryable error types").

A workflow author writes the timeouts on an `activity` or a `script` node, in the root graph and in the body of a collection or a
loop, and the retry fields on every `retry` object of the workflow's intermediate representation (the IR, the JSON document a
version publishes), `compensationRetry` included:

| Field | Meaning | In Temporal |
|---|---|---|
| `timeoutMs` (exists) | One attempt, from its claim to its report | start-to-close |
| `scheduleToStartMs` (new) | One attempt, from the moment it can be claimed to its lease | schedule-to-start |
| `scheduleToCloseMs` (new) | The whole step, from its first release to its end: every attempt, retry wait and gate wait | schedule-to-close |
| `heartbeatTimeoutMs` (new) | The lease of one attempt lapses after this long without a renewal | heartbeat timeout |
| `heartbeatFrom` (new) | `host` (the host renews while the handler runs, today's behaviour) or `handler` (only a handler's `heartbeat()` renews) | the activity code heartbeats |
| `retry.nonRetryableErrorCodes` (new) | Failure codes that end the step at once | `nonRetryableErrorTypes` |
| `retry.byErrorCode` (new) | A retry policy for named failure codes, over the node's base policy | (no equivalent) |

Differences from Temporal, and what each gives a customer:

- **A timeout never repeats an effect whose outcome is unknown.** Temporal retries an activity whose attempt timed out, assuming
  it is idempotent. Here a deadline that ends a live attempt leaves its outcome unknown, so the step is reconciled (`reconcileWith`,
  on an activity) or reviewed, as feature 037 decided for every lost attempt (constitution principle III). A deadline that ends no
  live attempt (a task nobody claimed, a retry that would start too late) is a known failure and takes the node's failure route.
- **One clock decides.** Every deadline is computed by the domain from the database clock when the transition that starts it
  happens, journaled, and compared only with the database clock; the lease decision refuses an attempt whose deadline has passed,
  so a lease and a timeout of the same attempt never both commit.
- **A retry policy per failure code**, not only a list of codes that are never retried: `RATE_LIMITED` can get ten attempts spaced a
  minute apart while `ACTIVITY_ERROR` keeps three. A worker can also ask for a longer wait before the next attempt
  (`nextRetryDelay`, for a provider's `Retry-After`), never a shorter one than the policy's.
- **Two kinds of heartbeat.** `host` detects a lost process without one line of handler code; `handler` detects a stuck handler,
  which is Temporal's meaning.
- **The journal says which deadline ended an attempt** (`deadline: startToClose`, `scheduleToClose` or `heartbeat`), and the Studio
  shows the deadlines of a step before they pass.

The rules are [Architecture decision 0110](../adr/ADR-0110-step-timeouts-are-deadlines-on-the-database-clock-and-retry-is-decided-per-failure-code.md),
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
  attempt deadline) is at least 18 s after the claim, three receive bounds of 6 s, the longest a 1.8 host may take to read a lease
  ([Architecture decision 0099](https://github.com/ortisun/sagat-octopus-orchestrator/blob/docs/delivery-acknowledgement/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md),
  pull request #264, accepted by the owner on 2026-11-21, not merged). `TASK_NOT_STARTED` is the code with which a 1.8 host hands
  back a lease it did not start.
- **The step's release.** The transition that first makes a step due to run: `step.released`, the admission of a body instance
  (`loop.iteration_admitted`, `collection.iteration_admitted`), or, for a node with a concurrency or rate gate of feature 022, the
  moment it starts waiting at the gate.
- **Claimable.** A step is claimable when it is `READY`: released and past any gate (Architecture decision 0089), a retry whose
  backoff has passed (`step.retry_due`), or a step an operator asked to retry (`step.retry_requested`).
- **Live attempt.** An attempt whose lease is committed: `LEASED` or `RUNNING` (the first heartbeat moves `LEASED` to `RUNNING` and
  journals `step.started`).
- **The database clock; `db_now`.** A run transaction reads the database's time once, under the run lock (`db_now`,
  `clock_timestamp()` in `crates/infra-postgres/src/execution.rs`); the domain uses it as the transition's time. A candidate query
  and a scan compare with the clock function they use today (`clock_timestamp()` or `now()`). No process clock decides a deadline.
- **The run lock, the chain lock.** A run transaction locks the run's continuation chain, then the run's row, and holds both until
  it commits; every transition of a step's status happens under them. A delivery acknowledgement and a binding revocation write
  lease rows without the run lock, under their own row and advisory locks.
- **Shard owner; window; pass.** Each run belongs to a shard; the scheduler process that holds a shard's lease (its owner) runs
  passes over that shard, each reading a bounded window of candidates fairly across tenants (`pick` in
  `crates/scheduler/src/passes.rs`) and re-checking each under the run lock before acting.
- **Root, scoped and path steps.** A root step is in the run's top graph; a scoped or path step is an instance inside a collection or
  loop body.
- **The deadlines.** *Start deadline* (`steps.start_deadline_at`): claimable time plus the node's *start window*
  (`scheduleToStartMs`, kept as `steps.start_window_us`). *Close deadline* (`steps.close_deadline_at`): release time plus
  `scheduleToCloseMs`. *Close floor*: the shorter of the node's `timeoutMs` and 6,000 ms; an attempt is not started with less than
  this left. *Close cutoff* (`steps.close_cutoff_at`): the close deadline less the close floor, the last moment an attempt may
  start. *Attempt deadline* (`steps.attempt_deadline_at`, exists): the earlier of the claim's time plus `timeoutMs` and the close
  deadline. *Lease expiry* (`steps.lease_expires_at`, exists): the claim's or the last renewal's time plus the node's
  `heartbeatTimeoutMs` (or the platform's 30 s); a renewal never moves it past the attempt deadline.
- **This design's level of a version.** 0 when the version uses none of its fields; 1 when it uses some; 2 when it also needs
  protocol 1.9 hosts (`heartbeatFrom: "handler"`, or a `heartbeatTimeoutMs` below 30,000 ms). Publication records it
  (`workflow_versions.activity_timeouts_level`). Only a version of level 1 or 2 is read with these rules; a version of level 0 is run
  exactly as today, whatever its stored document contains.
- **Outcome-unknown codes.** The seven codes for which `FailureCode::leaves_outcome_unknown` holds (`crates/domain/src/execution/event.rs`):
  `LEASE_EXPIRED`, `ATTEMPT_TIMEOUT`, `OUTCOME_UNKNOWN`, `STORAGE_QUOTA_EXCEEDED`, `RECONCILIATION_FAILED`,
  `RECONCILIATION_UNKNOWN`, `RECONCILIATION_NOT_FINAL`. This design adds none. It adds two known-failure codes,
  `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT` (Technical Plan 12).

## The Problem

A step that nobody claims waits until its run's deadline: there is no way to say "if no worker takes this within five minutes, go to
the fallback". A step whose retries back off exponentially can outlive the business window it serves, because there is no budget
for the whole step. The lease time-to-live is one value for the platform (`lease_ttl`, 30 s, `crates/application/src/worker.rs`),
so a slow external call that blocks a host's event loop for 40 s loses its lease, and a worker process that dies holds a fast step
for 30 s however short the step is. A handler stuck in a deadlock keeps its lease until the run's deadline, because the host renews
it while the handler runs. And the retry policy (`RetryPolicy` in `crates/domain/src/workflow.rs`) treats every failure alike: an
author cannot say "a declined card is final, a rate limit is retried ten times".

The roadmap assessment of 2026-11-11 (`docs/roadmap-assessment-2026-11-11.md`, Temporal-parity table) names these gaps:
"Schedule-to-start and schedule-to-close timeouts, per-activity heartbeat timeout, non-retryable error types: no spec found".

## Requirements

### Functional

- **Requirement 045-001 — Timeout fields.** An `activity` or `script` node, in the root graph (`$defs/activity`, `$defs/script`) and
  in a body (`$defs/bodyActivity`, `$defs/bodyScript`), MAY declare:
  - `scheduleToStartMs`, an integer from 10,000 to 31,622,400,000 (ten seconds to 366 days);
  - `scheduleToCloseMs`, an integer from 1,000 to 31,622,400,000;
  - `heartbeatTimeoutMs`, an integer from 18,000 to 86,400,000 (eighteen seconds to 24 hours);
  - `heartbeatFrom`, `host` (the default) or `handler`.

  `timeoutMs` stays optional. Publication MUST refuse, with the field's JSON pointer: a value out of range; `heartbeatFrom: "handler"`
  without both `heartbeatTimeoutMs` and `scheduleToStartMs`; `scheduleToCloseMs` below the close floor plus 1,000 ms (so that some
  moment exists at which an attempt may start); and, between fields that are both declared, `heartbeatTimeoutMs` greater than
  `timeoutMs` or than `scheduleToCloseMs`, `timeoutMs` greater than `scheduleToCloseMs`, `scheduleToStartMs` greater than
  `scheduleToCloseMs`. Any other kind of node, a compensation and a reconciliation included, MUST NOT declare them. *Acceptance:* a
  publication test per refusal and per accepted combination (`integration_gitops_versioning`), the IR schema test, and the Studio's
  form test.
- **Requirement 045-002 — Retry fields.** A `retry` (`$defs/retry`, `$defs/bodyRetry`, and so a `compensationRetry`) MAY declare
  `nonRetryableErrorCodes` (1 to 32 distinct codes) and `byErrorCode` (1 to 16 entries, each with `codes`, 1 to 32 distinct codes, and
  any of `maxAttempts`, `initialDelayMs`, `maxDelayMs`, `strategy`, `jitter` with the meaning and bounds they have in `retry`). A code
  matches `^[A-Z0-9_]{1,64}$`, the grammar every SDK host sends (`sdks/SPEC.md` §6.5). It appears at most once across both lists,
  and the lists name at most 64 codes in all. None is one the platform decides elsewhere: the seven outcome-unknown codes,
  `CANCELLED`, `TASK_NOT_STARTED`, `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT`. The effective policy of each entry
  (Technical Plan 6) MUST be valid. *Acceptance:* a publication test per refusal (`RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`,
  `RETRY_CODES_TOO_MANY`, `RETRY_OVERRIDE_INVALID`).
- **Requirement 045-003 — Schedule-to-start.** When an attempt of a node with `scheduleToStartMs` becomes claimable, the domain MUST
  compute its start deadline from `db_now` and record it in the event that makes it claimable; while the run is paused, it records
  none (Requirement 045-021). The lease decision MUST NOT lease an attempt whose start deadline is at or before `db_now`. Once it has
  passed, the step MUST end `FAILED` with `SCHEDULE_TO_START_TIMEOUT`, which is not retried and takes the node's failure route: no
  lease of the attempt started work in time. *Acceptance:* the claim tests of Technical Plan 7.
- **Requirement 045-004 — Schedule-to-close.** The close deadline and the close cutoff MUST be computed and recorded at the step's
  release and kept by every later attempt. An attempt MUST NOT be leased, and a retry MUST NOT be scheduled, after the close cutoff.
  A step past its close cutoff with no live attempt MUST end `FAILED` with `SCHEDULE_TO_CLOSE_TIMEOUT`, a known failure (Technical
  Plan 4 says which path ends it in each status). An attempt's deadline MUST be the earlier of its claim time plus `timeoutMs` and
  the close deadline; when the close deadline ends a live attempt, the attempt ends `ATTEMPT_TIMEOUT` with `deadline:
  scheduleToClose`, and the outcome-unknown rule applies. The close deadline and cutoff MUST be cleared when the step goes to
  reconciliation (`reconciliation.requested`) or to review (`step.needs_review`): from then on a reconciliation or a person decides,
  not the step's budget. *Acceptance:* the tests of Technical Plan 7, one per status in which the cutoff can be reached and one per
  transition that clears it.
- **Requirement 045-005 — Heartbeat timeout.** A lease of a node with `heartbeatTimeoutMs` MUST be granted and renewed with that
  time-to-live instead of the platform's; a renewal never moves the expiry past the attempt deadline. An attempt that lapses MUST end
  with `LEASE_EXPIRED`, as today, and its event MUST record `deadline: heartbeat`. *Acceptance:* the lapse test per node kind, with
  and without the field.
- **Requirement 045-006 — Steps that need protocol 1.9.** A node with `heartbeatFrom: "handler"`, or with a `heartbeatTimeoutMs` below
  30,000 ms, MUST be leased only to a session accepted at the protocol minor this design takes (1.9 at the time of writing) or later,
  because only such a host reads the time-to-live as a duration and honours the handler source. Such a host MUST send the first beat
  itself when the handler starts; with the handler source it afterwards renews the lease only when the handler called `heartbeat()`
  since its last renewal. *Acceptance:* the claim test with a 1.8 and a 1.9 session, and the conformance scenario
  `heartbeat-from-handler` in six languages.
- **Requirement 045-007 — The retry decision.** When an attempt of a root, scoped or path step ends with a report, one decision
  function MUST apply the rows of Technical Plan 6 in order. In short: an outcome-unknown code goes to reconciliation or review
  (feature 037, unchanged); a step asked to stop is cancelled (unchanged); a worker's `retryable = false` is final; a code in
  `nonRetryableErrorCodes` is final; the effective policy is spent at its `maxAttempts`; a retry after the close cutoff fails the step
  `SCHEDULE_TO_CLOSE_TIMEOUT`; otherwise the step is retried. *Acceptance:* the decision-table unit test (every row), for a root step,
  a scoped instance and a path instance, and the conformance scenario `retry-by-code`.
- **Requirement 045-008 — A worker's delay.** `FailStep.next_retry_delay_ms` (1 to 86,400,000; out of range refused
  `INVALID_ARGUMENT`) MUST make the delay of the next attempt the longer of the worker's delay and the policy's computed delay for
  that attempt, without further jitter, when the decision schedules one. The event MUST record `delaySource: worker` whenever the
  field was sent. It MUST NOT add an attempt or pass the close cutoff. Its value is part of the report's idempotency fingerprint
  only when it is present. A session below 1.9 that sends it is refused `INVALID_ARGUMENT`. *Acceptance:* unit tests and the
  conformance scenario `retry-by-code`.
- **Requirement 045-009 — Deadlines are acted on by a pass on the database clock.** A scheduler pass of its own, `step_deadlines`,
  MUST end `READY` steps whose start deadline or close cutoff has passed: on its shard, through `pick` with the tenant exclusion and
  its own window, re-checking under the run lock both the shard fence and the step (status, generation, deadline) in the transaction
  that ends it. A pass that crashes, or two passes, MUST end a step at most once. *Acceptance:* the crash and duplicate tests of
  `fault_activity_timeouts`.
- **Requirement 045-010 — Races.** Each race of Technical Plan 7 MUST resolve as its row says. The loser MUST be a no-op that leaves
  evidence: a refusal for a caller, a counted no-op for a pass, a counted skip for a claim. *Acceptance:* one barrier test per row,
  never a sleep (AGENTS.md rule 18).
- **Requirement 045-011 — Gates.** A step waiting at a gate of feature 022 MUST have as its waiter deadline the earlier of the gate's
  queue deadline and the close cutoff, so that 022's one timeout routine (Architecture decision 0089 decision 5) ends it. When the
  close cutoff was the earlier, the routine journals the dequeue of each open gate (`concurrency.dequeued`, `rate_limit.dequeued`) and
  `step.failed` with `SCHEDULE_TO_CLOSE_TIMEOUT`, in one transaction; otherwise it journals the gate's own `wait_timed_out` as today.
  A waiter past its close cutoff is not eligible for a grant. A granted step ended by either deadline before its lease MUST return
  its slot and permit as feature 022 specifies. *Acceptance:* the gate tests of Technical Plan 7, built with whichever of 022's gate
  and this design lands second.
- **Requirement 045-012 — Compensation and reconciliation keep their rules.** The new timeouts MUST NOT apply to a compensation or a
  reconciliation node, and the close deadline does not bound a reconciliation; `compensationRetry` gains only the retry fields.
  *Acceptance:* a test that a saga's compensation and a reconciliation are unaffected by a forward node's `scheduleToCloseMs`.
- **Requirement 045-013 — Journal.** The fields this design adds to events are optional, and are written only for a run of a version
  of level 1 or 2; an event written before this design decodes as before. A run's replay MUST equal its projection with and without
  them. The domain computes each deadline (truncated to microseconds) and the store writes the event's value, or, for the pause and
  the resume, applies the one arithmetic rule of Technical Plan 3 to every claimable row. *Acceptance:* the replay-equals-projection
  corpus of `integration_activity_timeouts`, comparing the deadline columns (the lease expiry, which renewals move, excluded).
- **Requirement 045-014 — Visible deadlines.** The REST view of a step and the Studio MUST show its start, close and attempt deadlines
  and the cause of the attempt deadline, and a timeout's code with its meaning; the time left is computed against the server time the
  view carries, never the browser's clock. *Acceptance:* `contract_rest_durable_execution` and the Studio's run-view tests.
- **Requirement 045-015 — SDK builders.** The six SDKs' workflow builders MUST offer the fields of Requirements 045-001 and 045-002,
  validated as the server validates them, and the SDKs MUST let a worker map exception classes to failure codes at registration,
  refusing a code outside the list grammar. *Acceptance:* each SDK's unit tests and the IR conformance scenarios.
- **Requirement 045-016 — SDK hosts at 1.9.** A host accepted at 1.9 MUST beat at the shorter of its configured interval and a third
  of `heartbeat_timeout_ms`, within today's bounds of 500 ms and 10 s, its first interval also no longer than a third of the lease's
  time left at receipt; it honours `heartbeat_from_handler` and sends `next_retry_delay_ms` when the handler asks. *Acceptance:* each
  SDK's unit tests and the conformance scenarios `activity-timeouts` and `heartbeat-from-handler`, one case with the lease received
  6 s after its claim.
- **Requirement 045-017 — Fleet switches and runs in flight.** A version of level 1 MUST publish only once the operator recorded the
  requirement `activity-timeouts-v1`, and one of level 2 also `activity-timeouts-protocol-v1`, which only a build that offers this
  design's minor lists. Both are recorded for control planes and engines. Publication holds each requirement it needs until its
  transaction commits, so a removal cannot commit between the publication's check and its commit. A requirement MUST NOT be removed
  while any workflow version of the level that needs it exists, retired included: once a version used these fields, the release
  that introduced them is the floor for a rollback. A version of level 0 MUST behave for every run exactly as today. *Acceptance:*
  the registry tests and the released 0.2 scenario of Technical Plan 13.
- **Requirement 045-018 — Telemetry and audit.** The metrics, spans and audit records of Technical Plan 11 MUST be emitted with closed
  labels, and no tenant, namespace, run identifier or worker-sent code as a label or in a log. *Acceptance:* the metrics text test of
  `integration_activity_timeouts`.
- **Requirement 045-019 — Only the platform reports a timeout.** A worker's report with `SCHEDULE_TO_START_TIMEOUT` or
  `SCHEDULE_TO_CLOSE_TIMEOUT` MUST be recorded as `WORKER_ERROR`, at the parse of the code, before the fingerprint and before the
  root or path branch. A worker-sent `ATTEMPT_TIMEOUT` or `LEASE_EXPIRED` never gets a `deadline` field. Timeouts MUST be counted
  where the platform decides them, never from a worker's code. *Acceptance:* a worker-report test per code, root and path.
- **Requirement 045-020 — Operator actions.** An operator's retry (`step.retry_requested`) of a step that failed with a known code
  MUST write a new start deadline and clear the close deadline and cutoff: the person has looked at the step and decided. A step
  whose outcome is unknown is not retried until a person settles it (feature 037, `ExecutionError::OutcomeUnresolved`, unchanged).
  *Acceptance:* an operator-retry test past the close cutoff, and the existing test that refuses a retry of an unsettled step.
- **Requirement 045-021 — Pause and withdrawal.** While a run is paused, no claimable step of it has a start deadline; the resume
  gives each claimable step of the run, root and path, a full start window from the resume's `db_now`. The close deadline is not
  moved by a pause and is enforced during it. A lease withdrawal MUST write the start deadline as the later of the old one and the
  withdrawal's `db_now` plus the shorter of the start window and 6,000 ms, so that a delivery lost on the wire does not fail a step
  whose workers are up. *Acceptance:* the pause and withdrawal tests of Technical Plan 7.

### Non-functional

- **Non-functional requirement 045-001 — No new statement on the claim or the completion.** A lease stays 38 statements in 2
  transactions and a completion 50 statements in 1 (the lines of `tests/integration/claim-transition-efficiency/work_budget.rs`); the
  new columns are written by the statements that write the step today; the claim's new conditions are on the rows its candidate
  query already reads; a budget line covers a poll with rows it cannot claim ahead of claimable work.
- **Non-functional requirement 045-002 — Lag.** A deadline is acted on by a pass of its shard owner. The bound follows from the
  scheduler's defaults (200 candidates per pass and tick in `crates/scheduler/src/passes.rs`, and a pause of 400 to 600 ms after a
  tick that found nothing, `apps/execution-engine/src/loops.rs`): 1,000 deadlines due in one second need at most 1,000 / 200 = 5
  ticks, so at most 5 × 600 ms = 3,000 ms of pauses plus the work of the 1,000 run transactions. Criterion, measured on the CI
  machine and reported with its environment (constitution, Delivery Constraints): every one of 1,000 steps ended within 5 s of its
  deadline, the 99th percentile under 4 s; and, while one tenant floods 100,000 such deadlines, another tenant's 1,000 keep that
  criterion.
- **Non-functional requirement 045-003 — Isolation.** No new table. The new columns are on `steps`, and one on `workflow_versions`,
  read and written under the run's or the version's tenant; when feature 022's gates are built, one column on its two waiter tables
  (Technical Plan 4). One new grant: `EXECUTE` on the function that holds a requirement for a publication (Technical Plan 13).
- **Non-functional requirement 045-004 — Compatibility.** Protocol 1.9 is additive; a host at 1.0 to 1.8 is served as today; the IR
  grows by optional fields only; the journal's new fields are optional and written only for versions of level 1 or 2.

### Success criteria

- **Success criterion 045-001.** A step with `scheduleToStartMs: 10000` and no worker registered ends `SCHEDULE_TO_START_TIMEOUT` and
  takes its `onError` route, and a worker that registers after the deadline is never leased it.
- **Success criterion 045-002.** A step with CUJ 2's policy never starts an attempt after its close cutoff and ends
  `SCHEDULE_TO_CLOSE_TIMEOUT` with the last code recorded.
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
  is the way to make such a step go on by itself, and a qualified safe-repeat (Architecture decision 0076) is not in this design. A
  `script` node cannot declare `reconcileWith`, so its timed-out attempt goes to review.
- Timeouts on compensation, reconciliation, child, wait or approval nodes (each has its own deadline today), and on the managed
  database and lake nodes, whose runners are platform processes with their own limits (Gap 045-003).
- Patterns or prefixes in the retry lists (Alternatives Considered).
- **Buffered signals.** The roadmap assessment also names "buffered signals (an early signal is refused today)". Feature 018 does not
  cover them (its spec lists them under Out of scope, on Architecture decision 0050 decision 4 and Architecture decision 0030
  decision 17). They share no mechanism with this design (no timer, no lease, a fleet switch of their own) and belong to a design of
  their own, which design 044 (workflow as code) will depend on.
  [Architecture decision 0111](../adr/ADR-0111-a-version-may-declare-buffered-signals-kept-in-a-bounded-per-run-inbox.md) is its
  proposal (Decision 045-009).
- Workflow as code: design 044 decides how its activity calls carry these options.
- Heartbeat details: feature 019 (Technical Plan 5 says how they fit). Feature 019's `asyncCompletion.heartbeatTimeoutMs` is another
  field, of a detached attempt, with its own bounds; the guide says so.

## Open Questions

- Whether a step's start deadline should also be offered to 019's priority order as an urgency hint (Gap 045-002); not in 0.3.0.

## Owner decisions needed

None open. The owner's decisions this design rests on: Owner decision 2026-11-24-1 (the scope), Owner decision 2026-11-20-6 and Owner
decision 2026-11-24-4 (the delegation of its questions, of its architecture decision and of its approval), Owner decision 2026-11-15-8
(the register of protocol minors). Each question below was decided under that delegation and is recorded with its options and
reasoning in `specs/045-activity-timeouts/research.md`:

- Owner question 045-001: does a timeout that ends a live attempt retry, as in Temporal? No: outcome unknown (Decision 045-001).
- Owner question 045-002: is `SCHEDULE_TO_START_TIMEOUT` retried? No: final, failure route (Decision 045-002).
- Owner question 045-003: a new error-type field, or the existing `error_code`? `error_code` (Decision 045-003).
- Owner question 045-004: heartbeat from the host, from the handler, or both? Both, `host` by default (Decision 045-004).
- Owner question 045-005: a retry policy per code, or only a non-retryable list? Both (Decision 045-005).
- Owner question 045-006: which node kinds? `activity` and `script`, root and body (Decision 045-006).
- Owner questions 045-007 and 045-008 (buffered signals across continuation; opt-in buffering) moved with buffered signals to their
  own design (Decisions 045-007 and 045-008).
- Owner question 045-009: where do buffered signals belong? A design of their own (Decision 045-009).
- Decided during the reviews: an operator's retry of a known failure clears the close deadline (Decision 045-018); a pause removes the
  start deadline and the resume gives a full window (Decision 045-019); the close floor and cutoff (Decision 045-021); the protocol
  requirement (Decision 045-022); the minimum heartbeat timeout of 18,000 ms (Decision 045-024); a withdrawal's fresh window
  (Decision 045-025).

## Critical User Journeys (CUJs)

### CUJ 1: No worker takes the task

*   **Given**: a `chargeCard` activity with `scheduleToStartMs: 30000` and `onError: notifyOps`, and every payments worker down.
*   **When**: the step becomes `READY` and 30 s pass on the database clock.
*   **Then**: the next pass ends it `SCHEDULE_TO_START_TIMEOUT`, the run goes to `notifyOps`; a worker that comes back at second 31
    is not leased the attempt; the Studio showed the start deadline while it waited.

### CUJ 2: Retries that outlive their window

*   **Given**: `reserveSeat` with `scheduleToCloseMs: 120000`, `retry: {maxAttempts: 10, initialDelayMs: 5000, jitter: false}`
    and no `timeoutMs` (so its close floor is 6 s and its close cutoff is at second 114), and attempts failing with `UPSTREAM_BUSY`.
*   **When**: the fourth failure, at second 95, would schedule its retry 40 s later (5 s doubled three times), at second 135.
*   **Then**: no retry is scheduled; the step fails `SCHEDULE_TO_CLOSE_TIMEOUT` with `lastAttemptCode: UPSTREAM_BUSY`, and a saga
    around it compensates what completed.

### CUJ 3: A declined card is final, a rate limit is patient

*   **Given**: on a 1.9 host, `retry: {maxAttempts: 3, nonRetryableErrorCodes: ["CARD_DECLINED"], byErrorCode: [{codes:
    ["RATE_LIMITED"], maxAttempts: 10, initialDelayMs: 60000, maxDelayMs: 600000, strategy: "fixed"}]}`.
*   **When**: an attempt fails `RATE_LIMITED` with `nextRetryDelay` of 90 s, later another fails `CARD_DECLINED`.
*   **Then**: the first is retried after 90 s (the longer of the worker's 90 s and the entry's 60 s), the second ends the step at once;
    the journal records which rule decided each and that the first delay came from the worker.

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
"scheduleToStartMs": {"type": "integer", "minimum": 10000, "maximum": 31622400000},
"scheduleToCloseMs": {"type": "integer", "minimum": 1000, "maximum": 31622400000},
"heartbeatTimeoutMs": {"type": "integer", "minimum": 18000, "maximum": 86400000},
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
                 "initialDelayMs": {"type": "integer", "minimum": 1, "maximum": 86400000},
                 "maxDelayMs": {"type": "integer", "minimum": 1, "maximum": 86400000},
                 "strategy": {"enum": ["fixed", "exponential"]}, "jitter": {"type": "boolean"}}}}
```

The schema cannot express the cross-field rules of Requirements 045-001 and 045-002. The domain's constructors own them and their
closed reasons (`TIMEOUT_OUT_OF_RANGE`, `TIMEOUTS_CONTRADICT`, `HEARTBEAT_SOURCE_NEEDS_TIMEOUTS`, `TIMEOUT_NOT_ALLOWED_ON_NODE`,
`RETRY_CODE_RESERVED`, `RETRY_CODE_DUPLICATED`, `RETRY_CODES_TOO_MANY`, `RETRY_OVERRIDE_INVALID`); `crates/infra-ir` parses and maps
each refusal to the field's pointer. The reasons, and `ACTIVITY_TIMEOUTS_NOT_ACTIVE` (Technical Plan 13), are listed in
`specs/045-activity-timeouts/contracts/errors.md`, written by Task 045-1.

In the domain (`crates/domain/src/workflow.rs`): `Node` gains `StepTimeouts` (the two schedule durations, and the heartbeat timeout
with its source) beside its `timeout`; a new type `RetryRules {base: RetryPolicy, final_codes, overrides}` sits beside `RetryPolicy`,
which stays `Copy` with its `const` presets; `RetryRules::for_code(&FailureCode)` answers `CodeRule::Final` or
`CodeRule::Policy(RetryPolicy)`; `WorkflowDefinition::activity_timeouts_level()` answers the version's level. No `serde_json` enters
the domain (principle II). In `infra-ir`, `retry_at` (`crates/infra-ir/src/convert.rs`) builds each entry's policy with the base as
its defaults.

### 2. The deadlines, their events and their columns

**One source.** For a version of level 1 or 2, the domain computes each deadline as `db_now` plus a duration, truncated to
microseconds, and puts it in the event of the transition that starts it; the reducer applies it; the store writes the event's value
into the column. For the pause and the resume the store applies one arithmetic rule to every claimable row (Technical Plan 3). A
version of level 0 writes none of these values, and its attempt deadline is computed in SQL as today. A static check keeps the store
from computing a deadline otherwise (Task 045-20).

| Deadline | Set by the events | Kept through | Cleared by |
|---|---|---|---|
| start (`start_deadline_at`; the window in `start_window_us`) | the release past its gates; `step.retry_due` and `step.retry_requested` (for a gated node, the grant instead); the admission of a body instance; `step.lease_withdrawn` (Technical Plan 3); `run.resumed` | the lease | the end of the attempt by a report; the end of the step; `run.paused` |
| close and cutoff (`close_deadline_at`, `close_cutoff_at`) | the step's release | every retry, a pause | the end of the step; `reconciliation.requested`; `step.needs_review`; `step.retry_requested` of a known failure |
| attempt (`attempt_deadline_at`, exists; `attempt_deadline_cause` new) | `step.leased`, which gains `attemptDeadlineUs` and `attemptDeadlineCause` for a version of level 1 or 2 | the attempt | the end of the attempt |

`min_protocol_minor` (`smallint NOT NULL DEFAULT 0`) is written at the release: the minor this design takes for a node of Requirement
045-006, 0 otherwise. It is a step-level form of what the claim already does per run (`required_worker_protocol_minor`) and per
managed kind (`DATABASE_PROTOCOL_MINOR`, `crates/application/src/worker.rs`). The attempt deadline is the earlier of `db_now` plus
`timeoutMs` and the close deadline, with the cause of the earlier one (a tie goes to `START_TO_CLOSE`); a node with a close deadline
and no `timeoutMs` gets the close deadline, so that renewals are capped by it.

**The statements.** No statement is added: the columns are added to the statements that write a step today, which are the root
statement, the scoped statement of a full-run transaction (`crates/infra-postgres/src/execution.rs`), `upsert_path_instance`
(`execution/path.rs`), and the statements that insert a body step `READY`: `insert_ready_activity` (`loop_progress.rs`), the
collection admission (`primitives.rs`) and the chained collections (`chain_progress.rs`). The row-shape predicates at the top of
`execution.rs` (the conditions that say a row holds no work) include the new columns. The loaders that restore a step (the root load,
`RUN_FOR_CLAIM` in `execution.rs`, the path snapshot reader) select them, and `StepState` gains `start_deadline`, `start_window`,
`close_deadline`, `close_cutoff` and `min_protocol_minor`, with `StepState::restore` taking them.

**Two migrations** in `crates/infra-postgres/migrations/`, named to sort after feature 022's (`20270210…`) and Architecture decision
0098's (`20270208…`), and built by the checklist of `.claude/skills/migration`. The first adds the six columns of `steps` and the
level of `workflow_versions`, the `CHECK` constraints `NOT VALID`, the two capabilities to the registry's three closed lists (replacing
`serving_instances_capabilities_known`, `platform_requirements_capability_known` and
`platform_requirement_history_capability_known`, as design 043's migration did), the hold function and the replacement of
`platform_requirement_activate` and `platform_requirement_remove` (Technical Plan 13), and the two indexes. The second validates the
constraints. The down migration refuses while a column holds a non-default value or the requirement history names either capability,
and restores the closed lists. Each variable-size column is classified in `crates/infra-postgres/storage-classification.json`. The
indexes, in the pattern of `20261017000000_scheduler_shards.up.sql` (`IF NOT EXISTS`, then a check that refuses an index a failed
concurrent build left invalid), so that an operator can build them `CONCURRENTLY` beforehand:

```sql
CREATE INDEX IF NOT EXISTS steps_start_deadline_shard_idx ON steps (run_shard(run_id), start_deadline_at)
    WHERE status = 'READY' AND start_deadline_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS steps_close_cutoff_shard_idx ON steps (run_shard(run_id), close_cutoff_at)
    WHERE status = 'READY' AND close_cutoff_at IS NOT NULL;
```

Only `READY` rows are indexed: a `RETRY_WAIT` step is ended by the retries pass, a gated step by feature 022's timeout routine, and a
step waiting for reconciliation or review has no close cutoff. A paused run's rows have no start deadline.

### 3. The claim, the start deadline, a pause and a withdrawal

**The lease decision is the domain's.** One predicate, `leasable(step, db_now, host)`, where `host` is the session's features (for
example "reads the time-to-live as a duration and sources heartbeats from the handler", mapped from its accepted minor by the
application), says whether a step may be leased: its start deadline and its close cutoff are absent or after `db_now`, and its
`min_protocol_minor` is met. `claim_root_run` and `claim_path` (`crates/application/src/worker.rs` and `worker/scoped_claims.rs`)
apply it where they build the list of steps to lease, beside the managed-kind check and before any audit, quota or input work, so a
skipped step never aborts the run's other leases and leaves no misleading record; each skip is counted
(`step_claim_skips_total{reason}`). The lease commands (`lease_step`, `lease_instance`, `lease_path_instance`) check it again and
return `ExecutionError::NotClaimable` with its closed reason, which is then an invariant violation.

**The candidate query carries the same conditions as a hint.** The two forward branches of `claimable_query!`
(`crates/infra-postgres/src/workers.rs`; its two compensation branches need none) and `RUN_FOR_CLAIM` add them on the rows they
already read, with the clock function they use today, so unclaimable rows do not fill a poll's window (the precedent is the `$5 >= 5`
branch of Architecture decision 0032 decision 5). A parity test runs the hint and the domain's predicate over the same rows at a
fixed clock (Task 045-20).

**The pass.** `step_deadlines`, its own pass beside `leases` and the run-level `deadlines` of `crates/scheduler/src/passes.rs`,
reads through `pick` (the tenant exclusion, the per-shard limit, its own window) the rows of `steps_start_deadline_shard_idx` whose
deadline is at or before the clock, and of `steps_close_cutoff_shard_idx` whose cutoff is. Its store method is
`MaintenanceStore::due_step_deadlines`; its re-check under the run lock is `Condition::StepDeadlinePassed {instance, generation,
kind}` (the shard fence `View::holds`, then the step: still `READY`, the generation read, the deadline passed against `db_now`). If
it holds, the domain ends the step `FAILED` with no lease fence and the generation in the event; if the start deadline and the close
cutoff have both passed, the earlier names the code, and a tie goes to `SCHEDULE_TO_CLOSE_TIMEOUT`. If not, the pass counts a
no-op. A scoped or path step is ended in its path transaction with the scoped-protocol filter the `leases` pass uses. A run with a
stop reason is left to the stop path, which cancels the step. The pass is registered in the tick with its lag gauge, as the others.

**A pause** (Decision 045-019). `run.paused` gains `pausedAtUs` for a run of a version of level 1 or 2; its transaction writes the
start deadline of every claimable step of the run, root and path, as null, in one statement, and the domain records none while the
run is paused. `run.resumed` gains `resumedAtUs`; its transaction writes, in one statement, the start deadline of every claimable
step as `resumedAt` plus its `start_window_us`: a fresh full window, because a pause is an operator's hold, not a missing worker.
The close deadline is not moved and is enforced during the pause, by every path, because it is the author's business window.

**A withdrawal** (Architecture decision 0098 decision 3, Architecture decision 0099 decision 5) sets the step back to `READY` without
spending an attempt. Its event, `step.lease_withdrawn`, gains `startDeadlineUs`: the later of the old deadline and the withdrawal's
`db_now` plus the shorter of the start window and 6,000 ms. A lease lost on the wire is not a missing worker, so the attempt gets one
receive bound to be claimed again; the four withdrawals per attempt of Architecture decision 0098 decision 7 bound how often. Built
by whichever of this design and pull request #260 lands second.

### 4. The close deadline

| Status when the cutoff is reached | What happens |
|---|---|
| `READY` | the lease decision refuses the step; the `step_deadlines` pass ends it `SCHEDULE_TO_CLOSE_TIMEOUT` |
| `RETRY_WAIT` | a retry is never scheduled after the cutoff (Technical Plan 6, row 6); if the retries pass finds a due retry past it (when it lags), it ends the step `SCHEDULE_TO_CLOSE_TIMEOUT` instead of making it claimable |
| `WAITING` at a gate of feature 022 | the waiter's deadline (`deadline_at` of `concurrency_waiters` and of the rate-limit queue, `specs/022-flow-control-step-policies/data-model.md`) is written as the earlier of the queue deadline and the close cutoff, with a cause column (`deadline_cause`: `QUEUE` or `CLOSE`) on those two tables; 022's one timeout routine (Architecture decision 0089 decision 5) ends the step: with `CLOSE`, it journals `concurrency.dequeued` and `rate_limit.dequeued` for each open gate and `step.failed` with `SCHEDULE_TO_CLOSE_TIMEOUT`, in one transaction, so 022's rule that every waiter row is closed by a grant, a timeout or a dequeue still holds; a waiter past its cutoff is not eligible under decision 4 |
| `READY` after a gate's grant, not yet leased | the pass ends it, and the transaction returns its slot (`rate_limit.returned`) and its permit (`concurrency.released`) in feature 022's lock order, as 022 specifies for a granted step that leaves `READY` other than by a lease |
| `LEASED` or `RUNNING` | the attempt deadline is at most the close deadline: the lost-lease pass ends the attempt `ATTEMPT_TIMEOUT` with `deadline: scheduleToClose`; a provisional lease at that deadline is withdrawn (Architecture decision 0099 decision 5) and the step, now `READY` past its cutoff, fails `SCHEDULE_TO_CLOSE_TIMEOUT` |
| `WAITING` for reconciliation or review | not reached: the transition into it cleared the close deadline |

What follows a live attempt's timeout is feature 037's, unchanged: the reconciliation's answer `applied` completes the step;
`not_applied` and `unknown` are not final and wait for a person, who settles the step before any retry.

Feature 022's gates are specified and being built (`time_out_step_gates` exists on its branch, `concurrency_waiters` is not in
`crates/` on `develop`); whichever is built second adds the gate rows of this table and their tests (Task 045-10, and its counterpart
in feature 022's tasks). This refines Architecture decision 0089 decisions 4 and 5 (Architecture decision 0110).

The ending events of a live attempt that a deadline ended (`step.failed`, `reconciliation.requested`, `step.needs_review`) gain an
optional `deadline` field (`startToClose`, `scheduleToClose`, `heartbeat`), written only for a version of level 1 or 2: from
`attempt_deadline_cause` for an attempt timeout (carried by `LostLease.deadline` and `Expiry::AttemptTimeout {deadline}`), and
`heartbeat` for a lapse of a node with `heartbeatTimeoutMs`.

### 5. The heartbeat timeout and its source

The lease's time-to-live becomes per node: the five places that set it (`crates/application/src/worker.rs` at the root claim, the
managed claim and `heartbeat`; `worker/scoped.rs`; `worker/scoped_claims.rs`) take the node's `heartbeatTimeoutMs` when it is
declared and `settings.lease_ttl` otherwise; a renewal is capped at the attempt deadline, as today (a fresh grant is not; the
attempt-deadline row of the lost-lease pass ends the attempt first). The lost-lease pass ends a lapsed attempt with `LEASE_EXPIRED`,
as today, and records `deadline: heartbeat` when the node declares the field; when the expiry and the attempt deadline have both
passed, the attempt timeout is reported, as today (`crates/infra-postgres/src/maintenance.rs` reads it first). A renewal that commits
before the pass's transaction saves the attempt, as today. No outcome-unknown code is added, so the list and its two copies
(`crates/infra-postgres/src/chain_progress.rs`, `frontend/apps/studio/src/features/runs/ops.ts`) do not change.

**The bounds.** A 1.8 host may take up to one receive bound (6,000 ms) to read a lease (`specs/006-sdk-multilanguage/stream-completion.md`
on the branch of pull request #264); a lease is made provisional, and so protected against a loss on the wire, only when its earlier
deadline is at least 18,000 ms after the claim (Architecture decision 0099 decision 2). The minimum `heartbeatTimeoutMs` is that
18,000 ms (Decision 045-024), so every lease of this design can be provisional. A node with `heartbeatTimeoutMs` below 30,000 ms (the
platform's time-to-live today) is leased only to a host at this design's minor (Requirement 045-006), which reads the time-to-live as
a duration: an older host sets its interval from the lease's expiry on its own clock, clamped to 10 s, and a clock off by a few
seconds would leave it no margin. At or above 30,000 ms any host is served, as today. The cost of each beat is one run transaction:
today a lease beats every 10,000 ms (a third of 30,000 ms, at the 10 s clamp), 0.1 beats per second; at the minimum a 1.9 host beats
every 6,000 ms (a third of 18,000 ms), 0.17 beats per second, five thirds of today's rate per lease; the number of leases is bounded
by the activity's concurrency quotas. At the other end, a `heartbeatTimeoutMs` of 24 hours lets a dead worker's attempt hold its step,
and its concurrency slot, for that long: the author's choice, stated in the Studio's form.

**The source.** `heartbeatFrom: "host"` keeps today's host loop: it beats while the handler runs. A 1.9 host's interval is the shorter
of its configured interval and a third of `heartbeat_timeout_ms`, within 500 ms and 10 s, and its first interval is also no longer
than a third of the lease's time left at receipt, so a lease read late is still renewed in time (with an 18 s time-to-live read 6 s
after the claim, the first beat goes out 4 s later, 8 s before the expiry). `heartbeatFrom: "handler"` needs such a host: the release
writes `min_protocol_minor`, and the required `scheduleToStartMs` ends the step if no such host comes. Such a host sends the first
beat itself when the handler starts, and afterwards sends a beat at its interval only if the handler called `heartbeat()` since the
last beat sent; calls between beats are coalesced, so at most one beat per interval reaches the server however often the handler
calls. A call can wait up to one interval before its beat goes out, so a handler calls `heartbeat()` at least every two thirds of
`heartbeatTimeoutMs`, less a round trip; `sdks/SPEC.md` §6.4 says so, and says that an older host's fixed `heartbeatIntervalMs` must
be below a third of the shortest heartbeat timeout it serves. A host that stops beating also stops learning of a cancellation (a
heartbeat's answer carries `cancel_requested`); a handler on a node with `heartbeatFrom: "handler"` that wants to be cancellable
calls `heartbeat()`.

**Feature 019's heartbeat details.** Feature 019 gives `heartbeat(details)` an argument whose details ride on the next scheduled
beat, from its own minor (its checkpoints, `HeartbeatRequest` fields 9 and 10). It is the same call: a call with details is a handler
heartbeat, and the coalescing keeps the latest details, as 019 says ("latest wins"). 019's detached heartbeat timeout is its own; a
detached attempt ignores `heartbeatTimeoutMs`, and its maximum duration is bounded by this design's attempt deadline.

### 6. The retry decision

One decision function, called by `end_attempt` (root steps) and `fail_scoped_attempt` (scoped and path instances) in
`crates/domain/src/execution/command.rs`, replaces their third rule with rows 3 to 7 of this table; rows 1 and 2 are their first two
rules, unchanged. The first row that applies decides, and the event records which (`retryRule`):

| Row | When | Decision | Journal |
|---|---|---|---|
| 1 | the code leaves the outcome unknown | reconciliation or review (feature 037) | as today |
| 2 | the step was asked to stop | `step.cancelled` | as today |
| 3 | the worker said `retryable = false` | final | `step.failed`, `retryRule: worker` |
| 4 | the code is in `nonRetryableErrorCodes` | final | `step.failed`, `retryRule: nonRetryableCode` |
| 5 | the step's attempt count is at or above the effective `maxAttempts` | final | `step.failed`, `retryRule: attemptsSpent` |
| 6 | the next attempt's time (`db_now` plus the delay) is after the close cutoff | final, code `SCHEDULE_TO_CLOSE_TIMEOUT` | `step.failed`, `retryRule: closeDeadline`, `lastAttemptCode`, and `delaySource` when the worker sent a delay |
| 7 | otherwise | retry | `step.retry_scheduled`, `retryRule: base` or `byErrorCode`, `delaySource: policy` or `worker` |

**The effective policy** takes each field from the code's entry when the entry names it and from the base policy otherwise; an entry
without `maxDelayMs` gets the longer of the base's maximum delay and the entry's initial delay. Each effective policy is built
through `RetryPolicy::new` at publication, so the domain's own invariant refuses an invalid one. The step's attempt count is shared
by every code, and the exponent of an exponential delay is the step's attempt number: with a base `maxAttempts` of 4 and an entry
`{codes: ["RATE_LIMITED"], maxAttempts: 10}`, a step whose first three attempts failed with other codes and whose fourth failed
`RATE_LIMITED` is retried (4 is below 10), and has up to six more attempts while each fails with that code.

**The delay** is the policy's computed delay for that attempt, with its jitter sample; when the worker sent `next_retry_delay_ms`,
the longer of the two, without further jitter. A code outside the list grammar, which only a client outside the six SDKs can send,
matches no entry, and the base policy decides. A compensation's failure policy (Architecture decision 0076) is applied after rows 3,
4 and 5, exactly as after a known non-retryable failure today. Row 6 uses the cutoff, so a retry is not scheduled into a window too
short to run; when the retries pass lags past the cutoff, it ends the step (Technical Plan 4).

The unhandled exception of an SDK is `ACTIVITY_ERROR`, which an author may name. Requirement 045-015 lets a worker map its own exception
classes to codes at registration (for example `CardDeclinedError` to `CARD_DECLINED`), so that an exception the handler did not catch
still reaches the policy with its own code; that is how a Temporal user's `nonRetryableErrorTypes` of class names is written here.

The close cutoff enters the domain at row 6, in the lease decision, and in the commands the passes call; each compares `db_now` with a
value recorded in an event. A replay applies the journaled decisions; it never decides again, so the jitter sample and the clock do
not need to be reproduced.

### 7. The races (AGENTS.md rule 9)

Every row is decided under the run lock, which every transition of a step's status takes (constitution principle I); the loser
commits nothing or a counted no-op. A claim locks a run with `SKIP LOCKED` and a pass with `Lock::Wait`, so the claim rows have three
tests each: no lock held, the deadline at `db_now` less and plus a margin; the claim's transaction held while the pass runs (the pass
then finds `LEASED`); and a barrier between the candidate query and the claim's run transaction, inside which the deadline passes (the
claim then skips the step and leases its sibling).

| Race | Resolution |
|---|---|
| a claim and the start deadline | the lease decision on `db_now`: before the deadline the lease commits and the pass's check finds `LEASED`; at or after, the decision skips it and the pass ends it |
| a claim and the close cutoff | the same, with the cutoff |
| two `READY` steps of one run, one past its deadline or needing a newer host | the lease decision leases the other only |
| a completion and the attempt deadline (either cause) | first commit wins, as today; a late report is `STALE_ATTEMPT` |
| a renewal and the heartbeat timeout | first commit wins, as today |
| a failure's retry and the close cutoff | decided in the failure's transaction (row 6), at the cutoff less one microsecond, at it, after it |
| the retries pass and the close cutoff | the retries pass ends a `RETRY_WAIT` step past the cutoff instead of making it claimable |
| a gate's grant, the gate's queue timeout and the close cutoff | one deadline per waiter, the earlier, in 022's one timeout routine; the grant refuses a waiter past its cutoff |
| a lease withdrawal and the start deadline | the withdrawal writes the later of the old deadline and a fresh receive bound; a healthy worker claims again; a test withdraws a lease just after the deadline |
| a pause and each deadline | the pause removes the start deadlines; the resume writes a full window; the close cutoff is enforced during the pause; tests with a step that becomes claimable during the pause, two pauses, and path instances |
| a stop and a deadline | first commit wins; both end the step without an effect |
| a step in reconciliation past its close deadline | never ended by the pass: its close deadline was cleared |
| an operator's retry of a known failure past the cutoff | the retry clears the close deadline and writes a start deadline; the attempt is leased |
| a crash of the pass, two passes | nothing committed is repeated: the check re-reads the shard fence, status, generation and deadline |

A lost broker hint does not delay a deadline: the pass reads the database, not the broker (constitution principle VII); the minimum
`scheduleToStartMs` of 10,000 ms is two stream windows of 5 s, so a worker that is up claims the step without its hint. A restart
loses nothing: every deadline is in the journal and in a column.

### 8. Protocol 1.9 and the field register

The register of `sdks/SPEC.md` §9 on `develop` stops at 1.7 (design 041, served from its Task 041-7, not yet built); 1.8 is reserved
for delivery acknowledgement by Owner decision 2026-11-21-6, on the branch of pull request #264, not merged. This design names 1.9,
the next free minor, and takes it, or the next free one, when its contract merges (Owner decision 2026-11-15-8); "1.9" below means
that minor. Minors are cumulative: a 1.9 host also carries 1.7 and the 1.8 host procedure, so the server and SDK work of 1.9 follows
that of 1.7 and 1.8 (Risks).

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

The field register across the drafts, so that no two take one number (recorded in drafts 019 and 031 on 2026-11-24):

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
  retry options `nonRetryableErrorCodes`, `byErrorCode`. Each validates what the server validates (Requirements 045-001 and 045-002),
  with a shared fixture of the overlay rule, and raises the same reason before anything is sent.
- **Worker registration.** A map from exception class to failure code (TypeScript: a constructor; Go: an `errors.As` target; Python: a
  class; C#: a `Type`; Java: a `Class`; Rust: a function on the error), consulted before an unhandled error becomes `ACTIVITY_ERROR`; a
  code outside `[A-Z0-9_]{1,64}` is refused at registration. Local only: nothing on the wire changes.
- **Host at 1.9.** The interval of Technical Plan 5; `heartbeat_from_handler` switches to the handler source; `ActivityFailure` takes an
  optional `nextRetryDelay`, sent as `next_retry_delay_ms` (raised locally as `VALIDATION_FAILED` out of range).
- **Conformance** (`tests/contract/sdk-conformance/`): the harness gains the test-only controls `elapseStartDeadline` and
  `elapseCloseDeadline` (as `expireLease` is today) and the worker behaviours `stopHeartbeat` and `failWith {code, nextRetryDelayMs}`.
  Scenarios: `activity-timeouts` (the three timeouts, at each SDK's accepted minor below 1.9, and at 1.9), `retry-by-code`,
  `heartbeat-from-handler` (1.9 only); the same scenarios in six languages (AGENTS.md rule 8).

### 10. REST and the Studio

`specs/001-platform-foundation/contracts/openapi.yaml`, additive: `RuntimeNode` (the step view) gains `startDeadlineAt`,
`closeDeadlineAt`, `attemptDeadlineAt` and `attemptDeadlineCause` (`startToClose` or `scheduleToClose`), and `RuntimeGraph` gains
`serverTime`, against which a client computes the time left; the publication reasons of Technical Plan 1 are added to the problem
body.

The Studio (`frontend/apps/studio/src/features/workflows/fields.tsx` and the run view in `features/runs/`): the four fields and the
retry lists in the inspector with the server's validation, and two warnings ("A dead worker keeps this step, and its concurrency slot,
for up to the heartbeat timeout" when it is above one hour; "Only hosts at protocol 1.9 or later can run this step" for a node of
Requirement 045-006); on a step of a run, its deadlines with the time left against the view's server time (the projection's lag
shown, principle VI) and, for each timeout, one sentence of what happened and what the platform does next, from a map of codes to
sentences that Task 045-14 adds ("No lease of the attempt started in time: the step failed and its failure route ran"; "The attempt
lost its heartbeat: the outcome is unknown, so the platform is asking `findExport`"). Types follow `openapi.yaml`
(`npm run api && npm run check:api`).

### 11. Telemetry and audit

- Metrics, closed labels only: `step_timeouts_total{kind}`, counted where the platform decides each (the `step_deadlines` pass, the
  retries pass, the failure's transaction for row 6, the lost-lease pass), where `kind` is `schedule_to_start`, `schedule_to_close`,
  `start_to_close` (an attempt deadline), `heartbeat` (the lapse of a node's own `heartbeatTimeoutMs`) or `lease` (the lapse of the
  platform's time-to-live); `step_retry_decisions_total{rule}` with the seven rows' rules; `step_deadline_lag_seconds` (a histogram,
  from the deadline to the pass's commit); `step_deadline_pass_noops_total{kind}`; `step_claim_skips_total{reason}`.
- Spans: the pass's transaction carries `sagat.deadline.kind`.
- Audit: publication of a version with these fields is audited as every publication. A timeout is a system transition, journaled and
  not audited, as a lease expiry is today.
- Logs: closed values only (a rule, a kind, a reason); a worker's code stays in the journal.

### 12. What this design adds to the journal and the codes

- **Two known-failure codes**, `SCHEDULE_TO_START_TIMEOUT` and `SCHEDULE_TO_CLOSE_TIMEOUT`, in `FailureCode`, in
  `specs/045-activity-timeouts/contracts/errors.md` and in the Studio's sentences. A worker cannot send them (Requirement 045-019).
- **Optional event fields**, in `specs/045-activity-timeouts/contracts/journal-events.md` (written by Task 045-1, beside the journal
  lists of `specs/002-durable-execution/data-model.md` and `specs/018-schedules-bulk-operations/contracts/journal-and-events.md`): the
  deadlines (microseconds since the epoch, as `StartDelayed {until_us}` does) on `step.released`, `step.retry_due`,
  `step.retry_requested`, the body admissions, 022's grant events and `step.lease_withdrawn`; `attemptDeadlineUs` and
  `attemptDeadlineCause` on `step.leased`; `pausedAtUs` on `run.paused` and `resumedAtUs` on `run.resumed`; `deadline`, `retryRule`,
  `lastAttemptCode` and `delaySource` on the events of Technical Plans 4 and 6; `generation` on a `step.failed` with no fence.
- **New transitions of the reducer**, root and scoped: `step.failed` from `READY` and from `RETRY_WAIT` (no fence, guarded by the
  generation). An older build refuses them, which is why the requirements are a floor for a rollback (Requirement 045-017).
- **Reset** (feature 018) needs no rule: a reset point is not offered while any step is open (`ResetBlock::OpenWork` in
  `crates/domain/src/execution/reset.rs`), so a copied event never makes a step claimable in the new run, whose steps are released
  afresh at the reset's time. A test checks the new run's fresh deadlines.
- Unchanged: the outcome-unknown list, the run-level deadline, the wait and approval timeouts, and every compensation and
  reconciliation rule.

### 13. Upgrade, rollback and runs in flight

- A version of level 0 is run exactly as today, at every minor, through every rolling upgrade (principle IV). That includes a document
  stored before this design whose root `activity` or `script` already carries one of the new keys (those schemas are open): its level
  is 0, so the keys are ignored; the upgrade's inventory lists such stored nodes so that an operator can republish them.
- The operator records `activity-timeouts-v1`, and `activity-timeouts-protocol-v1` once every engine offers this design's minor
  (`platform_requirement_activate`, Architecture decision 0082), for control planes and engines; until then publication of a version
  of that level answers `422` with the reason `ACTIVITY_TIMEOUTS_NOT_ACTIVE`. Once recorded, a binary without the capability serves
  nothing (the closed gate).
- **The publication hold.** A `SECURITY DEFINER` function, `platform_requirement_hold(capability text) RETURNS boolean`, with `SET
  search_path = pg_catalog, pg_temp`, schema-qualified names, the capability checked against the closed list, `REVOKE ALL ... FROM
  PUBLIC` before `EXECUTE` is granted to `sagat_app`, takes a shared transaction advisory lock on the requirement's key and returns
  whether the requirement is recorded, read after the lock; the operator's `platform_requirement_activate` and
  `platform_requirement_remove` take the same key's exclusive lock first. Advisory locks queue in order, so a stream of publications
  cannot starve the operator, and the runtime role takes no row lock on the registry. Publication calls the hold as its last statement
  before it commits, under a `lock_timeout`. This refines Architecture decision 0082 decision 1 ("the runtime role writes nothing and
  locks nothing"): it now holds an advisory lock for one transaction; the registry's privilege test is updated. Features 020, 029 and
  031 may use the same function.
- **The floor for a rollback.** `platform_requirement_remove` refuses either requirement while any workflow version of the level that
  needs it exists, retired included: an older build cannot replay those runs' journals or honour those versions. One indexed
  predicate on `workflow_versions.activity_timeouts_level`, which publication writes in the statement that inserts the version.
- Released 0.2 scenario (`scripts/test-release-upgrade-profile.py`): runs started on 0.2 finish on 0.3 unchanged; a 0.2 SDK host is
  served at its minor; a version of level 2 is never leased to it.

### 14. Other features

| Feature | Status on 2026-11-24 | What this design asks of it, or gives it |
|---|---|---|
| 006, protocol 1.8 (Architecture decision 0099) | accepted, on the branch of pull request #264, not merged | 1.9 follows 1.8 in the server and in each SDK; this design refines its decision 5 (the next lease of a withdrawn attempt keeps, or freshly extends, the start deadline) |
| lease withdrawal (Architecture decision 0098) | on the branch of pull request #260, not merged | the withdrawal writes the start deadline (Technical Plan 3) |
| 029 (custom roles and folders) | approved draft | the version level's predicate sits beside its own publication checks; it may use the hold function |
| 018 (schedules, resets) | built | no reset rule is needed (Technical Plan 12) |
| 019 (worker deployments) | approved draft | its `heartbeat(details)` is a handler heartbeat; a detached attempt ignores `heartbeatTimeoutMs`; its priority order is unchanged by the start deadline (Gap 045-002); field numbers above |
| 020 (visibility) | approved draft | the deadlines, `attemptDeadlineCause` and the deadline causes are additive facts it may list; it may use the hold function |
| 022 (flow control) | specified, being built | the close cutoff is one of a waiter's deadlines in its one timeout routine, with the cause column; a granted step ended by a deadline returns its slot and permit |
| 030 (cross-namespace services) | approved draft | its operation deadlines use the same names and clock (`scheduleToCloseMs`, `scheduleToStartMs`) |
| 031 (client-side encryption) | approved draft | `error_code` and `safe_message` are never encoded; it may use the hold function |
| 037 (compensation safety) | built | unchanged; its refusal to retry an unsettled step stands |
| 041 (step logs, protocol 1.7) | approved, being built | 1.9 follows 1.7 |
| 044 (workflow as code) | being designed | the options of an activity call map to these fields |
| ADR-0029 decision 6 | accepted | a handler-sourced lease is not renewed continuously by the host; refined by Architecture decision 0110 |

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A Temporal user expects a timed-out attempt to retry | The Studio's sentence, the guide's comparison table and `reconcileWith` as the documented way to go on automatically |
| Protocols 1.7 or 1.8 slip, so 1.9 cannot ship | Every version of level 1 works at 1.8 and below (an older host beats from the lease's expiry); `activity-timeouts-protocol-v1` is listed only by a build that offers 1.9, so publication refuses a version of level 2 until then |
| Building two partial indexes on a large `steps` table holds a lock that blocks writes while it reads | The upgrade notes give the measured time on the scale fixture of `tests/performance/` (Task 045-17); an operator with a very large table builds them `CONCURRENTLY` beforehand, which `IF NOT EXISTS` skips and the validity check verifies |
| Once a version uses these fields, rollback below this release is refused | Stated in the release notes and the runbook; the operator records the requirement only when the fleet is upgraded and a rollback is no longer planned |
| Security: a worker asks for a short delay to retry faster than the author's policy allows | The delay is the longer of the worker's and the policy's, and the journal says it came from the worker |
| Security: a worker reports a platform timeout code to claim that nothing ran | The server records it as `WORKER_ERROR`; timeouts are counted where the platform decides them |
| Security: one tenant floods deadlines or short heartbeats | The pass is fair across tenants with its own window; the minimum heartbeat timeout bounds a lease to five thirds of today's beat rate; an older host is never given a timeout below 30 s |
| Security: a requirement is removed while a publication that needs it commits, or publications starve the operator | The publication holds the requirement under a fair advisory lock until it commits |
| A host clock far from the database's distorts an older host's interval | An older host is served only nodes whose time-to-live is at least today's 30 s |
| The deadline pass falls behind under load | Its lag is a histogram with an alert in the runbook, and its criterion includes a one-tenant flood |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Every deadline is computed by the domain, journaled, written to a column and acted on in one run transaction under the run lock; the lease decision and the pass's re-check make a lease and a timeout exclusive; the retry decision is journaled with its rule. The principle's audited no-op is for a timeout racing a signal, which this design does not change; here a late worker report loses as today (`STALE_ATTEMPT`, recorded with its trace), a pass that loses is counted (`step_deadline_pass_noops_total`) and a claim's skip is counted (`step_claim_skips_total`), since no caller made them |
| II. Modular domain, ports and adapters | Pass | New types in `crates/domain` without `serde_json`; the domain sees host features, not protocol numbers; parsing in `infra-ir`; SQL in `infra-postgres`; the pass in `scheduler`; no new crate edge (`scripts/check-architecture.py`) |
| III. External effects are at-least-once; sagas compensate | Pass | A deadline that ends a live attempt is an unknown outcome (reconciliation or review); only a deadline with no live attempt is a known failure; compensation and reconciliation nodes are untouched; the close deadline is cleared when a person or a reconciliation takes over; an unsettled step is never retried |
| IV. Compatibility is versioned and pinned | Pass | Optional IR fields read only for versions of level 1 or 2, protocol 1.9 additive, optional journal fields written only for those versions, two fleet requirements with a floor for a rollback; versions of level 0 are unchanged; no code changes meaning |
| V. Code, tenant boundary and secrets are isolated | Pass | No code runs on the server; no new table; one grant of a definer function with a fixed search path, revoked from `PUBLIC`; a worker's delay and codes are bounded and cannot claim a timeout |
| VI. Observability and audit are part of the feature | Pass | Closed-label metrics counted where the platform decides, spans, deadlines and their cause in the projection with its lag |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Deadlines do not depend on a broker hint |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Barrier tests per race, crash and duplicate pass tests, restart, lost hint, pause, withdrawal, conformance in six languages, contract tests, a parity test and a static check; gaps registered |
| Delivery Constraints | Pass | No new technology; single region; two migrations; the registry of Architecture decision 0082 |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Six columns and two indexes on `steps`, one column on `workflow_versions` | A deadline must be a durable fact the claim and a pass can read on the database clock; the cutoff and the window keep the claim's hint, the pass and the pause exact without reading the definition; the version's level makes old documents safe and the rollback floor cheap | Timer rows per deadline: one more row and statement per attempt on the hot path; computing the floor in SQL: it depends on the node |
| `min_protocol_minor` on `steps` and in the candidate query | A step that needs 1.9 must never reach an older host, and such rows must not fill an older host's candidate window | Refuse to publish until every host is at 1.9: the server cannot know the hosts |
| `byErrorCode` beside `nonRetryableErrorCodes` | The owner's brief asks for a retry policy per error type; a rate limit and a bug need different patience, and a worker's delay cannot give a rate limit more attempts than a bug | Only the list: forces an author to choose between giving up and retrying a provider faster than it accepts |
| `heartbeat_timeout_ms` on the lease although an older host derives its interval from the expiry | The expiry is an instant on the database's clock; a duration lets a host beat in time whatever its clock says, as protocol 1.8 chose for its own fields | The instant alone: a host whose clock is off by more than the window beats too late |
| A second requirement for steps that need 1.9 | Their steps can only run on 1.9 hosts, which need 1.7 and 1.8 first | One requirement: such a version could publish before any server offers 1.9 and never be leased |
| A definer function with a fair advisory lock for the publication hold | Without it a removal can commit between a publication's check and its commit, and a row lock would let publications starve the operator | No hold: an older binary could serve a version it cannot honour |
| A pause that removes and rewrites start deadlines | A paused run is an operator's hold; without it a pause fails steps, or paused rows sit at the head of the index | Shift each deadline by the paused time: wrong for steps that became claimable during the pause, and needs a payload per step |

## Alternatives Considered

- **Temporal's semantics (retry after every timeout).** Rejected: it repeats an effect whose outcome is unknown, which principle III
  and Requirement 037-009 forbid (Architecture decision 0110, option 2).
- **Timer rows for deadlines.** Rejected: one more row and one more statement per attempt on a hot path that design 038 measured
  (Architecture decision 0110, option 3).
- **A new `HEARTBEAT_TIMEOUT` code.** Rejected: it would mean what `LEASE_EXPIRED` means, grow the outcome-unknown list and its copies,
  and give every consumer two codes to match; the `deadline` field says which bound passed (Architecture decision 0110, option 4).
- **A new error-type field.** Rejected: `error_code` already is the type; two names for one fact would fork what six SDKs send
  (Architecture decision 0110, option 5).
- **Patterns in the retry lists.** Left out: exact codes keep the decision auditable; addable later without breaking a version
  (Architecture decision 0110, option 6).
- **Only a non-retryable list, or only per-code policies.** Rejected: the list is what a Temporal user knows; the per-code policy is
  what a rate limit needs (Decision 045-005).
- **A heartbeat from the host only, or from the handler only.** Rejected: the host alone cannot see a stuck handler; the handler alone
  would break every existing handler that never calls `heartbeat()` (Decision 045-004).
- **Schedule-to-start counted from the release, gates included.** Rejected: a gate is a deliberate hold by the platform, and
  schedule-to-start exists to detect a missing worker; the close deadline covers the gate's time.
- **Heartbeat timeout as a second clock beside the lease.** Rejected: two expiries for one attempt would race each other; the lease
  expiry is the heartbeat timeout.
- **The close deadline as a second exit for a gate waiter.** Rejected: Architecture decision 0089 has one timeout routine per waiter,
  and two exits orphan waiter rows; the close cutoff joins the waiter's deadline instead.
- **A minimum heartbeat timeout of 10,000 ms.** Rejected: below 18,000 ms a lease is never provisional under protocol 1.8 and loses
  its protection against a delivery lost on the wire (Decision 045-024).
- **A replay floor that scans journals.** Rejected: the new transitions reuse an existing event type, and a scan of every tenant's
  journal does not fit the operator's statement timeout; the version's level is one indexed predicate.
- **An attempt's time left on every lease (`attempt_time_left_ms`).** Dropped: no requirement needs it; protocol 1.8 already gives it
  to a flagged lease.

## Non-Functional Requirements

Non-functional requirements 045-001 to 045-004 are stated under Requirements. Their tests: the budget lines of
`integration_claim_transition_efficiency` (unchanged, plus the line with unclaimable rows) for 045-001; the lag measurement of Task
045-17 for 045-002; the privilege audit of `integration_platform_foundation` and the suites as the runtime role for 045-003; the
released 0.2 scenario and the protocol baseline test for 045-004.

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Test first: each task that changes behaviour adds its tests and sees them fail for the right reason before the code (AGENTS.md rule
18). Documentation first: Task 045-1 changes the contracts and documents before any code (Owner decision 2026-11-20-8). The tasks are
ordered into slices for two builders whose lanes never touch the same crate at once. Each server slice is one pull request that
changes every caller of a signature it changes, so the workspace builds after each.

| Slice | Lane | Tasks | Crates and folders | Depends on |
|---|---|---|---|---|
| 0 | either | 045-1 | `specs/001-platform-foundation/contracts/`, `specs/045-activity-timeouts/contracts/`, `sdks/SPEC.md`, `docs/` | this design approved |
| 1 | server | 045-2, 045-3, 045-4 | `crates/domain`, and its callers in `crates/application`, `crates/infra-postgres` (the event codec and the exhaustive matches) and `crates/infra-ir` | slice 0 |
| 2 | server | 045-6, 045-7, 045-5 | `crates/infra-postgres`, `crates/infra-ir`, `crates/application` (the store ports, the publication gate, `LeaseGrant` and `LeaseRenewal`) | slice 1 |
| 3 | server | 045-8, 045-9, 045-11, 045-20 | `crates/scheduler`, `crates/application`, `crates/api-http`, `apps/control-plane`, `apps/execution-engine`, `scripts/`, `tests/fault/activity-timeouts/` | slice 2 |
| 4 | server | 045-19 | `crates/api-grpc`, `crates/application` | slice 3; the server side of protocols 1.7 (Task 041-7) and 1.8 |
| 5 | server | 045-10, 045-16, 045-18 | `crates/application`, `crates/infra-postgres`, `tests/integration/activity-timeouts/`, `tests/fault/activity-timeouts/` | slice 3; 045-10 also on feature 022's gates; the withdrawal parts on pull requests #260 and #264 |
| 6 | edges | 045-12 | `sdks/<language>` builders (six), `tests/contract/sdk-conformance/` | slice 0; its conformance run needs slice 3 |
| 7 | edges | 045-14 | `frontend/apps/studio` | slice 0 (types from `openapi.yaml`); its journeys need slice 3 |
| 8 | edges | 045-13 | `sdks/<language>` hosts (six) | slice 6, the 1.7 and 1.8 host work in each SDK, and slice 4 |
| 9 | either | 045-15, 045-17 | `docs/`, `tests/performance/` | slices 5 and 8 |

- [ ] T045-1 Write the contracts and documents first: the IR fields and their reasons on `activity`, `script`, `bodyActivity`,
  `bodyScript`, `retry` and `bodyRetry` (`workflow.schema.json`); `specs/045-activity-timeouts/contracts/errors.md` (the reasons and
  the two codes) and `journal-events.md` (the event fields and the new transitions); protocol 1.9 (`worker.proto`,
  `worker.baseline.json` re-pinned after reading its diff); `openapi.yaml` (`RuntimeNode`, `RuntimeGraph.serverTime`, the reasons);
  `sdks/SPEC.md` (§6.4 the heartbeat source, the interval and the fixed-interval rule, §6.5 `nextRetryDelay`, the exception map and
  the two codes a worker cannot send, §9 the register row for 1.9 and the field register); `docs/architecture/execution-semantics.md`
  (the deadlines and the retry table); the "Refined by" pointers in Architecture decisions 0082, 0029 and, on their branches, 0099;
  and the rows of `docs/engineering/architecture-principles.md` for the checks of Task 045-20. The contract suites fail on the new
  baselines until slices 2 and 3.
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-014, Non-functional requirement 045-004
- [ ] T045-2 Domain types: `StepTimeouts` on `Node`, `RetryRules` with `for_code` and `CodeRule`, the effective policies built through
  `RetryPolicy::new` with the overlay rule, `activity_timeouts_level`, and their validation (bounds, contradictions, the close floor
  plus 1,000 ms, reserved and duplicated codes, invalid overrides). Unit tests first, one per refusal and per boundary value (10,000
  and 9,999; 31,622,400,000 and one more; 18,000 and 17,999; 86,400,000 and one more; the close floor plus 1,000 ms and one less; 32 and
  33 codes; 16 and 17 entries; empty lists; 64 and 65 codes in all).
  Refs: Requirement 045-001, Requirement 045-002
- [ ] T045-3 The retry decision function of Technical Plan 6, called by `end_attempt` and `fail_scoped_attempt`, with the worker's
  delay, the close cutoff, and `retryRule`, `lastAttemptCode` and `delaySource` on the events. A unit test per row for a root step, a
  scoped instance and a path instance, first; the existing retry tests unchanged.
  Refs: Requirement 045-007, Requirement 045-008
- [ ] T045-4 Events and state: the fields of Technical Plan 12, computed by the domain from `db_now` and truncated to microseconds,
  written only for a version of level 1 or 2; `StepState`'s new fields and `restore`; the clearing rules; the lease predicate with host
  features and `ExecutionError::NotClaimable`; the two timeouts that end a step with no live attempt, guarded by the generation;
  `step.failed` from `READY` and `RETRY_WAIT` in both reducers; the pause and the resume; replay equals projection with and without the
  fields (a corpus of journals written before this design), and the new run of a reset with fresh deadlines.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-005, Requirement 045-013, Requirement 045-021
- [ ] T045-5 The publication gate, after Task 045-6: `infra-ir` parses the fields (with `retry_at` for the entries) and maps the domain's
  refusals to pointers; the application's publication (`publish` in `crates/application/src/definitions.rs`) computes the version's
  level, refuses `ACTIVITY_TIMEOUTS_NOT_ACTIVE` through the store's `hold_requirement(capability) -> bool` and writes the level with
  the version; a version of level 0 is read without the fields. An IR fixture per reason, and a stored document of level 0 that
  carries the keys.
  Refs: Requirement 045-001, Requirement 045-002, Requirement 045-017
- [ ] T045-6 The two migrations of Technical Plan 2 (the columns, the checks and their validation, the indexes with their validity
  check, the capabilities in the three closed lists, the hold function with its grant and the replaced operator functions, the down
  migration's refusals, the storage classification) and the event codec of the new fields; the capability variants in
  `crates/application/src/instances.rs`, with its test that keeps the schema and `specs/018-schedules-bulk-operations/data-model.md`
  equal. `integration_platform_foundation` (migrations as the runtime role, the privilege audit, the registry's privilege test updated)
  first.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-013, Requirement 045-017, Non-functional requirement 045-003
- [ ] T045-7 The step statements of Technical Plan 2 write the event's deadlines and `min_protocol_minor`; the row-shape predicates and
  the loaders include the columns; the candidate query's forward branches and `RUN_FOR_CLAIM` carry the conditions; the claim applies
  the lease predicate where it builds its list; the five places that set the time-to-live take the node's; the pause and resume
  statements; the deadline scans of the store. The budget lines of `integration_claim_transition_efficiency` must pass unchanged (seen
  passing before and after), plus a new line for a poll with unclaimable rows ahead of claimable work; `scripts/check-capacity-activation.py`
  passes; a store test per column. The withdrawal's start deadline is built by whichever of this design and pull request #260 lands
  second.
  Refs: Requirement 045-003, Requirement 045-004, Requirement 045-005, Requirement 045-006, Requirement 045-021, Non-functional requirement 045-001
- [ ] T045-8 The `step_deadlines` pass in `crates/scheduler` (`MaintenanceStore::due_step_deadlines`, `Condition::StepDeadlinePassed`,
  its report field, its registration in the tick and its lag gauge, root and path transactions, the metrics and the no-op counter);
  the retries pass ends a `RETRY_WAIT` step past its cutoff; the lost-lease pass records the deadline cause (`LostLease.deadline`,
  `Expiry::AttemptTimeout {deadline}`). The `fault_activity_timeouts` suite (its `[[test]]` entry in `tests/Cargo.toml`, its entry
  in `scripts/integration-suite-durations.toml`, its `main.rs`) with the crash, duplicate and pause tests, first.
  Refs: Requirement 045-004, Requirement 045-009, Requirement 045-018
- [ ] T045-9 `crates/application`: `fail()` records a worker's `SCHEDULE_TO_START_TIMEOUT` or `SCHEDULE_TO_CLOSE_TIMEOUT` as
  `WORKER_ERROR` at the parse; the delay joins the report's fingerprint when present (a separator byte and its decimal digits) and
  reaches the path report; the operator's retry of a known failure writes a start deadline and clears the close deadline. Tests per
  rule first.
  Refs: Requirement 045-008, Requirement 045-019, Requirement 045-020
- [ ] T045-10 The close cutoff at feature 022's gates: the waiter's deadline as the earlier of the queue deadline and the cutoff, with
  the `deadline_cause` column on 022's two waiter tables; the dequeue events and `step.failed` in 022's one timeout routine; the
  eligibility rule; the return of a granted step's slot and permit in 022's lock order; the start deadline on the grant events; the
  races of Technical Plan 7; the runtime role's grants on 022's tables checked and every statement scoped by `tenant_id`. Built by
  whichever of 022's gate and this design lands second (the counterpart task in `specs/022-flow-control-step-policies/tasks.md`).
  Refs: Requirement 045-011
  Owner-gate: OD-2026-11-24-1
- [ ] T045-11 `crates/api-http` and the fleet requirements: `RuntimeNode`'s deadlines and cause and `RuntimeGraph.serverTime`, the
  publication reasons in the problem body (contract test first, `contract_rest_durable_execution`); the capability
  `activity-timeouts-v1` in both process kinds and the floor for a rollback (registry tests first, in `apps/control-plane/src/registry.rs`
  and `apps/execution-engine/src/settings.rs`, design 043's capability work as the template).
  Refs: Requirement 045-014, Requirement 045-017
- [ ] T045-12 The six SDK builders, validation, the shared overlay fixture and the exception-to-code map, with the harness's new controls
  and behaviours and the conformance scenarios `activity-timeouts` (at each SDK's minor below 1.9) and `retry-by-code`; each SDK's
  checks of `sdks/AGENTS.md`.
  Refs: Requirement 045-015
- [ ] T045-13 The six SDK hosts at 1.9: the interval and first interval of Technical Plan 5, the handler source with coalescing,
  `nextRetryDelay`; the conformance scenario `heartbeat-from-handler` and `activity-timeouts` at 1.9, with a lease received 6 s after
  its claim.
  Refs: Requirement 045-006, Requirement 045-016
  Owner-gate: OD-2026-11-24-1
- [ ] T045-14 The Studio: the inspector's fields, validation and two warnings, the map of timeout codes to sentences, the deadlines on a
  run's step against the view's server time; component tests first, then the journeys.
  Refs: Requirement 045-014
- [ ] T045-15 Documents for people: a guide page on timeouts and retries with the Temporal comparison, the unknown-outcome rule and the
  difference from feature 019's `asyncCompletion.heartbeatTimeoutMs`; the runbook `docs/runbooks/step-timeouts.md` (the lag alert, the
  no-op and skip counters, the index build, the two requirements and the floor for a rollback); `docs/sdk-compatibility.md`; the
  release notes; `contract_developer_experience` regenerated only after reading its diff.
  Refs: Requirement 045-014, Requirement 045-018
- [ ] T045-16 The races of Technical Plan 7, each with the tests its row names, Requirement 045-012's test that a compensation and a
  reconciliation are unaffected, and the crash, duplicate, restart and lost-hint cases, in `fault_activity_timeouts` and
  `integration_activity_timeouts` (its `[[test]]` entry and `main.rs`); every suite also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. The
  withdrawal cases are built by whichever of this design and pull requests #260 and #264 lands second.
  Refs: Requirement 045-010, Requirement 045-012, Requirement 045-021
- [ ] T045-17 Measure Non-functional requirement 045-002 (1,000 deadlines in one second, and a one-tenant flood of 100,000), the beat rate
  at the minimum heartbeat timeout, and the index build on the scale fixture of `tests/performance/`; record them with their
  environment in `tests/performance/reports/`.
  Refs: Non-functional requirement 045-001, Non-functional requirement 045-002, Success criterion 045-005
- [ ] T045-18 Released 0.2 and mixed-fleet scenarios: runs from 0.2 finish unchanged, a 0.2 host is served at its minor and never leased
  a step of a level-2 version, a stored level-0 document with the keys runs as today, the requirement gates, the floor for a rollback,
  rollback before the requirements.
  Refs: Requirement 045-017, Non-functional requirement 045-004
- [ ] T045-19 Protocol 1.9 on the server (`crates/api-grpc`, `crates/application`): negotiation offered only with 1.8, the lease fields,
  `next_retry_delay_ms` validated, refused below 1.9 and passed to the domain, and the capability `activity-timeouts-protocol-v1` listed
  only by a build that offers it. Contract test of the descriptor first (`contract_grpc_durable_execution`).
  Refs: Requirement 045-006, Requirement 045-008, Requirement 045-016, Requirement 045-017, Non-functional requirement 045-004
  Owner-gate: OD-2026-11-24-1
- [ ] T045-20 The fitness checks: a parity test of claim eligibility (the candidate query's hint against the domain's predicate over the
  same rows, at a fixed clock); a rule in `scripts/check-architecture.py`, with its fixtures, that the columns `start_deadline_at`,
  `close_deadline_at`, `close_cutoff_at`, `attempt_deadline_cause` and, for level-1 and level-2 versions, `attempt_deadline_at` are
  assigned only from event values or by the pause and resume rule, and never by a withdrawal statement other than from its event.
  Refs: Requirement 045-003, Requirement 045-013

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
`python3 scripts/check-architecture.py` with its fixtures, `python3 scripts/check-agent-config.py`,
`python3 scripts/check-design-docs.py`, `python3 scripts/check-plain-words.py --base origin/develop`, the unit tests of the crates it
changes and of those that depend on them, and the suites below that it changes or that read what it changes
(`scripts/select-test-targets.py`):

- `integration_activity_timeouts` and `fault_activity_timeouts` (new; every race, crash, duplicate, restart, pause, withdrawal and
  lost-hint case), also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`;
- `integration_durable_execution`, `integration_sagas_compensations` (the retry table, compensation untouched),
  `integration_workflow_primitives` (deadlines of steps in bodies), `integration_schedules_bulk_operations` (the reset's fresh
  deadlines);
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
each cause; a renewal against the heartbeat timeout; a withdrawal just after the start deadline; a gate's grant and queue timeout
against the close cutoff; a lagging retries pass against the cutoff; a pause against each deadline, with a step that becomes claimable
during it, two pauses and path instances; a step in reconciliation past its close deadline; an operator's retry of a known failure
past the cutoff, and the refusal of a retry of an unsettled step; a stop against each deadline; a restart with deadlines pending; a
lost `task.ready` hint before a start deadline. A test that fails is a defect to explain, not to retry.
