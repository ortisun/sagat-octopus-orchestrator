# Architecture review, round 2: delivery acknowledgement (ADR-0099, worker protocol 1.8)

## Verdict

Not ready for acceptance. There is one Blocker.

The round-1 Blockers in my report are resolved:
- The acknowledgement now takes only the locks and restores nothing.
- The provisional mark is bound to the lease's generation.
- The loop of repeated withdrawals is bounded.

The cost claims are now honest. Each is labelled an estimate and given an acceptance bound. The layering is sound:
- In the domain: one new cause and one failure code, with no input or output.
- In the application: one pure decision function.
- In the adapter: one SQL macro.
- No new mapper.

The new problems:
- The way minor 1.8 is gated couples every later minor to an operator switch and to a minimum lease time-to-live.
- The acknowledgement still accepts a lease after its attempt deadline.
- The hand-back that a host keeping the protocol must send is also counted as a protocol violation.
- The changed text of feature 037 misses three cases.
- The window after the requirement is removed conflicts with ADR-0098 decision 4.
- The go/no-go measurement comes after the most expensive work.
- The conformance evidence at 1.8 is weaker than stated.

**Counts:** 1 Blocker, 6 Majors, 13 Minors, 9 Nits. I could not run `git branch -a` or `git worktree list` (no shell). I searched the sibling worktrees `sagat-wt-withdraw-leases` and `sagat-wt-withdraw-sdks` and found no copy of this work. The decision numbers do not clash: the ADR-0098 builds stop at D006-027 and G006-022.

---

## Status of the round-1 findings

### My report

| Round 1 | Status | Reason |
|---|---|---|
| Blocker 1: the acknowledgement had to load the run's state | **Resolved** | ADR decision 3 and D006-043: a pre-read without locks, then the chain lock and the run-row lock, the order `begin` (`crates/infra-postgres/src/execution.rs:1563-1577`) and `begin_path` (`execution.rs:882-920`) use, and one conditional write with `RETURNING`. The binding is checked on the activity that write returns. The budget assertion forbids a scan of the run's steps. The shape of the new store port is not fully named (Minor 1). |
| Blocker 2: the mark was not tied to its lease | **Resolved** | `lease_offered_generation` must equal `generation`. Every lease increments the generation (`crates/domain/src/execution/command.rs:399,438,481`), so a lease written by any binary invalidates a stale mark. There is a test for "a lease written with the old column list". |
| Blocker 3: an endless loop of withdrawal and new lease | **Resolved, with a residual** | The host falls back to 1.7 on any status that is not retryable (decision 10, as ADR-0029 decision 6 does). After three `NOT_ACKNOWLEDGED` withdrawals the next lease carries no mark. Residual: hand-backs after the host gives up under failures that are retryable are not bounded (Minor 6). |
| Major 1: scoped instances | **Resolved** | The section "Scoped instances", the scoped withdrawal of the ADR-0098 build, `live_path_fence`, and a scoped row in every task. |
| Major 2: the derivation of A and the threshold | **Partly resolved** | The derivation now uses specified bounds (2R = 12 s). The call bound (3 s) is a requirement that has to be measured, and the delivery delay and the call duration are measured separately. Not answered: how long one call may hold a ticket of the `Lease` admission class and a database connection (Minor 8). |
| Major 3: the time-to-live gate only at negotiation | **Resolved** | The capability follows the engine's configuration (ADR-0082 decision 7), and the claim also checks its own time-to-live. |
| Major 4: the meaning of `timeoutMs` | **Resolved as to its meaning** | It is counted from the claim at every minor, with the new code `ATTEMPT_TIMEOUT_BEFORE_START`. Two new defects come with it: Major 1 and Major 3. |
| Major 5: golden stability as evidence | **Partly resolved** | The mechanism exists (a harness override). The 26 scenarios that pin no minor run no worker host, and the override skips the scenarios that exercise the new host rules (Major 6). |
| Major 6: no acceptance bound for throughput | **Resolved** | A bound of 15% against 1.7, and 0.3.0 ships the requirement unrecorded. Where the gate sits in the plan is a new Major (Major 5); its statistic is a Minor (Minor 9). |
| Major 7: no fitness functions | **Resolved** | The generation binding with its test, a contract test driven by the descriptor, the budget lines, and rows in `architecture-principles.md` (T006-056). Gaps in the mechanisms: Minor 2 and Minor 13. |
| Major 8: gaps in the order of the build plan | **Resolved** | Preconditions (a) to (c), the `AppendStepLogs` case after T041-7, each SDK task after its T041 task, `bugfix/withdraw-leases-sdks` named, ADR-0098 accepted first, and T006-063 with a single owner. |
| Major 9: reviews that had not happened | **Resolved** | "Round 2 is still to run." The directory date is a Nit (Nit 1). |
| Major 10: cancellation depended on the ADR-0098 build | **Resolved** | The acknowledgement decides the cancellation itself (D006-051), and D006-059 (1) is stated as a requirement on the ADR-0098 build. |

