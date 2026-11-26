# Design critique, round 2: delivery acknowledgement (ADR-0099, worker protocol 1.8)

**Review status: complete.** I read:
- the persona and the full round-2 diff;
- my round-1 report;
- ADR-0098, ADR-0029, ADR-0015, ADR-0082 and the constitution (principles III, IV and VIII, "Targets are not promises");
- `sdks/SPEC.md` §5, §6.2 to §6.8, §9 and §10;
- the code the design leans on: `begin_row`, `begin_path` and `begin_current_path`; the chain lock; both root lease statements and the path statement; `lease_unexpired`; the heartbeat; the lost-lease candidate query; binding revocation; and the admission classes;
- the conformance scenario files.

## Verdict

The rewrite fixes the large problems of round 1:
- the loop is bounded;
- the acknowledgement no longer restores the run;
- scoped instances, the rollback, the clock and the frequency evidence are all handled;
- the cost now has an estimate and a stop rule.

It is not ready to approve, for three reasons:
- **Blocker 1.** The protocol requires hosts to hand back leases with `FailStep`, but its own rules forbid that call and record it as a violation. The same hand-back can also raise the alert that says a handler ran. Both misfire on hosts that keep the protocol, in exactly the failure this feature exists for.
- **The exactness argument has two gaps.** It uses the transaction's start time instead of the time after the lock, and its condition does not check the attempt deadline.
- **Most of what remains hard comes from one choice.** The acknowledgement renews the lease, and that renewal is what forces the run lock. Without the renewal, the run lock is not needed.

---

## Part 1: Round-1 Blockers and Majors

**Blocker 1 (an unbounded withdraw-and-reclaim loop): resolved.**
- Three `NOT_ACKNOWLEDGED` withdrawals in a row now end the provisional mark for a step (D006-052, which revises D006-006 explicitly).
- Any status that is not retryable sends the host back to 1.7.
- The give-up is counted on the host's monotonic clock, and `NOT_STARTED` withdrawals have a rate alert.
- What is left: a loop on retryable failures, paced by the time-to-live (Minor 6).

**Blocker 2 (the acknowledgement "loads nothing" yet calls `begin`): resolved.**
- There is now a lock-only open, a binding check on the activity the write returns, and a budget line that asserts no step scan, no journal insert and no outbox insert.
- The cancellation path still restores the run on purpose (Minor 4).

**Major 1 (option 5 rejected on an incomplete argument): partly resolved.**
- The new argument is correct for an acknowledgement that renews the lease.
- It never weighs an acknowledgement that does not renew, which needs no run lock (new Major 1).
- Its premise, "every write of a run's rows happens under the run lock" (ADR Context, line 27), is false: `crates/infra-postgres/src/bindings.rs` line 245 expires leases without the run lock.

**Major 2 (cost understated, no acceptance criterion): partly resolved.**
- Resolved: the per-task cost (102 against 94 round trips), the 8% estimate, the 15% stop, the networked topology and the fan-out case.
- Not resolved: the latency added before each handler has no budget (new Major 4), and the stop is measured last, after the work in six languages (new Major 7).

**Major 3 (frequency evidence misattributed): resolved** (D006-060, the ADR Context, and the counter that will measure it).

**Major 4 (rollback): resolved.**
- The closed-set request to ADR-0098 (D006-059 (2)) states the rollback floor either way.
- The removal command expires the 1.8 sessions, the host falls back on `UNIMPLEMENTED`, and the `down` migration refuses while a mark counts.
- What is left: Minor 3.

**Major 5 (one call serializes 64 runs inside the `Lease` reserve): partly resolved.**
- Added: a 3 s requirement and a busy-run case.
- Not adopted: concurrent runs, partial results, or a reserve of its own.
- New Majors 4 and 5 develop this.

**Major 6 (scoped instances unspecified): resolved.** What is left: Minor 9.

