# Correctness review of ADR-0099 (delivery acknowledgement, worker protocol 1.8), round 2

## Verdict

The rewrite closes the round-1 holes the reports named:

- The acknowledgement now takes the chain lock and then the run row's lock without restoring the run, in the same order as `begin` (`crates/infra-postgres/src/execution.rs:1563-1577`, `462-511`) and `begin_path` (`execution.rs:882-920`, `790-812`). A call never holds two run locks, so the order cannot deadlock.
- A provisional lease past its expiry is refused, and the lost-lease pass re-reads the marks under the lock.
- Binding the mark to the generation is sound against the domain. Every `step.leased` must carry generation + 1 (`crates/domain/src/execution/state.rs:1497`), and the ADR-0098 withdrawal keeps the generation (sibling worktree `state.rs:1474-1479`). So a lease written later by any binary, including one that does not know the columns, has a generation the old mark cannot equal.
- Replay still equals the projection. Each decision the marks drive is written to the journal as a distinct event or code (`step.lease_withdrawn{NOT_ACKNOWLEDGED}`, `ATTEMPT_TIMEOUT_BEFORE_START`), and the domain never reads the marks.

The rewrite also introduced new defects:

- The attempt-deadline rule repeats the hole that was fixed for the expiry.
- The bound of three is cancelled by the hand-back that the host rules require.
- The same hand-back is classed as a protocol violation.
- The 1.5-pinned goldens cannot pass unblessed under the override.
- Decision 7 consumes an attempt for a pure delivery loss, which the ADR's own Consequences deny.

Counts: 0 Blockers, 5 Majors, 11 Minors, 6 Nits. Nothing was built or run; every claim about the code comes from reading it.

## Status of round-1 findings

### My report

| Round 1 | Status | Reason |
|---|---|---|
| Major 1: "loads no run state" contradicted `begin` | **Resolved** | Lock-only open specified (decision 3, D006-043); the budget line asserts no step scan. Residual: the cancellation path must restore inside the already-locked transaction, and no store entry does that for a scoped path (Minor 1). |
| Major 2: the acknowledgement accepted an expired lease; marks read outside the lock | **Resolved for the expiry** | The write requires `lease_expires_at` in the future for a provisional lease, the pass re-reads under the lock, and T006-052 holds the candidate read before the renewal. **The same hole reopens for the attempt deadline** (new Major 1). |
| Major 3: cancel-requested hand-back refused for steps with declared secrets | **Resolved** | D006-051 decides cancellation inside the acknowledgement; D006-059 (1) asks the ADR-0098 build to accept the hand-back. Residual: the text orders the decision after the acknowledging write (Minor 1). |
| Major 4: scoped instances not designed | **Mostly resolved** | New "Scoped instances" section. Residual: three statements write leases, the text names two (Minor 2). |
| Major 5: rollback unsafe | **Resolved** | The rollback floor is a build that decodes the cause; removal expires the 1.8 sessions; the host falls back on `UNIMPLEMENTED`. Residuals: Minor 6, Minor 7, Nit 3. |
| Major 6: stale marks after a rollback | **Resolved** | Generation binding, checked against `state.rs:1497` and the withdrawal's kept generation. |
| Major 7: derivation of the deadline | **Resolved** | R-based argument; the 3 s call bound is a measured requirement; the delivery delay and the call duration are measured separately. Nit 1 on the attempt count. |
| Major 8: conformance plan cannot pass | **Partly resolved** | The 3 s scenarios run below 1.8 by construction, and the override is defined. But the 1.5-pinned goldens record `"protocolVersion": "1.5"` (new Major 5). |
| Major 9: wall-clock give-up | **Resolved** | Monotonic clock, time left at receipt, clamped to between 12 s and 120 s. |

### Other reports, in my domain

