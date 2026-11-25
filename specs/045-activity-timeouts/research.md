# Research register: design 045, activity timeouts, retry per failure code and buffered signals

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
- **Decision 045-006 — `activity` and `script` nodes only** (Owner question 045-006). Options: (a) every leased node kind,
  including the managed database and lake runners; (b) `activity` and `script`; (c) `activity` only. Chosen (b), delegate (Owner
  decision 2026-11-24-4): these are the nodes whose workers the author operates and whose stalls the author must detect; the managed
  runners are platform processes with their own limits (`timeoutMs` of their connection, `LakeLimits`), and adding them is Gap
  045-003. Compensation and reconciliation are excluded by principle III (a deadline must not end a pending compensation).
- **Decision 045-007 — Pending signals move across continue-as-new** (Owner question 045-007). Options: (a) discard them; (b)
  refuse the continuation while any is pending; (c) move those the successor's version declares buffered and discard the rest,
  journaled. Chosen (c), delegate (Owner decision 2026-11-24-4): (a) loses business messages silently, (b) lets a sender block a
  run's progress; (c) keeps every message a successor can consume and records the others, without user code draining a channel.
- **Decision 045-008 — Buffering is opt-in per version and per name** (Owner question 045-008). Options: (a) buffer every signal of
  every run (Temporal); (b) opt-in. Chosen (b), delegate (Owner decision 2026-11-24-4): (a) changes the answer of `signalRun` for
  runs in flight and for callers who rely on the `409` today (constitution principle IV); (b) gives the same capability to every new
  version that asks for it.
- **Decision 045-009 — Buffered signals are designed here** (Owner question 045-009). Options: (a) in design 044 (workflow as code);
  (b) a design of their own; (c) here. Chosen (c), delegate (Owner decision 2026-11-24-4): they are not covered by feature 018
  (explicitly out of scope); (a) would make the declarative mode wait for 044's determinism study; (b) is clean but takes another
  number while the 0.3.0 designs are being numbered; (c) shares this design's race with a wait's timeout, its fleet switch and its
  conformance scenarios, and its tasks form a slice that can be moved out unchanged.
- **Decision 045-010 — The start deadline does not move on a lease withdrawal.** A withdrawn lease (Architecture decision 0098,
  Architecture decision 0099) never started, so the attempt is the same attempt; resetting the deadline would let withdrawals
  postpone a timeout without limit.
- **Decision 045-011 — The minimum heartbeat timeout is 3,000 ms.** A host beats at a third of it (1,000 ms), three beats per window,
  above the 751 to 889 ms ready-to-handler 99th percentile measured in `tests/performance/reports/sdk-streaming.md`. Below 18,000 ms a
  lease is never provisional (Architecture decision 0099 decision 2), which is correct.
- **Decision 045-012 — Field numbers.** `TaskLease` 26 to 28 and `FailStepRequest` 14 for protocol 1.9, above every number the drafts
  and protocol 1.8 name; the register is in Technical Plan 8 of the design and is recorded in the research registers of drafts 019 and 031 on
  2026-11-24: 031 moves to `TaskLease` 25 and `WorkerCapabilities` 7, 019's `RegisterWorkerResponse` field moves from 4
  (taken by design 041) to 5.

## Gaps

- **Gap 045-001 — The deadline pass's share of a tick.** Whether it needs its own work budget per tick or shares the lost-lease pass's
  window. Closed by the measurement of Task 045-19.
- **Gap 045-002 — The start deadline as an urgency hint.** Feature 019's priority order could serve sooner a step whose start deadline
  is close; not in 0.3.0, recorded for 019.
- **Gap 045-003 — Timeouts on the managed runner nodes** (database, lake). Their runners have their own limits; a schedule-to-start for
  a runner fleet that is down is useful and is left for a later design.

## Owner questions

Owner questions 045-001 to 045-009 are decided by Decisions 045-001 to 045-009 above. None is open.
