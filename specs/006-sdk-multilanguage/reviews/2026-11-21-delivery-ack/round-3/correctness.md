# Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 3

## Verdict

The rework closes all five Majors of my round-2 report and the round-2 findings of the other reviewers that fall in my domain.

- The acknowledgement no longer takes the run lock and no longer renews the lease. That removes the head-of-line blocking and the renewal channel.
- Every withdrawal of a marked lease now re-reads the step row `FOR UPDATE`.
- Withdrawing at the earlier of the two deadlines keeps the attempt and keeps the meaning of `timeoutMs`.

The new design raises defects of its own. Four are Major:

1. A verdict is taken from the unlocked pre-read, so a repeated call can answer `NOT_HELD` for a lease its own earlier attempt acknowledged.
2. The margin of R = 6 s does not make the first heartbeat arrive before the expiry. Hosts schedule that heartbeat from a configured interval or from their wall clock.
3. A hand-back is judged by a session minor that can change while the lease is held.
4. Below 1.8, the hand-back that §6.2 still requires now consumes an attempt. That ends a step with one attempt for work that never ran.

Counts: 0 Blockers, 4 Majors, 9 Minors, 5 Nits. Nothing was built or run. Every claim about code comes from reading it, and every claim about PostgreSQL behaviour is marked as such.

## Status of round-2 findings

### My report

| Round 2 | Status | Reason |
|---|---|---|
| Major 1: acknowledgement after the attempt deadline accepted | **Resolved** | Step 4 of "The server's acknowledgement" requires R left before both `lease_expires_at` and `attempt_deadline_at`. Decision 5 withdraws at the earlier deadline. There is a failure-table row, and Task 006-051 has the case. Residual: the clock is not guaranteed to be read after the lock wait (Minor 2). |
| Major 2: the bound did not bound the loop | **Resolved** | Every withdrawal of a lease whose mark counts increments the counter, and only an acknowledgement or a reset clears it. The host falls back after three leases given up. The per-attempt cap bounds unmarked hand-backs, and the conformance case "four give-ups in a row" exists. Residual: the counter never clears for that step (Minor 4). |
| Major 3: the required hand-back was counted as a violation | **Resolved** | Hand-backs are exempt in decision 1, decision 12, Decision 006-053 and the failure table. Task 006-053 tests a give-up hand-back before and after the withdrawal. |
| Major 4: an attempt consumed for a delivery loss under a short `timeoutMs` | **Resolved** | The lease is withdrawn at the earlier deadline. Leases under 18 s are unmarked (Gap 006-047), and there is a conformance case with a 10 s `timeoutMs` (unflagged). Task 006-050 tests "one past its attempt deadline". |
| Major 5: 1.5-pinned goldens under the override | **Resolved in text** | Decision 006-058 and Task 006-058 compare `protocolVersion` against the pinned value, and, at 1.4, the instance identity. Harness unit tests are planned. Residual: the hand-back now differs by minor (Minor 6). |

### Other reports, in my domain

| Finding | Status | Reason |
|---|---|---|
| Design Blocker 1: unbounded withdraw-and-claim loop | **Resolved** | As my Major 2 above. |
| Security Major 2: the acknowledgement was a renewal channel | **Resolved** | It renews nothing and never touches an unmarked lease. |
| Security Major 3, my Minor 4: binding decided without the revocation's locks | **Resolved** | Each namespace is checked as `ensure_bound` checks it, under the binding's shared advisory lock (`execution.rs:4805-4825`, `bindings.rs:88-92`). |
| Architecture Major 1 | **Resolved** | Same as my Major 1. |
| Architecture Major 4: marks outliving `lease-withdrawal-v1` | **Resolved** | Removing `lease-withdrawal-v1` refuses while a mark counts, and the registry trigger refuses it while `delivery-acknowledgement-v1` is recorded. |
| Design Major 5, my Minor 10: 64 runs one after another, head-of-line blocking | **Resolved** | No run lock is taken. Residual: the acknowledgement now holds up to 64 step-row locks, in an order that is not the engine's (Minor 1). |
| My Minor 3: clock sampled before the lock | **Partly resolved** | See Minor 2. |
| My Minor 11: race cases missing | **Partly resolved** | Task 006-051 now has the acknowledgement against the pass, against the hand-back, after a deadline, and against a revocation, root and scoped. It lacks one acknowledgement against another (Major 1) and a holder that only locks (Minor 2). |