| Finding | Status | Reason |
|---|---|---|
| Design Blocker 1, architecture Blocker 3, security Minor 4: unbounded withdraw-and-claim loop | **Partly resolved** | The fallback bounds refusals and the counter bounds silent loss. A host that keeps failing retryably and then hands back resets the counter and is never bounded (new Major 2). |
| Architecture Blocker 2, security Major 3: mark not tied to its lease | **Resolved** | Generation binding. |
| Security Major 1: an acknowledged lease handed back as "nothing ran" | **Resolved** | D006-045. A stop that races an in-flight acknowledgement now consumes attempts (Minor 9). |
| Architecture Major 4, design Minor 3: meaning of `timeoutMs` | **Resolved in principle** | Decision 7 keeps it counted from the claim. It brings new Major 1 and new Major 4. |
| Security Major 5: trust assumption | **Resolved in text** | The violation rule now flags conforming hand-backs (new Major 3). |
| Architecture Major 3, my Minor 2: time-to-live per engine | **Resolved** | Capability listing plus a claim-time condition. |
| Design Major 5: 64 runs one after another | **Partly resolved** | Measured requirement, no server-side bound (Minor 10). |

## Major

**Major 1: A provisional lease acknowledged after its attempt deadline, but before the pass runs, is accepted. The known `ATTEMPT_TIMEOUT_BEFORE_START` becomes an unknown outcome or a review, and the host is handed an expiry in the past.** (Confirmed in code.)

- **Where:**
  - ADR-0099 decisions 4 and 7.
  - `stream-completion.md`, "The server's acknowledgement" step 3: the expiry is checked "when the lease is provisional", the attempt deadline is not.
  - The failure-table row "The attempt deadline passes before the acknowledgement".
  - T006-051, T006-052.
- **What the code does:** the claim sets the expiry and the attempt deadline independently: `lease_expires_at = now + ttl`, `attempt_deadline_at = now + timeout` (`execution.rs:7376`, `7382-7384`, `7048-7049`; `execution/path.rs:1230`, `1234`). A renewal is `LEAST(now + ttl, attempt_deadline_at)` (`execution.rs:7377-7379`).
- **Failing scenario:** `timeoutMs = 1000`, time-to-live 30 s.
  1. The claim commits at t = 0, so the attempt deadline is t = 1.0.
  2. The acknowledgement commits at t = 1.3. The pass has not run yet; the delivery takes 751 to 889 ms at the 99th percentile.
  3. The write's predicate holds (expiry 30 > 1.3). It sets `lease_acknowledged_at` and renews to `least(31.3, 1.0) = 1.0`, and the server answers `ACKNOWLEDGED` with `lease_expires_at_epoch_ms` already in the past.
  4. The host dispatches.
  5. At t = 1.8 the pass reads "acknowledged and timed out" and calls `expire_attempt` with `ATTEMPT_TIMEOUT`, which is in `outcome_unknown_codes` (`crates/domain/src/execution/event.rs:110-119`). The result is consultation or `step.needs_review`.
- **Why the plan misses it:** T006-051 tests a provisional lease past its attempt deadline with no acknowledgement in between. T006-052 interleaves the acknowledgement only with the expiry.
- **Fix:**
  - For a provisional lease, the conditional write also requires `attempt_deadline_at IS NULL OR attempt_deadline_at > <locked clock>`; otherwise the answer is `NOT_HELD` and nothing is written.
  - Add a barrier case "acknowledgement after the attempt deadline, before the pass", root and scoped, and a row in the failure table.

**Major 2: The bound of three withdrawals does not bound the loop it was written for.** (Confirmed in the documents; the server half is confirmed in the sibling build.)

- **Where:** ADR-0099 decision 10; `stream-completion.md` "The provisional mark" (`lease_unacknowledged_withdrawals` is "cleared by an acknowledgement and by any report"); D006-052.
- **What is wrong:**
  - (a) The hand-back `FailStep(TASK_NOT_STARTED)` is a report, so it clears the counter.
  - (b) A `NOT_STARTED` withdrawal does not increment it.
  - (c) The ADR-0098 hand-back path checks status, attempt and generation but not the expiry (sibling `crates/application/src/worker.rs:2430-2436`; `verify` uses `lease_matches_instance`, which has no expiry predicate, `execution.rs:4999-5019`). So a hand-back sent after a give-up at about the expiry beats the pass and becomes `NOT_STARTED`.
  - (d) An unmarked lease (one claimed after the bound) still needs `ACKNOWLEDGED` before the host runs it, and the ADR-0098 build withdraws it on hand-back as well.
