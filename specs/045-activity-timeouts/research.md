# Research register: design 045, activity timeouts and retry per failure code

The registers of [design 045](../../docs/designs/045-activity-timeouts.md): decisions (Decision 045-NNN), gaps (Gap 045-NNN)
and owner questions (Owner question 045-NNN). Append-only: an entry is never renumbered or deleted; a revised entry says so in
place. Every owner question below was decided under the owner's delegation (Owner decision 2026-11-20-6, widened by Owner decision
2026-11-24-4), each weighed over its options for the most architectural and business advantage, and is reported to the owner.

## What the code does today (2026-11-24, `origin/develop` at `546d5aa5`)

- A node's `timeoutMs` is an attempt deadline fixed at the claim: `steps.attempt_deadline_at`, written by the lease statement from
  the transition's database time, never extended by a renewal (`crates/infra-postgres/src/execution.rs`, `LeaseGrant.attempt_timeout`
  in `crates/application/src/execution.rs`).
- The lease expiry is the claim's or the last renewal's time plus `lease_ttl` (30 s by default, `crates/application/src/worker.rs`),
  the same for every node, capped by the attempt deadline.
- The lost-lease pass (`lost_leases` in `crates/infra-postgres/src/maintenance.rs`, `holds` in `crates/scheduler/src/passes.rs`)
  ends an attempt whose attempt deadline (`ATTEMPT_TIMEOUT`) or lease expiry (`LEASE_EXPIRED`) passed, re-checking under the run
  lock. Both codes leave the outcome unknown (`FailureCode::outcome_unknown_codes`, seven codes), so a declared activity goes to
  reconciliation or review (`end_attempt` in `crates/domain/src/execution/command.rs`, feature 037).
- `RetryPolicy` (`crates/domain/src/workflow.rs`) has `maxAttempts`, delays, strategy and jitter; a worker's `retryable = false`
  is the only way to make a failure final. `FailStep.error_code` is the handler's code (`[A-Z0-9_]{1,64}` from the SDKs,
  `sdks/SPEC.md` §6.5).
- A signal is delivered to the `waitEvent` steps waiting for its name, a copy each; otherwise `409` "the run is not waiting for
  that signal" (`deliver_signal`, `crates/application/src/runs.rs`). Architecture decision 0030 decision 17 and Architecture
  decision 0050 decision 4 say no signal is buffered; feature 018 lists buffered signals as out of scope.

## Decisions

- **Decision 045-001 — A deadline that ends a live attempt leaves its outcome unknown** (Owner question 045-001). Options: (a)
  Temporal's semantics, retry after the timeout; (b) the outcome-unknown rule of feature 037; (c) a per-node opt-in to retry after a
  timeout for activities the author declares idempotent. Chosen (b), delegate (Owner decision 2026-11-24-4): (a) and (c) repeat an
  effect whose outcome is unknown on the author's word alone, which constitution principle III ("absent or false risk metadata MUST
  NOT authorize repeat") and FR-037-009 forbid; `reconcileWith` already makes such a step go on automatically when the provider can
  say what happened, and that is the stronger product: a payment is never charged twice because a worker was slow.
- **Decision 045-002 — `SCHEDULE_TO_START_TIMEOUT` is final** (Owner question 045-002). Options: (a) final, the failure route;
  (b) retried by the policy; (c) retried only when listed in `byErrorCode`. Chosen (a), delegate (Owner decision 2026-11-24-4): a
  retry of an attempt that no worker took only waits again in the same queue, which a longer `scheduleToStartMs` says more
  honestly; the failure route is where a fallback (another pool, a person) belongs. Temporal also treats it as non-retryable.
- **Decision 045-003 — The retry lists name `error_code`; no new error-type field** (Owner question 045-003). Options: (a) a new
  `FailStep.error_type` beside `error_code`; (b) `error_code` is the type, and each SDK maps exception classes to codes locally. Chosen
  (b), delegate (Owner decision 2026-11-24-4): two names for one fact would fork what six SDKs send and what the journal records;
  the map gives Temporal users their class-name lists without putting class names on the wire.
- **Decision 045-004 — Two heartbeat sources, `host` by default** (Owner question 045-004). Options: (a) host only (today); (b)
  handler only (Temporal); (c) both, declared per node. Chosen (c), delegate (Owner decision 2026-11-24-4): `host` detects a lost
  process with no handler code and keeps every existing workflow's meaning; `handler` detects a stuck handler, which only the
  handler can show. A default of `handler` would break every existing handler that never calls `heartbeat()`.