### Findings of the other reports in my areas

| Finding | Status |
|---|---|
| Correctness Major 2 (the acknowledgement accepted an expired lease) | Resolved for the expiry. **Not resolved for the attempt deadline** (Major 1). |
| Correctness Major 5, design Major 4 (rollback) | Resolved: a floor for the rollback, sessions expired, the `down` migration refuses. New gap in the window after removal (Major 4). |
| Correctness Major 6, security Major 3 (a stale mark) | Resolved. |
| Correctness Major 8, design Major 7 (conformance) | Partly resolved (Major 6). |
| Security Major 4 (cost per call, locks, audits) | Resolved: the pre-read without locks, one audit record per call through the bounded gate. |
| Security Minor 3 (the dependency enforced in the database) | Resolved. The text misdescribes the trigger function (Minor 3). |
| Design Major 1 (option 5) | Resolved: the rejection now gives the correct reason. |
| Design Major 2 (honesty of the cost, an acceptance bound) | Resolved. |
| Design Major 4 (a), a cause set that later causes can extend | Not weighed (Minor 12). |
| Design Major 5 (the `Lease` reserve, runs handled one after another) | Partly resolved (Minor 8, Major 5). |

---

## Blocker

### Blocker 1. The 1.8 host rule is tied to the minor, and offering the minor depends on fleet state, so every later minor inherits the operator switch and the 12 s floor

- **Location.**
  - ADR-0099 decisions 1 and 11 (lines 43 and 104).
  - `stream-completion.md`, "Negotiation and activation", line 300: `protocol_ceiling` "answers 8 … only when it would answer 7, the store proves … and the engine knows the requirement … is recorded".
  - `sdks/SPEC.md` §6.1, the new 1.8 bullet, and the 1.8 row of the §9 register.
  - D006-041 and D006-047.
- **What is wrong.**
  - Minors are cumulative (ADR-0015, design 041). A server that offers 1.9 offers a session that also speaks 1.8. As written, 1.9 can be offered only "when it would answer 8", which means:
    - the operator has recorded `delivery-acknowledgement-v1`;
    - every engine runs a time-to-live of at least 12 s.
  - So the minors that drafts 020, 021, 025 and 026 will take become unavailable:
    - on every default 0.3.0 deployment (D006-057: shipped unrecorded);
    - on any fleet that runs a lease time-to-live under 12 s;
    - on any fleet that has rolled delivery acknowledgement back.
  - The only way around it is to offer 1.9 while the requirement is unrecorded. Then the host rule of 1.8 applies, and every lease pays the acknowledgement's round trip for nothing (the verdict `renewed`).
  - The ADR states neither consequence and weighs no alternative. Until now a minor's ceiling depended only on proofs that are constants of the build (`crates/application/src/worker.rs:82-90`). This makes it depend on operator state that changes at run time.