- **Failing scenario:** every attempt of the call fails with a retryable status. Causes: a layer-7 proxy that times out unknown methods (`DEADLINE_EXCEEDED`), a run whose lock is held beyond R, a bug that answers a retryable `INTERNAL`, or a host clock 15 s ahead whose give-up comes before the server's expiry. Each cycle is: claim, four attempts, give-up, hand-back, `NOT_STARTED` withdrawal with the counter back at 0, claim again. That repeats without end at the pace of the time-to-live, with two journal events per cycle and no attempt consumed. `SagatLeaseWithdrawalBound` never fires.
- **Why the plan misses it:** T006-051 tests "the fourth loss lapses" only for silent losses.
- **Fix:**
  - A `TASK_NOT_STARTED` hand-back never clears the counter.
  - Every withdrawal of a lease whose mark counts increments it, whatever the cause.
  - The host falls back to 1.7, as for a refusal, after K consecutive leases given up.
  - Add a conformance case with four give-ups in a row.

**Major 3: Decision 1 and D006-053 treat the hand-back the host rules require as a protocol violation.** (Confirmed in the documents.)

- **Where:**
  - ADR-0099 decision 1 ("MUST NOT make any other call for it" before `ACKNOWLEDGED`) and decision 13.
  - The failure-table row "A `Heartbeat`, `CompleteStep` or `FailStep` on a provisional lease".
  - T006-054.
  - Against: host item 1 (the excess and the leases received while stopping are "handed back with `TASK_NOT_STARTED` and are not acknowledged"), host item 4 (a lease given up "is handed back"), host item 5 (fallback: "hand the call's leases back"), and `sdks/SPEC.md` §6.8.
- **What happens:** a conforming host that hands back one lease has its session's `acknowledgement_kept` cleared. Its later leases are unmarked, so gap G006-001 silently reopens for that session, and `worker_protocol_violations_total` counts conforming behaviour. That pollutes the one signal the trust assumption relies on (D006-053).
- **Why the plan misses it:** T006-054 tests the rule alone. The conformance "held acknowledgement and `stopWorker`" case ends the session, so the cleared flag is never observed.
- **Fix:** exempt `FailStep` with `TASK_NOT_STARTED`, retryable, from decision 1's prohibition, from the violation rule and from clearing the counter (Major 2). Add a T006-054 case: a give-up hand-back keeps the session's next lease provisional.

**Major 4: Decision 7 consumes an attempt for a pure delivery loss whenever `timeoutMs` is below the time-to-live. The ADR's Consequences, its option-7 rejection and the failure table deny this.** (Confirmed in code and documents.)

- **Where:**
  - Decision 7.
  - Consequences, "Good": a lost frame returns "to `READY` … with the attempt number … unchanged".
  - Considered option 7, rejected because it "fails a run for a delivery fault".
  - The first failure-table row.
  - The FR-037-006 clarification.
- **What happens:** the attempt deadline is the claim plus `timeoutMs` and the expiry is the claim plus the time-to-live (code cited in Major 1). So for `timeoutMs` below 30 s, a lost `PollTasks` answer reaches the attempt deadline first and ends as `ATTEMPT_TIMEOUT_BEFORE_START`, attempt consumed.
- **Failing scenario:** `timeoutMs = 5000`, `maxAttempts = 1`, and the answer is lost. At t = 5 s the step fails for good (or takes its `onError` route), for work that never ran and a fault that was only in delivery.
- **Why the plan misses it:** no case combines a lost frame with a short timeout. The conformance lost-answer case uses the default timeout.
- **Fix:** either withdraw at the attempt deadline too (attempt not consumed, counted toward the bound of Major 2; the next lease gets a fresh `timeoutMs` from its own claim, so the meaning is unchanged), or state the exception in Consequences, the failure table and the 037 clarification. Either way, add the conformance case.

**Major 5: The override cannot keep the 1.5-pinned goldens unblessed.** (Confirmed in code.)

- **Where:** D006-058; T006-057 and T006-065; the acceptance matrix row "Six SDKs".
- **What is wrong:** the goldens record the negotiated minor, for example `tests/contract/sdk-conformance/expected/chain-final-success.json:57` (`"protocolVersion": "1.5"` under `startWorker.result`). The same field appears in the expected files of the collection, loop, chain and scoped-identity scenarios. Raised to 1.8, the observation reads `"1.8"` and the comparison fails.
- **Fix:** specify in T006-057 that under the override the harness compares `startWorker.result.protocolVersion` against the pinned value it replaced, and nothing else. Add a harness unit test that it masks only that field.

## Minor