- **Decision 045-005 — A retry policy per code, and the short non-retryable list** (Owner question 045-005). Options: (a) the list
  only (Temporal parity); (b) a policy per code only; (c) both. Chosen (c), delegate (Owner decision 2026-11-24-4): the list is the
  form a Temporal user knows and the common case; the per-code policy is what lets an author be patient with a rate limit and quick
  with a bug, which Temporal cannot express in one activity.
- **Decision 045-006 — `activity` and `script` nodes only** (in the root graph and in bodies, revised 2026-11-24 after the
  zoom-out review: `bodyActivity` and `bodyScript` get the fields too, since the scoped and path statements carry them) (Owner question 045-006). Options: (a) every leased node kind,
  including the managed database and lake runners; (b) `activity` and `script`; (c) `activity` only. Chosen (b), delegate (Owner
  decision 2026-11-24-4): these are the nodes whose workers the author operates and whose stalls the author must detect; the managed
  runners are platform processes with their own limits (`timeoutMs` of their connection, `LakeLimits`), and adding them is Gap
  045-003. Compensation and reconciliation are excluded by principle III (a deadline must not end a pending compensation).
- **Decision 045-007 — Pending signals move across continue-as-new** (moved, with buffered signals, to their own design on
  2026-11-24; kept here as the reasoning ADR-0111 records) (Owner question 045-007). Options: (a) discard them; (b)
  refuse the continuation while any is pending; (c) move those the successor's version declares buffered and discard the rest,
  journaled. Chosen (c), delegate (Owner decision 2026-11-24-4): (a) loses business messages silently, (b) lets a sender block a
  run's progress; (c) keeps every message a successor can consume and records the others, without user code draining a channel.
- **Decision 045-008 — Buffering is opt-in per version and per name** (moved, with buffered signals, to their own design on
  2026-11-24; ADR-0111 decision 1) (Owner question 045-008). Options: (a) buffer every signal of
  every run (Temporal); (b) opt-in. Chosen (b), delegate (Owner decision 2026-11-24-4): (a) changes the answer of `signalRun` for
  runs in flight and for callers who rely on the `409` today (constitution principle IV); (b) gives the same capability to every new
  version that asks for it.
- **Decision 045-009 — Where buffered signals belong** (Owner question 045-009; first decided "here", revised below to "a design of their own"). Options: (a) in design 044 (workflow as code);
  (b) a design of their own; (c) here. Chosen (c), delegate (Owner decision 2026-11-24-4): they are not covered by feature 018
  (explicitly out of scope); (a) would make the declarative mode wait for 044's determinism study; (b) is clean but takes another
  number while the 0.3.0 designs are being numbered; (c) shares this design's race with a wait's timeout, its fleet switch and its
  conformance scenarios, and its tasks form a slice that can be moved out unchanged.
  Revised 2026-11-24, after the zoom-out review (its Major 1): buffered signals go to a design of their own. Of the three reasons
  above, the shared race turned out not to exist (a wait that consumes at its entry has no timer to race), the fleet switch is
  a separate requirement and the conformance scenario is separate; only the numbering reason was left, which is administrative.
  Their design keeps ADR-0111 as its proposal, which now also weighs feature 004's `lookbackMs` path (the zoom-out review's
  Major 2). Delegate (Owner decision 2026-11-24-4); the coordinating session numbers it.
- **Decision 045-010 — The start deadline does not move on a lease withdrawal** (superseded by Decision 045-025, then by the third round's rule: a withdrawal extends it to a full window; the first text follows). A withdrawn lease (Architecture decision 0098,
  Architecture decision 0099) never started, so the attempt is the same attempt; resetting the deadline would let withdrawals
  postpone a timeout without limit.
- **Decision 045-011 — The minimum heartbeat timeout** (revised 2026-11-24 to 10,000 ms after the security, correctness, architecture and design reviews; the first text follows: it was 3,000 ms). A host beats at a third of it (1,000 ms), three beats per window,
  above the 751 to 889 ms ready-to-handler 99th percentile measured in `tests/performance/reports/sdk-streaming.md`. Below 18,000 ms a
  lease is never provisional (Architecture decision 0099 decision 2), which is correct.
- **Decision 045-012 — Field numbers.** `TaskLease` 26 and 27 and `FailStepRequest` 14 (revised 2026-11-24: the first draft also
  named `TaskLease` field 28, which is no longer used) for protocol 1.9, above every number the drafts
  and protocol 1.8 name; the register is in Technical Plan 8 of the design and is recorded in the research registers of drafts 019 and 031 on
  2026-11-24: 031 moves to `TaskLease` 25 and `WorkerCapabilities` 7, 019's `RegisterWorkerResponse` field moves from 4
  (taken by design 041) to 5.