- **Why it matters.** This is the hard-to-reverse part of the decision. Once six SDKs implement "a session at 1.8 acknowledges every lease", the register is fixed for every later minor. It also forces the extra machinery of the removal path: the second transaction that expires sessions, the fallback after `UNIMPLEMENTED`, and the rollback floor for sessions.
- **Smallest correction.** Weigh, and I recommend adopting, a flag on each lease instead of a host rule bound to the minor:
  - Add a field to `TaskLease`, for example `bool acknowledge_before_start = 19;`, set only on a lease the claim marked.
  - A host at 1.8 or later must not start or call anything for a flagged lease before `ACKNOWLEDGED`. It dispatches an unflagged lease at once, as it does today.
  - The server offers 1.8 on the proof of the build alone. The requirement and the time-to-live then gate only the marking, where the claim already checks them.

  Gains:
  - nothing is paid on a fleet that has not activated;
  - minor 1.9 and later are not gated;
  - after a removal, engines stop flagging within one heartbeat, so no sessions need expiring;
  - a binary rolled back never flags, so a 1.8 host never calls `AcknowledgeTasks` on it;
  - the trust assumption is unchanged.

  Cost: one additive field in the proto copies, which the six SDKs regenerate anyway for the new call.

  If the gated minor is kept instead:
  - say in the ADR's consequences and in §9 that every later minor inherits the activation and the 12 s floor;
  - add that note to the research of drafts 020, 021, 025 and 026;
  - put it to the owner.

---

## Majors

### Major 1. The acknowledgement accepts a provisional lease after its attempt deadline, so a lease that timed out before it started becomes a review, and the handler runs past its timeout

- **Location.**
  - `stream-completion.md`, "The server's acknowledgement", step 3 (line 356): the write is conditional on "when the lease is provisional, `lease_expires_at` still in the future" and nothing else.
  - The failure-table row at line 494 ("a later acknowledgement is `NOT_HELD`").
  - ADR-0099 decisions 4 and 7.
- **What is wrong.** At the claim, a new token sets `lease_expires_at = now + time-to-live` without capping it at the attempt deadline. The cap applies only to renewals (`execution.rs:7047-7051`, `execution/path.rs:1229-1232`). With `timeoutMs` = 5 s and a 30 s time-to-live:
  1. The acknowledgement arrives at the claim plus 6 s. The attempt deadline has passed; the expiry has not.
  2. The write succeeds and renews the lease to `least(now + 30 s, attempt_deadline_at)`, a time already in the past.
  3. The host is told `ACKNOWLEDGED` and starts the handler.
  4. The pass then reads "acknowledged and timed out" and calls `expire_attempt` with `ATTEMPT_TIMEOUT`, an unknown outcome (`crates/domain/src/execution/event.rs:110-119`), which sends an `Activity` to review.

  `ATTEMPT_TIMEOUT_BEFORE_START` (decision 7) is therefore decided only when the pass happens to run before the acknowledgement. The row "a later acknowledgement is `NOT_HELD`" holds only after the pass.
- **Smallest correction.**
  - For a provisional lease, the conditional write also requires `attempt_deadline_at IS NULL OR attempt_deadline_at > now()`. Otherwise it answers `NOT_HELD` and writes nothing, exactly as for the expiry. The attempt deadline is then literally the attempt deadline too.
  - Add the interleaving to the barrier tests of T006-052: attempt deadline passed, expiry in the future, acknowledgement before the pass.
  - Add the combination to the unit test of the pure function.

### Major 2. A host that keeps the protocol must send a hand-back that is also counted as a protocol violation

- **Location.**
  - ADR-0099 decision 1 (line 43: "MUST NOT make any other call for it, before … ACKNOWLEDGED") and decision 13 (line 115).
  - D006-053 and T006-054.
  - `stream-completion.md`, line 499 and "The host at 1.8", items 1, 4 and 5.
  - `sdks/SPEC.md` §6.2 and §6.8.