**Major 7 (the test plan cannot produce its evidence): partly resolved.**
- The override for scenarios pinned at 1.5, the named exclusions and the missing cases are good.
- But the 26 "unpinned" scenarios start no worker (Minor 1).
- The probe at a 6 s time-to-live contradicts the gate (new Major 6).

**Major 8 (the give-up rule depends on the wall clock): resolved.** The lower clamp of 12 s keeps a host retrying past the expiry, which is harmless in itself but feeds Blocker 1.

**Round-1 Minors and Nits.** Resolved: Minors 1 to 6 and 9, and Nits 1 to 4. Minor 7 is mostly resolved (Nit 5 below). Minor 8 is partly resolved (Minor 5 below). Nit 5 (the blob read waits for the acknowledgement) is not resolved (Minor 8 below).

---

## Part 2: Findings on the rewritten design

### Blocker 1: The protocol forbids, and the server punishes, the hand-backs it requires

**Location:**
- ADR-0099 decision 1 (line 43) and decision 13 (line 115).
- `research.md` D006-053 (line 391).
- `stream-completion.md`, "The host at 1.8" items 1, 4 and 5 (lines 434 to 457), and the failure table rows at lines 499 and 500.
- `sdks/SPEC.md` §6.2 (lines 356 to 370) and §6.8.
- `tasks.md` T006-054 (line 219).

**What is wrong.**
- Decision 1 says a host "MUST NOT make any other call" for a lease before `ACKNOWLEDGED`.
- The host rules require `FailStep TASK_NOT_STARTED` on leases that were never acknowledged in five cases: leases beyond the credit, leases received while stopping, `stop()` for leases still waiting, the give-up, and a refused call.
- Decision 13 counts every `FailStep` on a provisional lease as a violation and clears `acknowledgement_kept` for the session.
- The failure table counts any report that carries the token of a lease withdrawn as `NOT_ACKNOWLEDGED` as "a handler ran on a lease the journal says never ran". It audits it, and `SagatReportsAfterWithdrawal` fires on any increase.

**Why it matters.** Take the failure this feature exists for: acknowledgements cannot get through, because of a network partition or a database outage. The host gives up when the time left at receipt has passed, which is about the expiry. The pass withdraws at the expiry plus its period. So each conforming host's hand-back lands in one of two places:
- **Just before the withdrawal.** It is counted as a violation. The session silently loses its marks for life, so its later lost frames lapse into reviews again.
- **Just after the withdrawal** (always, when the clamp has raised the give-up time). It pages, and a security audit record claims a handler ran when none did.

The two signals the owner is asked to rely on when accepting the trust assumption are polluted by hosts that keep the protocol. Six SDK owners will also resolve decision 1 against §6.2 in different ways.

**Fix:**
- Reword decision 1 to "MUST NOT start a handler, and MUST NOT make any call for it other than `AcknowledgeTasks` and its hand-back (`FailStep` with `TASK_NOT_STARTED`)".
- Exempt `TASK_NOT_STARTED` from the violation rule and from `worker_reports_after_withdrawal_total`. Count a stale hand-back separately: it is evidence that nothing ran.
- In T006-054, test a give-up hand-back both before and after the withdrawal, asserting that `acknowledgement_kept` stays set and the alert counter stays at zero.
- Add a partition case to the `delivery-acknowledgement` scenario.

### Major 1: The run lock is paid for the renewal, and the renewal buys almost nothing

**Location:** ADR-0099 decision 3 (line 58) and option 5 (line 144); D006-040 and D006-043; `stream-completion.md` "The server's acknowledgement", step 3.

**What is wrong.** Option 5 is rejected because every writer that decides from the lease columns under the run lock would have to re-check them. That is true only because the acknowledgement renews `lease_expires_at`.

Consider an acknowledgement that writes only `lease_acknowledged_at` (and clears the counter):
- Only two writers decide from that column: the `NOT_ACKNOWLEDGED` withdrawal and ADR-0098's `NOT_STARTED` hand-back. The design already fences both on what they read, and rolls back with `fence_missed`.
- Its condition would be token, attempt, generation, `lease_expires_at > clock_timestamp()` and the attempt deadline. With that condition, the row lock and the read-committed re-check give "exactly one commits" without the run lock.