- **Decision 045-013 — No `HEARTBEAT_TIMEOUT` code** (zoom-out review, Major 5, accepted). A lapse keeps `LEASE_EXPIRED` and its
  event records `deadline: heartbeat`; the outcome-unknown list and its two copies stay as they are.
- **Decision 045-014 — No `attempt_time_left_ms` on the lease** (zoom-out review, Major 4, accepted in part). No requirement needs it.
  `heartbeat_timeout_ms` stays: the lease's expiry is an instant of the database's clock, and a duration lets a host beat in time
  whatever its own clock says, which is why protocol 1.8 chose durations for its fields.
- **Decision 045-015 — `byErrorCode` stays** (zoom-out review, Major 3, contested). The coordinating session's brief asks for "a retry
  policy per error type", and the case it serves is not met by `nextRetryDelay`: a worker's delay changes when the next attempt
  starts, not how many attempts a rate limit gets compared with a bug. The cost (one schema array, three refusal reasons, one
  overlay rule) is small next to the business case. Delegate (Owner decision 2026-11-24-4).
- **Decision 045-016 — The exception-to-code map stays** (zoom-out review, Nit 13, contested). Without it an uncaught exception is
  `ACTIVITY_ERROR` whatever its class, so a retry list cannot name it unless every handler catches and rethrows; the map is local to
  each SDK and changes nothing on the wire.
- **Decision 045-017 — A `RETRY_WAIT` step past its close deadline is ended by the retries pass** (zoom-out review, Nit 12, accepted).
  Revised 2026-11-24 (architecture review round 2, Nit 1): the close index covers `READY` rows only; a gated step is ended by feature
  022's timeout routine.

- **Decision 045-018 — An operator's retry clears the close deadline** (revised in round 2, architecture review Major 3: only the
  retry of a step that failed with a known code; a step whose outcome is unknown is settled by a person first, as feature 037's
  `OutcomeUnresolved` refusal says) (correctness review, Major 6; architecture review, Major 5).
  Options: (a) refuse the retry past the deadline with a typed reason; (b) give the retry a new close budget; (c) clear the close
  deadline. Chosen (c), delegate (Owner decision 2026-11-24-4): the operator has looked at the step and decided; (a) makes the
  button useless after the budget and (b) invents a budget nobody wrote. The new attempt keeps `timeoutMs` and gets a start deadline.
  The same rule clears the deadline when a reconciliation, a review or an operator's hold takes over: from then on a person or a
  reconciliation decides (correctness review, Blocker 1).
- **Decision 045-019 — A pause suspends the start deadline, not the close deadline** (revised in round 2, correctness review Major 3,
  security review Major 5, architecture review Major 9: the pause removes the claimable steps' start deadlines and the resume gives
  each a full window from `db_now`, in one statement each, with the window kept on the row; a shift by the paused time was wrong for
  steps that became claimable during the pause and needed a payload per step; the close deadline is enforced during the pause by
  every path) (correctness review, Major 7; architecture
  review, Major 4). Options: (a) both keep running; (b) both suspended; (c) the start deadline suspended, the close deadline kept.
  Chosen (c), delegate (Owner decision 2026-11-24-4): schedule-to-start detects a missing worker, and a pause is not one; the close
  deadline is the author's business window, which a pause does not extend. The pass skips a paused run; the resume moves each
  claimable step's start deadline later by the paused time; the close deadline is enforced from the resume.
- **Decision 045-020 — A reset recomputes the deadlines** (withdrawn in round 2, codebase review Major 6 and correctness review Nit 5:
  a reset point is not offered while any step is open, so no copied event makes a step claimable and no rule is needed) (architecture review, Major 5). Options: (a) copy the source's instants;
  (b) refuse a reset point with open deadlines; (c) recompute from the reset's time. Chosen (c), delegate (Owner decision
  2026-11-24-4): (a) fails the new run at once, (b) makes a common reset impossible; a reset is a deliberate new start.
- **Decision 045-021 — The close floor** (round 2: the domain journals the close cutoff, the close deadline less the floor, which is the
  indexed column, so the hint, the pass and the lease decision compare the same value; publication refuses a `scheduleToCloseMs` below
  the floor plus 1,000 ms) (correctness review, Major 3; design review, Major 2): an attempt is not leased, and a retry
  not scheduled, with less than the earlier of `timeoutMs` and 6,000 ms left before the close deadline, so that running out of budget
  is a known failure and not a review.
- **Decision 045-022 — `heartbeatFrom: "handler"` requires `scheduleToStartMs`, and has its own requirement** (round 2: the requirement
  is `activity-timeouts-protocol-v1` and also covers a `heartbeatTimeoutMs` below 30,000 ms, which only a 1.9 host is given,
  architecture review Major 4) (security review,
  Major 3; architecture review, Blocker 1 and Major 7): such a step waits for a 1.9 host, and only a bounded wait keeps it from
  waiting without end; `activity-timeouts-handler-v1` is listed only by a build that offers 1.9.