- **What is wrong.** The procedure requires `FailStep(TASK_NOT_STARTED)` on leases that are still provisional:
  - credit over the limit, and leases received while stopping;
  - a lease the host gave up;
  - the fallback after `UNIMPLEMENTED`;
  - `stop()` handing back leases still waiting for their acknowledgement.

  Decision 1 forbids "any other call" before the verdict. Decision 13 counts any `FailStep` on a provisional lease as a violation and clears `acknowledgement_kept`. As built from this text:
  - a host that gave up one lease loses every later mark for the rest of its session;
  - `worker_protocol_violations_total` counts every rolling deploy.

  The trust telemetry would then mean nothing.
- **Smallest correction.**
  - Decision 1: "… MUST NOT start a handler or make any call for it other than the hand-back `FailStep(TASK_NOT_STARTED, retryable)`."
  - Decisions 13 and D006-053, and T006-054: exempt that report, and test that it neither counts nor clears `acknowledgement_kept`.

### Major 3. The 037 clarification covers one of the four cases the design creates

- **Location.** `specs/037-compensation-safety/spec.md`, the FR-037-006 clarifications (lines 151-160).
- **What is wrong.** The new clarification covers only the withdrawal at expiry (`NOT_ACKNOWLEDGED`). It misses three cases:
  - (a) `ATTEMPT_TIMEOUT_BEFORE_START`. This is an attempt, consumed, whose outcome is recorded as known on the same trust assumption. FR-037-006 still reads it as an attempt that ended without a report, which is an unknown outcome.
  - (b) The withdrawal inside the acknowledgement on cancellation (decision 8, cause `NOT_STARTED`). The server withdraws, not the host, so neither clarification covers it.
  - (c) The earlier clarification (line 152-153) still says a lease "handed back with `TASK_NOT_STARTED` while the step was still `LEASED`, is not an attempt". At 1.8, an acknowledged lease whose step is still `LEASED` is an attempt (D006-045). The two clarifications now contradict each other.
- **Why it matters.** FR-037-006 applies constitution principle III. T006-067 confirms the clarification against the build, and the owner accepts the trust assumption on this text.
- **Smallest correction.**
  - Extend the 2026-11-20 ADR-0099 clarification to (a) and (b).
  - Amend the first clarification with "and, at 1.8, not acknowledged".
  - List all three under decision 13 for the owner's acceptance.

### Major 4. After the requirement is removed, marks keep their meaning, but ADR-0098 forbids the event once `lease-withdrawal-v1` is removed

- **Location.**
  - ADR-0099 decision 5 ("whatever the requirement says by then") and decision 11.
  - `stream-completion.md`, "Rollback".
  - ADR-0098 decision 4 ("The event MUST be written only while the requirement `lease-withdrawal-v1` is recorded").
  - The ADR-0098 build's runbook, `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/runbooks/lease-withdrawal.md` "Rollback": removal by hand SQL, then the binary is rolled back below ADR-0098.
- **What is wrong.** The trigger refuses to remove `lease-withdrawal-v1` only while `delivery-acknowledgement-v1` is recorded. Take this sequence:
  1. The operator runs `delivery-acknowledgement-remove`, then right away removes `lease-withdrawal-v1` by the ADR-0098 runbook.
  2. Provisional leases still in flight, for up to one time-to-live plus the period of the pass, are then either:
     - withdrawn with `step.lease_withdrawn`, which breaks ADR-0098 decision 4; this is the event whose absence makes a rollback below ADR-0098 safe;
     - or suppressed, so they lapse into review, which breaks "a mark keeps its meaning".

  ADR-0098's `down` migration refuses on such a journal, but rolling back the binary alone does not run it.