What the renewal buys is a fresh time-to-live at the acknowledgement:
- A healthy delivery takes under 1 s of a 30 s lease. The first heartbeat, at a third of the time left, comes about 10 s after the start either way.
- The renewal matters only for a delivery near the expiry at the 12 s floor. A margin handles that: acknowledge only while at least a third of the time-to-live is left, otherwise answer `NOT_HELD`, and the lease is withdrawn, which is safe.

**Why it matters.** The run lock causes most of what is still hard:
- serial transactions, one per run;
- a 3 s bound that the design's own upper estimate (6.592 s) misses;
- the busy-run stall (Major 4);
- the clock problem (Major 2);
- the time each call holds a `Lease` admission ticket (Major 5);
- a new lock-only open in both the root and path stores;
- about five of the six statements added per task.

Without it, the acknowledgement is one statement per call, plus the permission check and the binding read, and it never contends with completions.

**Fix.** Weigh option 5 as "no run lock, no renewal": name the two fenced writers, use the `clock_timestamp()` condition, add the margin, and handle cancellation as in Minor 4. If the run lock stays, record the renewal as the reason, and state in numbers what it buys.

### Major 2: The acknowledgement measures its deadline with the transaction's start time

**Location:** `stream-completion.md` lines 360 to 362; ADR-0099 decision 4 (line 68).

**What is wrong.** The write uses `now()` three times: `lease_acknowledged_at = now()`, `least(now() + time-to-live, …)`, and "`lease_expires_at` still in the future". `now()` is the transaction's start, which comes before the wait for the chain and run locks.

The code base deliberately does the opposite:
- `begin_row` (`execution.rs` lines 311 to 316), `begin_current_path` (line 870) and `begin_path` (line 910) sample `clock_timestamp()` after the lock, "so contention cannot backdate deadlines".
- Every condition on the lease expiry uses `clock_timestamp()`: `execution.rs` line 4835, `path.rs` line 1380, `workers.rs` line 173 and `quotas.rs`.

**Why it matters:**
- **(a) The deadline is not the expiry under contention.** An acknowledgement that waited on a busy run past the expiry is accepted. So "the deadline is literally the expiry" is false exactly when there is contention.
- **(b) The renewal is backdated by the lock wait.** If the wait approaches the time-to-live, the host gets `ACKNOWLEDGED` with an expiry already past. It starts the handler, and the pass then expires an acknowledged lease, which opens a review.
- **(c) A revoked lease can be revived.** Binding revocation (`bindings.rs` line 245) expires a doomed session's leases with `clock_timestamp()` and without the run lock. An acknowledgement whose transaction began before the revocation re-checks `lease_expires_at > now()`, finds it true, and its renewal overwrites the revocation's expiry. Because the binding patterns are "read once per call", before the per-run transactions, a revoked worker's lease is acknowledged and kept alive for another full time-to-live.

**Fix:**
- Sample `clock_timestamp()` after the run lock for the condition, the mark and the renewal, as `begin_row` does.
- Read the binding patterns inside each run's transaction, after the lock.
- Add two barrier tests: an acknowledgement waiting on the run lock across the expiry, and an acknowledgement racing a revocation.
- Correct the Context premise about the run lock.

### Major 3: The acknowledgement's condition ignores the attempt deadline

**Location:** `stream-completion.md` lines 357 to 362 and the failure row at line 494; ADR-0099 decision 7.

**What is wrong.** The claim sets two deadlines independently (`execution.rs` lines 7047 to 7058):
- `lease_expires_at` = claim time + time-to-live;
- `attempt_deadline_at` = claim time + `timeoutMs`.