## Major

**Major 1: The verdict is taken from the unlocked pre-read. A repeated call whose pre-read comes before its own earlier attempt's commit answers `NOT_HELD` for an acknowledged lease. The host drops it, nobody runs it, and it lapses into a review.** (Confirmed in the documents and by PostgreSQL's update semantics; the occurrence is plausible.)

- **Where:** `stream-completion.md`, "The server's acknowledgement", steps 2, 4 and 5; Decision 006-043.
  - Step 5 answers `ACKNOWLEDGED` for "a row written, or a row the pre-read found already acknowledged". Anything else is `NOT_HELD`.
- **Scenario:**
  1. Attempt 1 runs its conditional statement and holds the row lock while its commit is slow. The commit is slowed by the per-row deferred quota flush: `steps_quota_flush` is `DEFERRABLE INITIALLY DEFERRED FOR EACH ROW`, three statements per updated row, so 192 for a frame of 64 (`20261024000000_durable_quotas.up.sql:526`, `296-372`).
  2. The host's 6 s bound fires. It repeats after the backoff of §5.
  3. Attempt 2's pre-read sees `lease_acknowledged_at` empty, because attempt 1 has not committed yet.
  4. Attempt 2's statement waits on the row, and attempt 1 commits.
  5. PostgreSQL rechecks attempt 2's `WHERE` on the new row version. `lease_acknowledged_at IS NULL` is now false, so the row is skipped.
  6. Attempt 2 answers `NOT_HELD`. The host drops the lease without a hand-back (host item 3).
  7. The lease is acknowledged, so it is never withdrawn. At its expiry, `expire_attempt` gives an unknown outcome, which leads to consultation or `step.needs_review`, for work that never ran.
- **Second case:** a cancellation that commits between the pre-read and the statement is answered `NOT_HELD` instead of `CANCELLED`. The host drops instead of handing back, so the cancellation waits for the earlier deadline.
- **Why the plan misses it:** Task 006-049 has "a repeat answered `duplicate`" only after a committed attempt. Task 006-051 has no case of one acknowledgement against another.
- **Fix:**
  - Lock first: `SELECT … ORDER BY <key> FOR UPDATE` over the bound rows.
  - Decide every verdict with the pure function on the locked rows. `ACKNOWLEDGED` means acknowledged with this token, by this call or an earlier one, and `CANCELLED` is read under the lock too.
  - Then update the rows that win.
  - Add a barrier test: the first call is held between its statement and its commit, the second call has done its pre-read, then the first is released. The second must answer `ACKNOWLEDGED` (`duplicate`). Root and scoped.

**Major 2: The margin of R does not make the first heartbeat arrive before the expiry. Hosts schedule that heartbeat from a configured interval with no upper bound, or from their wall clock.** (Confirmed in code for Go; plausible for the other SDKs.)

- **Where:**
  - Decision 6 and Decision 006-046: "the first heartbeat (at a third of the time left, at least 2 s) commits before the lease can expire".
  - `stream-completion.md`, "Why the deadline is the lease's own", item 3.
  - `sdks/SPEC.md` §6.4: "at 1.8 the heartbeat of a flagged lease starts when it is answered `ACKNOWLEDGED`, with the time left on the lease's own expiry".