- **Smallest correction.**
  - Decision 5: a mark keeps its meaning while `lease-withdrawal-v1` is recorded. Without it, an expired provisional lease ends as a known retryable failure whose code an older binary replays (a `step.failed`). Such failure codes are open strings, and the domain's unknown-outcome list does not hold this one.
  - The rollback order and both runbooks: remove delivery acknowledgement, wait until `worker_leases_provisional` is 0, then remove `lease-withdrawal-v1`.
  - T006-046 updates `docs/runbooks/lease-withdrawal.md`, not only `instance-registry.md`.

### Major 5. The go/no-go gate (15% and 3 s) comes after the six SDKs and after the path that switches the feature on has merged; the gate has no fallback and the code has no dormant state

- **Location.** `tasks.md`, T006-064 (line 261) after T006-058 to T006-063. T006-046 (line 170) lists the capability in server work that merges first. D006-046 and D006-057.
- **What is wrong.**
  - The design's own arithmetic says the call bound may fail: 64 × 0.103 s = 6.592 s against 3 s.
  - The throughput gate is measured with the in-process workers of `tests/performance/scalability-ha`, which need no SDK. Yet both gates run last, after six SDKs have been built.
  - By then `develop` holds engines that list `delivery-acknowledgement-v1`, so an operator could activate a build that failed its acceptance. "Stops and reports to the owner" names neither the state the code is left in nor the next design step.
- **Why it matters.** The most expensive work would be done before the one measurement that can cancel it. There is a precedent for keeping such a feature dormant: "Incomplete capacity coordination stays dormant", enforced by `scripts/check-capacity-activation.py`.
- **Smallest correction.**
  - Move the server half of T006-064 (throughput against 1.7, the 64-run call, the delays) to right after T006-049 and T006-055, before T006-057.
  - Keep the capability unlisted until a last server task, after the gate passes, turns it on.
  - Name the fallback if the call bound fails: a bounded number of runs per call, with the host splitting frames, or a stated small concurrency.

### Major 6. The conformance evidence at 1.8 comes from scenarios that do not exercise the new host rules

- **Location.** D006-058 (`research.md:415`); `tasks.md` T006-057 and T006-065; the acceptance-matrix row "Six SDKs".
- **What is wrong.**
  - (a) None of the 26 scenarios that pin no minor declares a worker host: the 52 files with `workers` are exactly the 52 that pin a minor (`tests/contract/sdk-conformance/scenarios/`). They give no evidence about 1.8.
  - (b) The override raises only 1.5. The scenarios that exercise what 1.8 changes are pinned at 1.4 and never run at 1.8:
    - `secrets` and `blobs` (capability calls after the acknowledgement);
    - `cancellation`;
    - `worker-shutdown` (`stop()` hands back what waits for its acknowledgement);
    - `lost-responses`, `lease-lost`, `failures-and-retries`, `run-lifecycle`.
  - (c) `protocol-negotiation` is one of the 29 files pinned at 1.5, yet it runs with a 3 s time-to-live and gets a new golden. So "29 … with their existing goldens" means 28.
- **Smallest correction.**
  - Say that the 26 are no evidence for 1.8.
  - Add to `delivery-acknowledgement` the cases the 1.4 scenarios would have shown: `ResolveSecrets` and `ReadBlob` after `ACKNOWLEDGED`, refused before it (from the SDK side), and a failure retried at 1.8. Or extend the override to the 1.4 host scenarios, normalizing the `instance` field out of the golden.
  - Correct the count to 28.

---

## Minors

1. **The shape of the new store port, and decision 8's "in the same transaction".**
   - Location: ADR-0099 decisions 3 and 8; `stream-completion.md` step 4 (line 365); T006-049.
   - Problem: the lock-only open is a new port, and decision 8 restores the run or the path inside it. The existing entries cannot do that. `begin_in_transaction` is `pub(crate)` (`execution.rs:267`). `begin_current_path` and `load_path` open their own transactions (`execution.rs:817-880`).
   - Fix: name the port, for example `lock_run(tenant, run) -> LockedRun`, with promotions `restore()` and `restore_path(path)` in the same transaction. Or say that on a cancellation the run is rolled back and reopened through `begin` or `begin_current_path` and decided again, which is equally exact.

