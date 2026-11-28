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
- **Decision 045-019 — A pause suspends the start deadline, not the close deadline** (current text: Requirement 045-021 and Technical Plan 3 of the design, the fresh full window at the resume and the close cutoff enforced during the pause; what follows is the history) (revised in round 2, correctness review Major 3,
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
- **Decision 045-022 — `heartbeatFrom: "handler"` requires `scheduleToStartMs`, and has its own requirement** (current text: every level-2 node requires `scheduleToStartMs`, and the requirement is `activity-timeouts-protocol-v1`; what follows is the history) (round 2: the requirement
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
- **Decision 045-025 — A withdrawal gives the attempt a fresh receive bound** (superseded in round 3: a full window from the withdrawal, keeping `ready_at` at levels 1 and 2; what follows is the history) (correctness review round 2, Major 6). Options: (a) keep
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
- **Decision 045-028 — Starvation of the operator is settled by a test** (2026-11-26, Owner decision 2026-11-26-564, after
  Architecture decision 0112 replaced the hold's row lock by the requirement epoch `FOR SHARE`): Task 045-21 and design 054's Task
  054-20 write first a held-lock test in which a relay of four publication sessions keeps the epoch held `FOR SHARE` for at least
  8,000 ms (4 × the operator's `lock_timeout` of 2,000 ms); the operator's activate and remove must commit within 2,000 ms, and no
  hold requested after they wait may be granted first. If they starve, the fair advisory lock of this decision's first text (or an
  operator-intent row) returns as a gate before the epoch for every reader of it, chosen by the same test, and Architecture
  decision 0112 is amended. Rejected: deciding it by reading alone; the order in which PostgreSQL serves a tuple lock's waiter and
  new compatible lockers is a behaviour of the database version in use, which only a test of that version shows.

- **Decision 045-029 — The operator starves under the epoch `FOR SHARE`, so the hold has a fairness gate** (2026-11-27, Task 045-21,
  Owner decision 2026-11-26-564). Evidence, on PostgreSQL 16.15 (the `postgres:16-alpine` the platform and its tests run): with four
  sessions relaying `FOR SHARE` on one row, the next granted before the previous commits, 48 share locks were granted after a `FOR
  UPDATE` with `lock_timeout = 2s` was seen waiting, and that `FOR UPDATE` was still waiting after 10 s (a new share locker of a row
  that is only share-locked never sleeps, and `lock_timeout` restarts on every wait of the waiter). With a transaction advisory lock
  taken shared by the holds and exclusively by the operator, one share request was granted after the operator waited, and it read the
  epoch the operator had already moved: the operator committed first. The same relay against the functions of migration
  `20270218000000_requirement_hold` (the hold as the runtime role; `platform_requirement_activate`, and `platform_requirement_lock`
  then `platform_requirement_remove` as `PgInstanceStore::remove` runs them) committed the operator's command at once and every hold
  requested after it waited read the new epoch. Options: (a) an operator-intent row that a new hold reads; (b) a transaction-scoped
  advisory lock, shared for readers, exclusive for the operator; (c) a lock on an empty table, `SHARE MODE` for readers,
  `EXCLUSIVE MODE` for the operator. First chosen (b), then (c) after the security review of the same day (its Nit 1: any role can
  take an advisory lock, outside a transaction too, and other features hash tenant-chosen text into the same key space); a probe
  showed (c) as fair as (b). Chosen (c), delegate (Owner decision 2026-11-24-4): the lock manager's queue is fair by construction
  for both, `LOCK TABLE` checks privileges so only the owner's functions can take the gate, and it costs one statement per function, while (a) needs a row the operator commits in a
  transaction of its own before its command and clears after it, and a crash between the two leaves every publication refused until
  someone clears it. The gate is first in the registry's lock order; ADR-0112 is amended in place (its amendment of 2026-11-27, with
  the hold's new body). The repository's evidence is `integration_platform_foundation::requirement_hold::an_activation_commits_under_a_continuous_stream_of_holds`
  and `...::a_removal_commits_under_a_continuous_stream_of_holds`, run first against the hold of design 054's text, without the gate. Seen failing on that text (CI run 45936551219 of pull request #295, commit `0a16d2fe7`, 2026-11-27): the activation's relay
  granted 413 of 1,125 holds ahead of the waiting command and the removal's 376 of 1,113, and each command ended after 3.0 s on its
  `statement_timeout` (`canceling statement due to statement timeout`), never on its 2,000 ms `lock_timeout`, which restarts on every
  wait.
- **Decision 045-030 — Task 054-18 is built here, as a pull request of its own** (2026-11-27). No design had landed the prerequisite
  of ADR-0112 (no branch of designs 044 or 054 creates `platform_requirement_hold` or `platform_requirement_floors`), and Task 045-21
  orders it before Task 045-6. Delegate (Owner decision 2026-11-24-4), under ADR-0112 decision 4 ("landed by whichever of the designs
  builds first") and Owner decision 2026-11-26-540, which covers Task 054-18. Designs 044 and 054 depend on it and their migrations
  only compare the hold with the amended text.
- **Decision 045-031 — The floor's isolation check is for a capability that has a floor** (2026-11-27, ADR-0112 amendment decision
  4). Options: (a) every removal refused outside `READ COMMITTED`, as decision 2's text reads; (b) only a removal that is about to
  read a floor. Chosen (b), delegate (Owner decision 2026-11-24-4): the check exists so that a floor read after waiting for a
  publication sees the committed version; a capability with no floor reads nothing, and (a) would change how every existing
  requirement is removed for no guarantee.
- **Decision 045-032 — The floors have a fourth closed list** (2026-11-27, ADR-0112 amendment decision 5). Options: (a) a fourth check,
  `platform_requirement_floors_capability_known`, kept equal to the other three by the existing capability-list test and rule; (b) no
  check, since only a migration writes a floor. Chosen (a), delegate (Owner decision 2026-11-24-4): ADR-0112 decision 2 bounds the
  column by the closed list, and the existing test makes the cost (one more list to restate) a failure at review time instead of a
  silent drift. The branch of feature 022 restates three lists today; whichever of it and this one merges second restates four.
- **Decision 045-033 — Architecture decision 0098's floor stays in its trigger for now** (2026-11-27). ADR-0112 says it becomes a
  row "when both are built"; moving it changes the messages and the isolation rule its own tests pin
  (`integration_platform_foundation::migrations`, decisions D006-084 and D006-085 of feature 006), which is a change to feature 006's
  contract that no task of this design or of design 054 names. Delegate (Owner decision 2026-11-24-4): left to a task of its own;
  until then two triggers guard `platform_requirements` removals, each for its own capabilities, and neither replaces the other.

- **Decision 045-034 — The correctness review of the prerequisite** (2026-11-27, `correctness-reviewer` on the slice of Task 045-21).
  Blocker 1 (the automatic handover took the epoch and then asked for the gate through the activation, a deadlock with a
  registration or a hold): fixed, the handover takes the gate first (migration `20270218000000`, the gate-order test lists it, and
  `integration_schedules_bulk_operations::trigger_auto_handover::an_engine_registering_without_the_capability_during_the_check_both_orders`
  runs the held order). Major 1 (the relay tests passed if the command never waited; the 8,000 ms assertion could not fail): fixed,
  the relay asserts the command was seen waiting and that holds were requested while it waited; the 8,000 ms stays the loop's exit.
  Major 2 (no test of `REQUIREMENT_BUSY`; the bound is per lock, not per call): fixed, a test of the refusal within its bound and the
  helper's documentation. Nits: the helper takes the publication's transaction (fixed); the floor check admitted an oid with no
  function (fixed, `IF NOT FOUND`); the down migration's bodies are compared by a probe that hashed the five functions' source,
  configuration and grants before the migration and after its rollback (equal), delegate (Owner decision 2026-11-24-4), no test of its
  own, as for the other registry migrations; `CAPABILITY_NOT_ACTIVE` is the publication's refusal, built with Task 045-5 and Task
  054-5, delegate; the relay rotates four connections of which at most two hold at once, which is the relay Task 045-21 describes
  (session k commits once session k + 1 is granted or waits), delegate; the 2,000 ms bound includes the harness's polling, kept,
  because it is the criterion Owner decision 2026-11-26-564 states; the ordering test of a hold after a removal calls the function with
  no bound (fixed). The lock-order harness (`tests/support/lock_order.rs`) has no slot for the gate: Gap 045-008.

- **Decision 045-035 — The security review of the prerequisite** (2026-11-27, `security-reviewer`, no Blocker and no Major; every
  finding fixed, as security findings always are). Nit 1 (the advisory gate's key space is open to every role and to hashed
  tenant text): the gate is a lock on the empty table `platform_requirement_gate`, which only its owner can lock (Decision 045-029).
  Nit 2 (a floor's function checked only when registered): the floor trigger checks it again before each call and refuses the
  removal when it is no longer admissible. Nit 3 (set-returning functions and floors that do not run as their owner admitted): a
  floor must be a plain function, `SECURITY DEFINER` with the fixed search path, with no overload. Nit 4 (a hand edit could delete a
  floor): a row is deleted only under the down migration's flag `sagat.floor_change`, and the table cannot be truncated. Nit 5 (an
  answer a caller could ignore, and a savepoint that releases the hold): the helper returns the `#[must_use]` `Held`, and its
  documentation forbids a savepoint around it; a fitness rule that every publication path calls it is Task 045-5's (Gap 045-008).
  Nit 6 (an overload created in `public` by a role that may create there makes the floor's call ambiguous): the admissibility check
  refuses a floor with an overload, so such a removal fails closed and names the cause. Tests:
  `requirement_hold::a_floor_names_only_an_admissible_function` and
  `...::a_floor_is_removed_only_by_a_migration_and_checked_again_before_each_call`.

## Notes for other features (2026-11-26)

- **For feature 022 (relayed by the coordinating session, which owns 022):** in FR-022-036, after "a concurrency key with
  `ENCODING_READ_BY_CONCURRENCY_KEY`", add "and a rate-limit key with `ENCODING_READ_BY_RATE_LIMIT_KEY`" (feature 031's readiness
  review, Decision 031-009). Also from this design: Task 045-10 builds the close cutoff into 022's waiter deadline, its one timeout
  routine and its grant (Requirement 045-011), once 022's gates land.

## Gaps handed over by design 044 (2026-11-26)

- **Gap 044-020** (design 044's register): answered; this design accepts a code version's `timeoutLevel` as its level, owns
  `steps.min_protocol_minor`, and excludes design 044's workflow-task step from its deadlines (Technical Plan 15).
- **Gap 044-026** (design 044's register): answered; the shared hold function has no capability list in its body, and a migration that
  finds it compares and fails on a difference, and none drops it (Technical Plan 2).

## Gaps

- **Gap 045-005 — An attempt leased less than 18 s before its close deadline is not provisional** under protocol 1.8 (architecture review
  round 3, Major 6): its attempt deadline is the earlier deadline. Accepted for 0.3.0; a close floor of 18 s would remove it at the
  cost of a `scheduleToCloseMs` minimum of 19 s for nodes without `timeoutMs`.

- **Gap 045-001 — The deadline pass's share of a tick.** Whether it needs its own work budget per tick or shares the lost-lease pass's
  window. Closed 2026-11-24 after the security and design reviews: it never shares; it is its own `pick` pass with its own window.
- **Gap 045-002 — The start deadline as an urgency hint.** Feature 019's priority order could serve sooner a step whose start deadline
  is close; not in 0.3.0, recorded for 019.
- **Gap 045-004 — Whether the minimum heartbeat timeout should be 18,000 ms.** Closed 2026-11-24 by Decision 045-024: it is.
- **Gap 045-006 — The beat stays outside the gate** (2026-11-27, ADR-0112 amendment; revised the same day after the correctness
  review, Blocker 1: the automatic handover is no longer outside it, see Decision 045-034). The beat of a process
  (`instance_heartbeat_reporting_change`) takes the epoch `FOR SHARE SKIP LOCKED` without the gate, because it must never wait; beats
  can therefore still overlap an operator's wait for the epoch, bounded by the beat rate (one beat per process every few seconds,
  at most 1,000 registered processes). A follow-up can give the beat `LOCK TABLE platform_requirement_gate IN SHARE MODE NOWAIT`, which never
  waits and fails while the operator waits or holds.
- **Gap 045-007 — The design texts of Task 045-21 are not yet revised** (2026-11-27). Task 045-21 also asks for a re-approved revision
  of Technical Plan 13 and Tasks 045-5 and 045-6 (and design 054's Technical Plan 15 and Task 054-18 follow, its Task 054-20):
  the hold's body and the gate of ADR-0112's amendment, the floor as a row of `platform_requirement_floors` with a function on
  `workflow_versions.activity_timeouts_level`, and the one refusal `CAPABILITY_NOT_ACTIVE`. A change to an approved design makes it a
  draft until its re-review, so it is a documents pull request of its own, before Task 045-5; the code of the prerequisite follows
  ADR-0112 as amended, which both designs cite.
- **Gap 045-008 — A publication's hold against the operator's step** (2026-11-27, correctness review, Nit 3). The hold is a
  publication's last statement, after it wrote its rows (`workflow_triggers` among them); an operator's step that locks such a table
  under the gate and the epoch (the handover's `LOCK TABLE workflow_triggers`) waits for the publication, which waits for the gate: a
  cycle that the hold's 1 s bound or the deadlock detector cuts, refusing one side. Task 045-5 decides it before any publication calls
  the hold (the hold before the version's writes, or the step outside the publication's tables), and the lock-order harness gains a
  slot for the gate.
- **Gap 045-003 — Timeouts on the managed runner nodes** (database, lake). Their runners have their own limits; a schedule-to-start for
  a runner fleet that is down is useful and is left for a later design.

## Owner questions

Owner questions 045-001 to 045-009 are decided by Decisions 045-001 to 045-009 above. None is open.