So whenever `timeoutMs` is shorter than the time-to-live, the attempt deadline comes first. Consider an acknowledgement that arrives after the attempt deadline but before the pass:
1. It passes the condition, which checks only the expiry.
2. It renews to `least(…, attempt_deadline_at)`, a time already past.
3. It answers `ACKNOWLEDGED`, and the host dispatches.
4. The pass then sees a lease that is acknowledged and timed out, so `expire_attempt` runs: an unknown outcome, which opens a review.

The failure table says "a later acknowledgement is `NOT_HELD`", and decision 7 promises a known outcome.

**Fix.** For a provisional lease, also require `attempt_deadline_at IS NULL OR attempt_deadline_at > clock_timestamp()`. Add the case to the pure decision function's test matrix and to T006-052.

### Major 4: One call is serial and all-or-nothing, so one busy run delays every handler in the frame and can consume their attempts

**Location:** ADR-0099 decision 3; `stream-completion.md` steps 3 and 5 (lines 355 to 369), the failure row at line 504, and host item 4; G006-041.

**What is wrong.** The runs are handled one after another, each waiting for its lock, and the answer comes only at the end. Take a run held longer than R = 6 s (a large collection settling, a reset, a long completion):
1. The host's attempt times out, and it repeats the whole call.
2. The server has already committed acknowledgements for the runs before the busy one, but the host does not know that.
3. Each repeat stalls on the same run.
4. At the expiry the host gives up and hands back leases the server had acknowledged. Each becomes an ordinary retryable failure, and the attempt is consumed for work that never ran.

This is G006-041 generalised: from "the answer was lost" to "a neighbour in the frame was busy". A step with one attempt fails its run.

Two more problems:
- "A run that cannot be opened for a reason other than an unavailable store answers `NOT_HELD`" leaves lock and statement timeouts unclassified. If they count, a healthy lease is dropped, withdrawn one time-to-live later, and its withdrawal counter goes up.
- Every handler in a frame waits for the slowest run. The design's own bound allows 3 s at the 99th percentile, against today's 0.75 to 0.89 s from ready to handler entry, and nothing budgets that latency.

**Fix.** Adopting Major 1 removes the problem. If the run lock stays:
- Bound each run's lock wait (`lock_timeout` or `SKIP LOCKED`).
- Add a verdict meaning "not decided, repeat this one", and have the host repeat only those leases.
- Or answer within a server-side budget below R, marking the rest as pending.
- Classify timeouts explicitly.
- Add a latency criterion to T006-064: ready to handler entry at 1.8 against 1.7.

### Major 5: Putting `AcknowledgeTasks` in admission class `Lease` can starve heartbeats

**Location:** `stream-completion.md` line 291; `crates/api-grpc/src/limits.rs` lines 500 to 556.

**What is wrong.**
- Discovery calls are shed above three quarters of the limit. `Lease` calls may use the whole limit. The quarter above discovery is what protects heartbeats and reports.
- `AcknowledgeTasks` arrives as often as polls (one call per frame). It bursts with claims, and can hold its ticket and a database connection for seconds.

**Why it matters.** Under a burst, acknowledgements fill the reserve, so heartbeats and reports get `RESOURCE_EXHAUSTED`. An acknowledged lease whose heartbeats are shed lapses into a review. That is worse than at 1.7, where the claim itself is a discovery call and is shed first.

The two sheddings are not equal. A shed acknowledgement costs a withdrawal: safe, and no attempt consumed. A shed heartbeat costs a review. The priority should therefore be heartbeats and reports first, then the acknowledgement, then discovery.

**Fix:**
- Give `AcknowledgeTasks` a class of its own, shed after discovery and before `Lease`, or give it a sub-cap inside the reserve.
- Rename the class test `only_the_calls_that_keep_leased_work_alive_are_reserved_capacity` (T006-049) to match.
- Add a burst to T006-064 with the reserve full, asserting that no heartbeat is shed while acknowledgements are admitted.

### Major 6: The time-to-live floor is enforced twice, and the registry form takes an engine out of service