2. **"Provisional" is written twice, with no check that the two agree.**
   - Location: `stream-completion.md` "The provisional mark" (the SQL macro) and "The withdrawal" (the pure function).
   - Problem: the acknowledgement's conditional write and the capability fence decide in SQL; the pass decides in Rust. A disagreement cannot double an outcome, because `lease_acknowledged_at` is where the two writers are ordered. It can refuse capabilities wrongly or send a lease to review.
   - Fix:
     - Add a parity test that writes every combination of the row and compares the macro with the function, as `crates/domain/tests/outcome_unknown_parity.rs` does for the unknown-outcome codes, and add its row.
     - Put the function beside `LeaseDeadlines` (`crates/application/src/execution.rs:993`) and extend that type rather than add a parallel one.
     - Define the bound of three once.

3. **The registry trigger.**
   - Location: `stream-completion.md` "Negotiation and activation"; T006-046.
   - Problems:
     - `platform_requirements_guard` is not `SECURITY DEFINER` and does not pin its `search_path` (`crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:98-114`). Making it a definer function adds privilege for nothing.
     - Writing capability names into the registry's generic guard couples the registry to features.
     - An exception raised by the trigger reaches the command as raw SQL, not as one of the typed outcomes of `platform_requirement_activate`.
     - ADR-0082 has an Amendments section (as ADR-0083 used), and "refines ADR-0082" adds nothing there.
   - Fix:
     - Schema-qualify the guard, as `platform_requirements_epoch_moved` is.
     - Consider a small dependency table, seeded by the migration and read by the guard.
     - Have the command check under the epoch lock for a typed refusal, with the trigger as the backstop.
     - Add an amendment line to ADR-0082 at acceptance.

4. **The migration is incompletely specified.**
   - Location: T006-044 (`tasks.md:159`); the ADR's consequences ("one migration").
   - Missing: the three closed capability checks (`*_known` on `serving_instances`, `platform_requirements`, `platform_requirement_history`), added `NOT VALID` and then validated, as `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql` does; the replacement of the trigger function and its restoration in `down`.
   - The `down` should also refuse, as that build's `down` does:
     - while the history names the capability;
     - while a live process lists it;
     - while a journal holds `NOT_ACKNOWLEDGED`, if the rollback floor is this build.

5. **Operating the removal command.**
   - Location: decision 11; `stream-completion.md:314`; T006-046.
   - Problems:
     - Interrupted between its two transactions, the command leaves the requirement removed and the sessions unexpired.
     - The expiry changes every tenant's sessions and writes no audit record.
     - "At most one lease time-to-live later" assumes the learning bound always holds. Under ADR-0082 decision 5, a process whose registry connection fails keeps serving and may keep accepting 1.8.
   - Fix:
     - A second run of the removal, when the requirement is already absent, still waits and expires the sessions.
     - Write a bounded audit or history record with the count.
     - The rollback checks a count of sessions at 1.8 and `worker_leases_provisional` = 0, not a duration.
   - Most of this goes away under Blocker 1's flag on each lease.

6. **Part of the loop is unbounded.**
   - Location: decision 10; D006-052.
   - Problem: a host whose `AcknowledgeTasks` fails with retryable errors for good (a proxy answering `UNAVAILABLE`, constant `RESOURCE_EXHAUSTED`) gives up and hands back. That is a `NOT_STARTED` withdrawal, which the counter ignores. So: claim, give up, hand back, claim again, at the pace of the time-to-live and with no bound, alerted only after 30 minutes.
   - Fix: count a hand-back of a lease that was never acknowledged in `lease_unacknowledged_withdrawals`, or name this as a gap with the alert as its control.

7. **The alert `SagatLeaseWithdrawalBound` (line 527) has no metric to read.** No listed counter or gauge records a step reaching the bound. Add one, for example `worker_lease_marks_skipped_total{reason="withdrawal_bound"}` or a gauge from the operations-statistics query.