1. **The cancellation decision is written after the acknowledging write.** In "The server's acknowledgement", step 3 sets `lease_acknowledged_at`, renews the lease and clears the counter, then returns `cancel_requested`; step 4 then says the lease "is not acknowledged".
   - **Risk:** a build that reuses the guard "ADR-0098's withdrawals require the lease not to be acknowledged" (D006-045) reads its own write and refuses, and the lease then lapses into review.
   - **Second gap:** the scoped restore needs a store entry that reads the path snapshot inside the already-locked transaction. `begin_current_path` opens its own transaction (`execution.rs:850`); `begin_path` needs a snapshot read before the lock (`execution.rs:882-909`).
   - **Fix:** the provisional branch of the write excludes `cancel_requested` rows, which go to the cancellation path unwritten. Name the scoped entry.
   - **Confidence:** plausible. T006-050 would catch a refusing guard.

2. **Three statements write a lease; the text names two.**
   - **The three:** root `upsert_step` (`execution.rs:7359-7402`); scoped in a full-run transaction, `upsert_scoped_instance` (`execution.rs:7036-7083`), reached by the full-run claim of a scoped instance (`crates/application/src/worker.rs:1669-1683`); and the path statement (`execution/path.rs:1219-1259`).
   - **Also unnamed:** `crates/infra-postgres/src/bindings.rs:244-253` writes `lease_expires_at` with no run lock. That makes the ADR context's "every write of a run's rows happens under the chain lock and the run row's lock" false. It is safe only because the write only shortens the expiry; say so.
   - **Why Minor:** generation binding makes a missed writer leave an unmarked lease, which is safe but silent.
   - **Fix:** name all three, and add a row to T006-047 for a scoped instance leased by the full-run claim.

3. **"`now()`" in the conditional write is the transaction start, taken before the lock waits.**
   - **The rule it breaks:** `begin_row` deliberately samples `clock_timestamp()` after the lock so that "contention cannot backdate deadlines" (`execution.rs:311-316`).
   - **Effect:** an acknowledgement that waited behind a busy run accepts a provisional lease that expired during the wait, and backdates `lease_acknowledged_at` and the renewal. Exactness still holds, because the pass serializes behind it. "The deadline is literally the expiry" and the delivery-delay histogram do not.
   - **Fix:** bind the clock sampled after the run lock.

4. **Bound patterns read once per call, unlocked, race a binding revocation.**
   - **Scenario:** a revocation (`bindings.rs:221-262`) commits between the call's read of the patterns and a run's write. The write renews an acknowledged or unmarked lease that the revocation had just expired: there is no expiry predicate outside the provisional case. The stale patterns then pass the binding check, so the revoked worker keeps its lease for a full time-to-live. Heartbeat reads the patterns inside its run transaction.
   - **Fix:** read the patterns under the shared binding advisory lock (as `execution/database.rs:40-47` does), or require an unexpired lease for every acknowledgement write.

5. **`NOT_HELD` is answered for leases the server does not know to be lost.** It covers "a run that cannot be opened" for transient reasons (deadlock `40P01`, serialization failure `40001`, lock timeout `55P03`, statement timeout `57014`), and a binding refused on a repeat whose earlier attempt already acknowledged the lease.
   - **Effect:** the host drops an acknowledged lease, and it lapses into review for work that never ran.
   - **Fix:** transient errors end the call with a retryable status after the runs already committed; `NOT_HELD` only for a deterministic refusal.

6. **The definitions leave out the status.** "Acknowledged" requires only that the mark counts and `lease_acknowledged_at` is set. The generation changes only on the next lease, so a SUCCEEDED row keeps a mark that "counts" forever.
   - **Effect:** the `down` migration, which "refuses while any mark counts", refuses permanently after the first 1.8 lease completes.
   - **Second gap:** the pass's table has no row for a RUNNING step with a counting, unacknowledged mark (a violating host that heartbeated first).
   - **Fix:** "a mark counts" is status `LEASED` or `RUNNING` and the generation equal. Add the RUNNING row, decided by `expire_attempt`.