- **What the code does:** `sdks/go/worker/task.go:304-310` returns the configured interval when one is set, whatever its size. Otherwise it returns `min(10 s, max(500 ms, time.Until(deadline)/3))`, which is wall-clock time against the server's epoch. The first beat is sent only after one interval (`task.go:313`).
- **Scenario:** an engine time-to-live of 18 s, which Considered option 6 recommends for faster recovery, or any acknowledgement that commits late. The server accepts with 6.5 s left.
  - A host configured with `heartbeatIntervalMs = 10000` beats at 10 s.
  - A host whose clock is 15 s behind computes (6.5 + 15) / 3 ≈ 7.2 s.
  - In both cases the lease expires at 6.5 s. If the pass runs in between, an acknowledged lease becomes an unknown outcome and a review.
  - §6.4 itself tells hosts with unsynchronized clocks to set an explicit interval, so these are exactly the hosts at risk.
  - Before 1.8 this was harmless: a lease arrived with about the whole time-to-live left, and the 10 s cap kept the beat inside it.
- **Why the plan misses it:** the conformance case skews the clock 60 s *ahead*, which only shortens the interval. No case has a configured interval or a clock that is behind.
- **Fix:**
  - `TaskAcknowledgementResult` carries the time left as a duration measured under the row lock (no clock comparison).
  - The first beat of a flagged lease is sent at the minimum of the normal interval and a third of that duration, on the monotonic clock, whatever interval is configured.
  - Add host unit tests (clock 60 s behind; a configured 10 s interval) and the conformance case.
  - Replace "at least 2 s" with the bound that actually matters (Nit 3).

**Major 3: The session minor that decides a hand-back can change while the lease is held. The fallback procedure itself turns hand-backs of 1.8 leases into ordinary failures that consume attempts.** (The mutation is confirmed in code; what the server build reads is plausible, because the design does not say.)

- **Where:** decision 15, the hand-back table in `stream-completion.md`, Decision 006-061, and host item 5 ("hand back what waits … and register again offering 1.7").
- **What the code does:**
  - A worker call authenticates the identity bearer, not a session (`crates/api-grpc/src/service.rs:470-482`).
  - The only link from a lease to its session is `leased_by` = session id (`worker.rs:1619`).
  - Registering again keeps that id and overwrites `accepted_protocol_minor` (`crates/infra-postgres/src/workers.rs:338-343`).
- **Scenario:**
  1. `AcknowledgeTasks` answers `UNIMPLEMENTED`, because a replica was rolled back.
  2. The host hands back its flagged leases and registers again at 1.7.
  3. Hand-backs are repeated up to 5 times with backoff (§5). Any one that commits after the re-registration reads 1.7.
  4. It becomes an ordinary retryable failure, and the attempt is consumed for a lease that never ran. With `maxAttempts = 1` the failure is terminal.
- The same happens when a host restarts under the same worker name at another minor. This contradicts Consequences: "a lease's meaning never changes while it is held".
- **Why the plan misses it:** Task 006-050 tests "from 1.8" and "from 1.7" as fixed sessions. The `UNIMPLEMENTED` conformance case does not hold a hand-back across the re-registration.
- **Fix:**
  - Record the claim-time accepted minor on the step row in the three lease statements. For a flagged lease, a counting mark already proves 1.8.
  - Decide the hand-back on that record.
  - Require the host to finish or abandon its hand-backs before it registers again.
  - Add a case where a hand-back is held at the transport hook until after the re-registration.

**Major 4: Below 1.8 the hand-back that §6.2 still requires now consumes an attempt. A deploy can end a step that has one attempt, for work that never ran.** (Confirmed in the documents.)