**Location:**
- ADR-0099 decision 11 (line 104).
- `stream-completion.md` lines 305 to 308 and 341.
- D006-047; T006-046.
- T006-057's `protocol-negotiation` probe "at 6 s that expects 1.7".

**What is wrong.**
- An engine lists the capability only if its time-to-live is at least 12 s. Under ADR-0082 decisions 3 and 7, an engine restarted at 11 s after the activation is refused and closes its whole gate: the worker API, schedules and passes all stop.
- The claim already refuses to mark a lease on an engine whose time-to-live is below 12 s. That is all safety needs: an unmarked lease keeps today's meaning, and by decision 4 safety never depends on the deadline.
- So the capability condition adds no safety. It does add a fleet-outage mode, triggered by tuning the ADR itself recommends: option 4 invites operators to shorten the time-to-live "down to 12 s".
- The test plan contradicts it. With the requirement recorded, an engine at 6 s is refused, so the probe would see `UNAVAILABLE`, not 1.7. Booted without the requirement, it sees 1.7 for another reason and tests nothing about the floor.

**Fix:**
- Drop the capability condition and keep the check at claim time.
- Count claims left unmarked because of the floor.
- Test the floor with an engine at 11 s: the lease is unmarked and the acknowledgement answers `renewed`.
- If the operator should be warned, let `--dry-run` report engines below the floor.

### Major 7: The go/no-go measurement comes after the work in six languages

**Location:** `tasks.md` T006-064 (line 261) and the phase order; D006-057.

**What is wrong.**
- The 15% stop and the 3 s call bound are measured last, after T006-057 to T006-063.
- The six SDK tasks are themselves blocked behind T041-15 to T041-20 (1.7 in six SDKs), which are not done.
- The workers of `tests/performance/scalability-ha` are the platform's own, so the measurement needs only T006-043 to T006-055.
- The design's own upper estimate misses the 3 s bound by a factor of two, and the 15% margin is a choice, not a measurement.

**Fix.** Run the measurement right after T006-049 and T006-055. Make it the gate for T006-057 and everything after it. Record the result in the ADR before the SDK work starts.

### Minor findings

**Minor 1: The 26 "unpinned" scenarios lease nothing.**
- *Location:* D006-058 (line 416) and T006-065.
- *Problem:* All 52 scenario files that start a worker pin a minor. The 26 others (builders, approvals, schedules, chain queries, state operations) start no worker. "26 run at 1.8 by the SDK default" is therefore vacuous as evidence.
- *Fix:* Drop the 26 from the evidence. The real coverage is the 29 scenarios run under the override.

**Minor 2: The attempt timeout narrows the benefit, and this is not stated.**
- *Location:* ADR-0099 decision 7 and the Consequences, "Good".
- *Problem:* For any step whose `timeoutMs` is shorter than the time-to-live, a lost frame ends as `ATTEMPT_TIMEOUT_BEFORE_START` and consumes an attempt. A step with one attempt then fails its run for a delivery fault, which is the reason option 7 was rejected.
- *Fix:* State this. Or withdraw, rather than consume, a provisional lease at its attempt deadline, bounded by the same counter.

**Minor 3: `lease-withdrawal-v1` can be removed while marks are in flight.**
- *Location:* ADR-0099 decision 11; `stream-completion.md` "Rollback".
- *Problem:* The trigger checks the requirement, not the marks. Right after `delivery-acknowledgement-remove`, an operator can remove `lease-withdrawal-v1`. This build's passes keep writing `step.lease_withdrawn` for up to one time-to-live (decision 5), while the fleet may already admit an older binary.
- *Fix:* Refuse that removal while a provisional mark counts.

**Minor 4: The cancellation path inside the acknowledgement is no longer needed.**
- *Location:* ADR-0099 decision 8; D006-051.
- *Problem:* With the mark, a provisional lease whose cancellation is requested can never lapse into a review. D006-059 (1) makes the hand-back acceptable whatever `cancel_requested` and the declared secrets say. So the server can answer `CANCELLED` without writing, the host hands the lease back, and ADR-0098's withdrawal runs the settle.
- *Fix:* Take that path. It removes the only path on which the acknowledgement restores the run. The price is a cancellation delayed by up to one time-to-live, and only when the hand-back is also lost.