- **Decision 045-023 — The worker's delay is clamped to at least the author's initial delay** (security review, Major 4). Revised in
  round 2 (security review Major 4): the longer of the worker's delay and the policy's computed delay for that attempt.
- **Decision 045-024 — The minimum heartbeat timeout is 18,000 ms** (implementation-readiness review Major 10; correctness review round
  2, Major 9). Options: (a) 10,000 ms; (b) 18,000 ms. Chosen (b), delegate (Owner decision 2026-11-24-4): below 18,000 ms a lease is
  never provisional under protocol 1.8 (Architecture decision 0099 decision 2), so it loses the protection against a delivery lost
  on the wire, and such a loss becomes an unknown outcome and a review for work that never ran; detecting a dead worker in 18 s
  instead of 10 s is the smaller cost. Decided before the schema merges, because raising a minimum later would tighten a published
  schema (Architecture decision 0032 decision 2). This closes Gap 045-004.
- **Decision 045-025 — A withdrawal gives the attempt a fresh receive bound** (correctness review round 2, Major 6). Options: (a) keep
  the start deadline, so a lease lost on the wire past it fails the step; (b) write the later of the old deadline and the withdrawal's
  `db_now` plus the shorter of the window and 6,000 ms. Chosen (b), delegate (Owner decision 2026-11-24-4): a withdrawal is not a
  missing worker, and (a) would run a failure route, or a saga's compensations, after a delivery loss that protocol 1.8 exists to
  recover; the four withdrawals per attempt of Architecture decision 0098 decision 7 bound how often.
- **Decision 045-026 — The level of a version** (security review round 2, Majors 1 and 3; architecture review round 2, Major 8).
  Publication records 0, 1 or 2; only a version above 0 is read with these rules, so a document stored before the upgrade that already
  carries one of the keys runs as today; the removal of a requirement is refused while a version of its level exists, which is one
  indexed predicate and cannot race the publication, which holds the requirement.
- **Decision 045-027 — The publication hold** (revised in round 3, security Majors 1 to 3 and architecture Major 4: it reads the requirement's row `FOR SHARE` and checks both process kinds, as `lease_withdrawal_required()` of Architecture decision 0098's build does, inside the store's publication transaction, and the rollback floor is a rule of 0098's replay-floor trigger; the advisory lock of round 2 is withdrawn; the first text follows) was first a fair advisory lock (security review round 2, Major 2; architecture review round 2,
  Major 7): a shared transaction advisory lock on the requirement's key, which the operator's functions take exclusively; advisory
  locks queue in order, so publications cannot starve the operator, and the runtime role takes no row lock on the registry. It refines
  Architecture decision 0082 decision 1, and features 020, 029 and 031 may use the same function.

## Notes for other features (2026-11-26)

- **For feature 022 (relayed by the coordinating session, which owns 022):** in FR-022-036, after "a concurrency key with
  `ENCODING_READ_BY_CONCURRENCY_KEY`", add "and a rate-limit key with `ENCODING_READ_BY_RATE_LIMIT_KEY`" (feature 031's readiness
  review, Decision 031-009). Also from this design: Task 045-10 builds the close cutoff into 022's waiter deadline, its one timeout
  routine and its grant (Requirement 045-011), once 022's gates land.

## Gaps

- **Gap 045-005 — An attempt leased less than 18 s before its close deadline is not provisional** under protocol 1.8 (architecture review
  round 3, Major 6): its attempt deadline is the earlier deadline. Accepted for 0.3.0; a close floor of 18 s would remove it at the
  cost of a `scheduleToCloseMs` minimum of 19 s for nodes without `timeoutMs`.

- **Gap 045-001 — The deadline pass's share of a tick.** Whether it needs its own work budget per tick or shares the lost-lease pass's
  window. Closed 2026-11-24 after the security and design reviews: it never shares; it is its own `pick` pass with its own window.
- **Gap 045-002 — The start deadline as an urgency hint.** Feature 019's priority order could serve sooner a step whose start deadline
  is close; not in 0.3.0, recorded for 019.
- **Gap 045-004 — Whether the minimum heartbeat timeout should be 18,000 ms.** Closed 2026-11-24 by Decision 045-024: it is.
- **Gap 045-003 — Timeouts on the managed runner nodes** (database, lake). Their runners have their own limits; a schedule-to-start for
  a runner fleet that is down is useful and is left for a later design.

## Owner questions

Owner questions 045-001 to 045-009 are decided by Decisions 045-001 to 045-009 above. None is open.