7. **The dependency trigger protects only while `delivery-acknowledgement-v1` is recorded, but marks outlive its removal.**
   - **Scenario:** an operator removes `delivery-acknowledgement-v1`, then `lease-withdrawal-v1`, while provisional leases are in flight. The ADR-0098 withdrawal paths are gated on `withdraws_leases()` (sibling `withdrawal.rs:119-125`, `worker.rs:2431`).
   - **Effect:** those leases either lapse into review, if the pass and the cancellation path consult that gate, or `NOT_ACKNOWLEDGED` events are written with `lease-withdrawal-v1` unrecorded, if they do not.
   - **Fix:** say which gate they consult, and refuse removing `lease-withdrawal-v1` while `worker_leases_provisional` is above 0.

8. **The fallback to 1.7 is too broad.** "Any other status not listed in item 4" includes `CANCELLED`, `UNKNOWN` and `ABORTED`, which gRPC libraries surface on server shutdown or `RST_STREAM`.
   - **Effect:** one rolling deploy can move a fleet to 1.7 for the rest of the hosts' lives.
   - **Fix:** treat these as transport failures and fall back only on `UNIMPLEMENTED`, or on a repeated refusal. Alert on `acknowledgementFallbacks`.

9. **A stop that races an in-flight acknowledgement consumes attempts.**
   - **What §6.8 says:** stop hands back every lease "still waiting for its acknowledgement".
   - **Effect:** if the in-flight call commits first, the hand-back is an ordinary retryable failure (D006-045). Every deploy can consume attempts of tasks that never ran, which is terminal for `maxAttempts = 1`. ADR-0098 alone would withdraw them.
   - **Fix:** a stopping host waits, bounded by R, for the verdict of an in-flight call and runs what is `ACKNOWLEDGED` (stop already waits for tasks in flight).
   - **Test:** in the "held acknowledgement and `stopWorker`" case, release the hold after the stop.

10. **Head-of-line blocking inside one call.** A run whose lock is held longer than R makes every attempt time out on the host; the runs after it in request order are never reached, so all their leases are given up. The verdict set has no per-task "repeat".
    - **Fix:** bound each run's lock wait (`SET LOCAL lock_timeout` below R / 2) and answer the rest.
    - **Test:** T006-057's busy run must be held longer than R and placed first.

11. **Race cases missing from T006-052.**
    - The acknowledgement against `TASK_NOT_STARTED`, in both orders; D006-045 depends on it.
    - The acknowledgement's cancellation path against the pass.
    - The attempt-deadline case of Major 1.
    - The scoped rows, which the phase preamble requires and the task does not name.

## Nit

1. "(30 − 6) / 6 = 4 complete attempts" ignores the backoff of §5 between attempts and a `RESOURCE_EXHAUSTED` retry hint. Say "at most 4".
2. D006-048: "The clamp's lower end is 2R, the time one complete attempt needs". From receipt, one complete attempt needs R; 12 s from receipt is two attempts.
3. Rollback step 2 says "at most one lease time-to-live". The withdrawal happens when the pass looks, so it is the time-to-live plus the pass period plus any backlog.
4. The `CANCELLED` proto comment ("its run asked to stop") and the failure table ("the run ends `CANCELLED`") are too narrow. A step's `cancel_requested` is also set for the leftovers of a joined parallel (`plan.rs:671-699` in the sibling build), where the run goes on.
5. `ATTEMPT_TIMEOUT_BEFORE_START` is a code a published workflow observes where it observed `ATTEMPT_TIMEOUT` below 1.8. That is the principle-IV concern the design raises for `timeoutMs`; state it in the workflow-document reference. (Plausible.)
6. `lease_unacknowledged_withdrawals` and `acknowledgement_kept` are projection-only and cannot be rebuilt from the journal, since acknowledgements are not journaled. Say so, so that no replay-equals-projection test asserts them.

## Files consulted

- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs` (lines 282-511, 790-920, 1563-1596, 4566-4592, 4846-4867, 4999-5019, 7036-7083, 7359-7402)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution/path.rs` (lines 1219-1259)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/bindings.rs` (lines 221-262)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/maintenance.rs` (lines 361-424)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/scheduler/src/passes.rs` (lines 436-507, 4309-4539, 5601-5682)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs` (lines 1469-1509, 1650-1699)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/state.rs` (lines 1174-1569)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/command.rs` (lines 1804-1888)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/event.rs` (lines 53-120)
- `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/expected/chain-final-success.json`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs`
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs` (lines 2378-2464)
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/state.rs` (lines 1474-1527)
- `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/plan.rs` (lines 343-447, 660-706)