**Minor 5: The alerts use absolute rates.**
- *Location:* `stream-completion.md` lines 524 to 529.
- *Problem:* The "1 a minute" thresholds of `SagatLeasesNotAcknowledged` scale with fleet size, and an autoscaling fleet hands leases back all the time. `SagatLeaseDeliverySlow` at R fires only when most of the margin is gone.
- *Fix:* Alert on the ratio of withdrawals to claims, and set the delay threshold from the baseline T006-064 measures.

**Minor 6: The give-up loop on retryable failures is still unbounded.**
- *Location:* the host rules, item 4.
- *Problem:* A host whose acknowledgements keep failing with retryable statuses (a proxy timing out the method, a full reserve) keeps claiming, waiting, and handing back. Each of its leases is delayed by one time-to-live, forever.
- *Fix:* After three give-ups in a row with no `ACKNOWLEDGED`, stop polling until an acknowledgement succeeds, or fall back to 1.7. Count it.

**Minor 7: The fallback is "for the rest of the host's life" and is visible only to the host.**
- *Location:* ADR-0099 decision 10; host item 5.
- *Problem:* One misrouted `UNIMPLEMENTED` demotes a host permanently. Only an SDK metrics hook sees it.
- *Fix:* Add a server-side gauge of live sessions by accepted minor, with a bounded label. Allow 1.8 again at the next registration after a bounded time.

**Minor 8: Blob inputs pay a second serial round trip, and the cost section does not say so.**
- *Location:* ADR-0099 decision 9 and the Consequences, "Bad".
- *Problem:* `ReadBlob` is refused on a provisional lease, so the read of a task's input blob waits for the acknowledgement.
- *Fix:* State it in the cost.

**Minor 9: Not every statement that writes a lease is named.**
- *Location:* `stream-completion.md` "The provisional mark"; T006-047.
- *Problem:* Two root statements write a lease (`execution.rs` around line 7036 and around line 7359), plus `path.rs` around line 1229. The design says "the claim's root statement", in the singular. A missed statement is safe, because the mark is bound to the generation, but its leases would silently never be provisional.
- *Fix:* Name every statement. Run the T006-047 matrix through each.

### Nits

1. `sdks/SPEC.md` §5's retry table has no row for `AcknowledgeTasks`, and §5's default of four attempts contradicts §6.2's "no limit on attempts".
2. "(30 − 6) / 6 = 4 attempts" ignores the backoff of §5 and the gap between reading a frame and sending the acknowledgement. Count each attempt's bound from the read.
3. `CANCELLED` and `NOT_HELD` lead to the same host behaviour. Say that `CANCELLED` is only a diagnostic.
4. Decision 6 states a benchmark target as a MUST ("within 3 s"). The constitution says targets are not promises. Phrase it as the build gate it is.
5. The review directory is named `2026-11-21`, but the ADR records the reviews on 2026-11-20.
6. The design does not say which rules apply to leases still waiting for their acknowledgement when the host registers again at 1.7.

---

## Part 3: Answers to the questions

**Is it now the right design?**

The family is right:
- a provisional mark bound to the generation;
- a batched acknowledgement;
- the expiry as the deadline;
- the trust assumption put to the owner.

The chosen member is not: a renewing, run-locked acknowledgement with its own cancellation path. The simpler member is an unlocked, conditional acknowledgement that does not renew, with the withdrawal fences the design already has and the hand-back for cancellation (Major 1, Minor 4).

**Is it simpler or more complex than it needs to be?**