8. **The `Lease` admission reserve and connections.**
   - Location: `stream-completion.md` "Wire"; `crates/api-grpc/src/limits.rs:548-556`.
   - Problem: one call may hold a ticket of the reserve and a connection for up to 3 s (64 transactions, one after another). It competes with `Heartbeat` and `CompleteStep`, and T006-064 measures no shedding.
   - Fix: assert, from the admission counters during the fan-out burst of T006-064, that no heartbeat or completion is shed and that the wait for the pool stays bounded. Or give the call a sub-limit of its own.

9. **The 15% gate has no statistic.** The bound is one comparison on a machine whose 2026-11-12 load average was 26. Fix: state the number of interleaved runs of 1.7 and 1.8 and the statistic compared (for example the median of five). Otherwise the gate passes or fails on noise.

10. **The reservation of 1.8.**
    - Location: `sdks/SPEC.md` §9 row 1.8; D006-041.
    - Problem: OD-2026-11-15-8 and the register say a minor is taken "when its contract merges". Reserving one at design time, "at the coordinator's instruction", is an exception to an owner decision with no record in `docs/owner-decisions.md`. If ADR-0099 is accepted and the build then stops at T006-064, the register has an unserved 1.8 below every later cumulative minor.
    - Fix: record the exception, and state what releases the reservation in the stopped case.

11. **The override pass also turns on step logs.** A 1.5 scenario raised to 1.8 also speaks 1.7, and `SAGAT_STEP_LOGS_ENABLED` defaults to true from T041-7 (design 041, line 1158). A failure would be ambiguous between the two minors. Fix: run the pass with step logs off, or state that goldens carry no step-log effect.

12. **A cause set that later causes can extend was not weighed.** D006-059 (2) adds `NOT_ACKNOWLEDGED` to the closed `WithdrawalCause` of the ADR-0098 build (`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/event.rs:436-462`). Design round-1 Major 4 (a) proposed instead a cause that decodes and replays like any known one ("readers tolerate what they do not know", architecture principles, "Evolving safely"), which also covers future causes. Weigh it in D006-059.

13. **How the test driven by the descriptor works.**
    - Location: decision 9; T006-053.
    - Problem: the token is nested. It sits in `WriteBlobRequest` under the `oneof` `header` (`specs/001-platform-foundation/contracts/worker.proto:161-177`), and in the repeated `TaskAcknowledgement`. A test that enumerates methods cannot call each one generically.
    - Fix: the test walks the descriptor recursively, through nested messages, `oneof` members and repeated fields. It asserts that the set of methods carrying a token equals the set of methods its table of calls exercises, plus the four exempt ones. A method added without an entry then fails.

---

## Nits