- **Where:** `sdks/SPEC.md` §6.2 ("A host that drops a lease it received … reports `FailStep` … `TASK_NOT_STARTED`", at every minor); decision 15; the Requirement 037-006 clarification.
- **What happens:**
  - The merged base build withdraws a hand-back from any session (`crates/application/src/worker.rs:2430-2464`). Under decision 15, a session below 1.8 gets an ordinary retryable failure instead.
  - Hosts that hand back below 1.8 are:
    - the Architecture decision 0098 SDK build;
    - a 1.8 SDK negotiated down to a 1.7 server or to the rollback floor;
    - every SDK of 0.3.0 if the gate of Task 006-056 stops the build.
  - Go and Python drop leases received while stopping (`research.md:198-199`), and with the hand-back each deploy turns them into failures.
  - With `maxAttempts = 1`, the step fails for good. Its `onError` route or the compensation of earlier steps runs.
  - That is the outcome Considered option 7 was rejected for ("fails a step with one attempt for a delivery fault"). Released 0.2 hosts do not hand back, so today such a lease goes to a review, which holds the decision for an operator.
- **Why nothing catches it:** neither Consequences nor the 037 clarification states it. Task 006-066 cannot observe it (Minor 5).
- **Fix:** either a host hands back only when the accepted minor is 1.8 or later (below that, today's lapse), or the trade-off is put to the owner and recorded, with a test using a host of the Architecture decision 0098 SDK build at 1.7.

## Minor

1. **The acknowledgement's lock order is not the engine's, so the two can deadlock.** (The order mismatch is confirmed in code; the deadlock is plausible.)
   - **The two orders:**
     - The acknowledgement locks in "primary-key order" (tenant, run, `instance_path_nodes`, `instance_path_indices`, `node`), which is the database collation (`20261027000000_scoped_instance_identity.up.sql:30-31`).
     - A run transaction writes its steps in `BTreeSet` order: byte order of `NodeId` for root steps (`execution.rs:5337-5346`), and segment by segment for `ExecutionInstanceId` (`crates/domain/src/execution/instance.rs:56-114`, `execution.rs:6173-6182`).
   - **Example:** under `en_US.UTF-8`, the default of the official image, "bill" sorts before "Ship", but `Ship` sorts before `bill` in byte order.
   - **Scenario:** a frame carries both leases of a parallel. Meanwhile a run cancellation, or the leftovers of a joined parallel, sets `cancel_requested` on both. Each transaction holds one row and waits for the other. PostgreSQL aborts one after `deadlock_timeout`, and the victim can be the engine's transaction (a cancel answered `UNAVAILABLE`, a `CompleteStep` repeated).
   - Considered option 1 ("never waits behind a run's transaction and never holds two") is also inaccurate: the call waits behind any transaction that has written one of its rows, and holds up to 64 row locks across runs.
   - **Fix:** one total order for every writer of several step rows: byte order with `COLLATE "C"` on the text keys, and the engine's `touched` sorted the same way, with a fitness test. Add a barrier case.

2. **"`clock_timestamp()` evaluated when the statement holds the row's lock" is not what a single `UPDATE` does when the holder only locked the row.** (PostgreSQL semantics; plausible.)
   - **Why:** when a holder takes the row `FOR UPDATE` and commits without writing (a withdrawal re-read that decides "nothing", the pass's condition reads), the waiting `UPDATE` proceeds without rechecking. Its `WHERE` and `SET` were evaluated before the wait.
   - **Effect:** the margin shrinks by up to `lock_timeout` (3 s), and `lease_acknowledged_at`, which feeds the delivery-delay histogram, is backdated.
   - **Fix:** the same as Major 1. Lock first, then evaluate the clock in the next statement. Add a barrier case with a holder that only locks.

3. **The host gives up while an attempt is in flight.** (Plausible.)
   - **Why:** the give-up instant equals the server's cut-off (deadline minus R) only with synchronized clocks and a time-to-live of at most 126 s. A host clock ahead, which is the conformance case of 60 s and clamps the give-up to 6 s after receipt, gives up while an attempt can still commit.
   - **Effect:** the lease is acknowledged, the hand-back is an ordinary failure, and an attempt is consumed (the failure-table row "answer lost" names only the lost-answer cause).
   - **Fix:** never give up while an attempt is in flight. Wait for its outcome, which is bounded by R, as stopping does, and run a late `ACKNOWLEDGED`.

4. **The run of withdrawals never clears once it reaches 3.** Only an acknowledgement clears it, and only a marked lease can be acknowledged. So after three withdrawals no lease of that step is marked again, for every later attempt.
   - The text says "the next lease is not marked and a fourth loss lapses".
   - **Fix:** also clear it on a report of an unmarked lease, which proves delivery, or state that it is permanent. Task 006-050 should assert the lease of the next attempt.

5. **Task 006-066 cannot fail.** "Archived v0.2 SDKs … their hand-backs answered as ordinary retryable failures": v0.2 hosts never send `TASK_NOT_STARTED` ("none reports it", `research.md:198-199`).
   - **Fix:** use a host of the Architecture decision 0098 SDK build, or a 1.8 SDK forced to 1.7.

6. **The override's "goldens unblessed" claim assumes no scenario hands back.** At 1.4 and 1.5 a hand-back is now a failure and a retry; at 1.8 it is `step.lease_withdrawn`. Any host scenario that hands back diverges: a lease arriving while `stopWorker` runs, or a frame over the credit.
   - **Fix:** the harness asserts that no `TASK_NOT_STARTED` was reported in an overridden run, or names those scenarios and excludes them.

7. **Precondition (a) is not met by the merged base.**
   - `worker.rs:2430-2436` withdraws a hand-back from any session, with no cap.
   - It decides on the snapshot of `begin`, not on a `FOR UPDATE` re-read.
   - It checks no expiry.
   - **Fix:** Task 006-050 should own the minor gate and the re-read if pull request #260 does not deliver them, rather than "as the Architecture decision 0098 build applies it".

8. **"About 6 statements per call whatever the number of leases" leaves out the per-row deferred flush.** The flush is three statements per updated row at commit, so 192 statements for 64 leases (`durable_quotas.up.sql:526`). It sits inside the 3 s bound and lengthens the window of Major 1.
   - **Fix:** state it in the cost model, or make the flush a statement-level trigger for rows whose content does not change.

9. **While waiting up to R / 2 on a row, the acknowledgement holds the binding's shared advisory lock.** A revocation queued behind it makes later shared requests of that identity and namespace wait (PostgreSQL queues them behind the waiting exclusive request): its `Heartbeat`, `CompleteStep` and `FailStep` stall for up to about 3 s. Bounded, but not stated.
   - **Fix:** say so in the failure table, or bound the wait below R / 2.

## Nit

1. Decision 2: "its expiry, or its attempt deadline when `timeoutMs` is set" contradicts `stream-completion.md` ("plus `timeoutMs` when it is shorter"). Use the earlier of the two.
2. `stream-completion.md` lines 107 and 243-244 still say "withdrawn at its expiry"; it is the earlier deadline.
3. Decision 6 and Decision 006-046: "at least 2 s" contradicts the 500 ms floor of §6.4. The argument needs an upper bound on the first beat, not a lower one (Major 2).
4. "`timeoutMs` keeps its meaning": under losses, the wall time of one attempt number can reach four times `timeoutMs`. Say so in the workflow-document reference.
5. If the per-attempt cap is below 3, a hand-back ends the protection before the run of withdrawals does. Name that interaction where the cap is recorded.

## Files consulted

- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-2/` (all four reports)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs` (282-460, 4805-4867, 5294-5346, 6137-6182, 7206-7439)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/bindings.rs` (85-274)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/maintenance.rs` (361-424)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs` (329-344)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/identity.rs` (723-760)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261027000000_scoped_instance_identity.up.sql`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql` (190-372, 524-527)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/scheduler/src/passes.rs` (5601-5682)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs` (1826-1892, 2370-2470, 2761-2884)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/instance.rs` (54-114)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs` (470-482, 567-610)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/go/worker/task.go` (297-330)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/lost-responses.json`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/expected/worker-transport-faults.json`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/durable-execution/grpc/lease_withdrawal.rs`