What it adds:
- 6 columns: 5 on `steps`, 1 on `worker_sessions`;
- 1 session flag (`acknowledgement_kept`);
- 1 requirement, with a trigger guard;
- 1 remote call;
- 3 verdicts;
- 1 withdrawal cause, 1 failure code and 1 stable reason;
- 2 operator commands;
- 1 harness override;
- 11 new metrics, 1 extended metric and 3 SDK hooks;
- 4 alerts (one with two conditions);
- 6 host rules;
- 14 decisions.

What earns its place:
- the call;
- the `ACKNOWLEDGED` and `NOT_HELD` verdicts;
- `lease_offered_generation`, `lease_acknowledged_at` and `lease_unacknowledged_withdrawals`;
- the requirement and its trigger guard (it stops hosts at 1.8 from meeting old replicas during a rollout);
- the cause, and the capability refusal;
- the activation command;
- `SagatReportsAfterWithdrawal`, once Blocker 1 is fixed.

What is marginal but cheap: `lease_offered_at` (one histogram and one alert), `withdrawn_lease_token_hash` (forensic), `acknowledgement_kept` (useful once fixed), the `CANCELLED` verdict, and the session expiry in the removal command.

What does not earn its place:
- the renewal, and the run lock it forces;
- the restore and settle inside the acknowledgement;
- the time-to-live condition on the capability.

**Is the exactness argument sound?**

The mutual exclusion is sound: both writers hold the run lock, and the pass re-reads under it. Three gaps remain:
- the start-of-transaction clock (Major 2);
- the missing attempt-deadline condition (Major 3);
- a writer outside the run lock (revocation, Major 2 (c)).

"No clock enters the argument" holds for "exactly one commits". It does not hold for "the deadline is literally the expiry".

**Is the deadline derived?**

- The choice of the expiry is reasoned and safe.
- The 12 s floor is derived from specified bounds, which is an improvement. But it is a liveness floor, not a safety floor, so it should not be able to close an engine's gate (Major 6).
- R / 2 for the server is a split of the attempt budget, not a derivation, and the design's own estimate fails it.

**Is the cost honest?**

- The throughput estimate is honest, labelled as a prediction, and has a stop rule.
- Missing: the latency added before each handler (Major 4), waits on the run lock (which the round-trip model does not count), and the blob round trip (Minor 8).

**Is the trust assumption fairly stated?**

Yes: decision 13, G006-042, the clarification of FR-037-006, and the owner's acceptance as a precondition. But the signals meant to reveal violations misfire on hosts that keep the protocol (Blocker 1). The assumption also covers six implementations being correct, which the conformance suite only samples.

**What will be hard to build, test or operate?**

- **Build:**
  - the lock-only open in both stores;
  - six host state machines (an acknowledgement running beside the next receive, give-up timers, hand-backs, a fallback registration while leases are in flight);
  - the `hold` and `dropFrame` actions in six gRPC stacks.
- **Test:**
  - the barrier tests with candidates read before an acknowledgement;
  - the dependency on 1.7 in six SDKs.
- **Operate:**
  - the order of the two requirements;
  - the coupling to the floor;
  - alerts that scale with the fleet.

**What fails first in production?**

1. **The first partition or database outage after activation.** Hosts that keep the protocol hand leases back around the expiry. The result is false `SagatReportsAfterWithdrawal` pages, audit records saying a handler ran, and sessions silently demoted (Blocker 1).
2. **Overload.**
   - Acknowledgements fill the `Lease` reserve, heartbeats are shed, and acknowledged leases lapse into reviews (Major 5).
   - A busy run stalls whole frames until hosts hand back acknowledged leases and consume attempts (Major 4).
3. **An operator who lowers the time-to-live to 10 s.** The engine is refused and closes its gate (Major 6).
4. **The 15% or 3 s gate fails** only after six SDKs have been built (Major 7).

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-1/egm-design.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (lines 311 to 316, 4827 to 4856, 7036 to 7058, 7359 to 7379)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution/path.rs (around lines 1229 and 1380)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/bindings.rs (line 245)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/maintenance.rs (lines 361 to 419)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs (lines 487 to 556)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (lines 1819 to 1878)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/