1. The review directory is named `2026-11-21-delivery-ack`, but the reviews ran on 2026-11-20 (the ADR's status line says so).
2. The ownership check for a managed node in the acknowledgement can never be reached: the database runner stays at 1.6 (decision 12). Say so, or drop it.
3. `GET /version` (`crates/api-http/src/routes/identity.rs:68`) is served where no engine's time-to-live is known. Derive "1.8 offered" from the recorded requirement alone; recording it already implies every live engine is at 12 s or more.
4. The application service should take the lease identity type that `Heartbeat` already uses. Do not add a new data-transfer type (constitution principle II).
5. `lease_unacknowledged_withdrawals` is a hint that survives the lease and is not journaled, like `hint_count`. Say what a reset does to it, and that a wrong value is safe either way.
6. The acknowledgement's update fires `steps_quota_delta` and `steps_quota_flush` (`crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql:524-526`). The budget counts top-level statements only, so note it.
7. Constitution principle I says the losing side of a race between a timeout and a signal is "a no-op that is audited". State that `not_held` after the expiry, and `fence_missed`, are that record.
8. "Enforced by" omits the unit test of the pure function, the `promtool` cases and the unit tests of the host's clamp.
9. The activation and removal procedure belongs beside the ADR-0098 runbook (`lease-withdrawal.md`), or in a runbook of its own, with a link from `instance-registry.md`.

---

## What holds

- **Layering.**
  - Domain: the cause and the failure code. The code is a known outcome by list (`event.rs:102-119`), and an older binary replays it because failure codes are open strings.
  - Application and scheduler (both application layer in `scripts/check-architecture.py:69-70`): the decision.
  - Adapter: the SQL. No mapper is added, and no format or framework enters the domain.
- **Lock order.** The lock-only open takes the chain lock and then `SELECT run_version FROM runs … FOR UPDATE`. That is the order of `begin` and `begin_path`, so a call never holds two run locks. The generic step updates keep `lease_expires_at` unless they issue a lease or renew one (`path.rs:1229-1232`). The acknowledgement moves no `run_version`, which `begin_path`'s check against its snapshot does not need, because the marks are not in `RunState`.
- **The cost claim is now honest.**
  - 88 statements plus 6 `BEGIN`/`COMMIT` give 94 round trips. Adding about 6 statements and 1 transaction (2 more round trips) gives 102.
  - 1 − 94/102 = 7.8%, so "about 8%" is right; with several leases per frame the floor is about 5 round trips per task.
  - (30 − 6) / 6 = 4, and 64 × 0.103 s = 6.592 s.
  - Variant 2 is now compared on what it really adds.
- **ADR-0082.**
  - The capability follows the configuration (decision 7).
  - The activation has no step of its own, and the dependency check in the trigger only reads, so decision 2 holds.
  - The second transaction of the removal runs outside the epoch lock.
- **Compatibility.**
  - Leases issued before the activation carry no mark.
  - A release-0.2 binary does not register, so activation is impossible while one is live.
  - The upgrade and rollback scenario of v0.2 is planned (T006-066).
  - The proto change is additive.
- **Telemetry.** Labels come from closed sets, the gauges have none, and nothing carries a token or an identifier.

---

## Items for the owner

1. **Accept ADR-0098 first, then ADR-0099,** including the trust assumption of decision 13, with the 037 clarification extended to `ATTEMPT_TIMEOUT_BEFORE_START`, to the cancellation at acknowledgement, and to the amended hand-back (Major 3).
2. **Blocker 1.** Either every later minor (drafts 020, 021, 025, 026) is gated behind the activation of delivery acknowledgement and a lease time-to-live of at least 12 s, or the protocol takes the flag on each lease. This shapes the protocol register for good.
3. **The default of 0.3.0** (D006-057, delegate): on a default deployment the problem the owner asked to close remains until an operator runs two commands (`lease-withdrawal-activate`, then `delivery-acknowledgement-activate`). Report it under OD-2026-11-20-6.
4. **Release coupling.** Delivery acknowledgement in 0.3.0 needs:
   - the server task T041-7 of design 041 and its six SDK tasks, T041-15 to T041-20;
   - both ADR-0098 builds, with the requests of D006-059 answered (those go to the coordinator).
5. **The reservation of 1.8** as an exception to the rule of OD-2026-11-15-8 (Minor 10).
6. **The 15% bound** chosen as a delegate. Report it, together with the statistic it is measured on (Minor 9).

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 298-320, 348-400, 434-535)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (lines 318-470)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (lines 140-290)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§6.1, §6.2, §9)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md (lines 149-160)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (lines 267, 462-511, 790-920, 1563-1577, 7047-7051)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution/path.rs (lines 24-53, 1219-1240)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (lines 98-168, 312-378)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (lines 64-90)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/execution.rs (line 993)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/scheduler/src/passes.rs (lines 436-507)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs (lines 96-120)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/tests/outcome_unknown_parity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md (section 5)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql and .down.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/event.rs (lines 436-462)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/runbooks/lease-withdrawal.md
