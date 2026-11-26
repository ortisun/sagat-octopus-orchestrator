# Design critique, round 3: delivery acknowledgement (ADR-0099, worker protocol 1.8)

**Review status: complete.** I read:
- the persona;
- the round-3 diff;
- my round-2 report;
- ADR-0099 and ADR-0098 as they stand in the worktree;
- `stream-completion.md`: "Leases that never reach a host" and "Delivery acknowledgement (protocol 1.8)";
- `sdks/SPEC.md` §5, §6.1 to §6.5, §6.8, §9 and §10;
- `research.md`: D006-001 to D006-018 of the merged build, D006-040 to D006-061 and G006-040 to G006-047;
- `tasks.md`, phase "Delivery acknowledgement";
- the clarification of Requirement 037-006 in the diff.

I also read the code the design relies on, including the merged server build of ADR-0098:
- `crates/application/src/worker.rs`: the `TASK_NOT_STARTED` path at lines 2426 to 2464, `heartbeat`, and `ensure_bound`;
- `crates/application/src/worker/withdrawal.rs`;
- `tests/contract/durable-execution/grpc/lease_withdrawal.rs`;
- `crates/infra-postgres/src/workers.rs`, session registration at lines 328 to 369;
- `upsert_step` (`execution.rs` lines 7206 to 7405);
- the binding revocation (`bindings.rs` lines 241 to 262);
- the admission classes (`limits.rs` lines 487 to 563);
- the `steps` triggers.

## Verdict

The rewrite is the design I asked for in round 2:
- a flag on each lease;
- an acknowledgement that writes one column under the row lock, with no run lock and no renewal;
- a margin before the earlier deadline;
- withdrawal at the first of the expiry and the attempt deadline;
- `CANCELLED` writes nothing and the host hands back;
- hand-backs are exempt from violation evidence;
- the gate is measured before any SDK work.

The core is now simpler, and it is exact. The row-lock argument holds without the run lock.

The remaining problems almost all come from the new input, the reserved hand-back code. The design has taken it in as wording, not as mechanism:
- **Major 1.** Whether a hand-back is honoured depends on a session row that changes under the lease.
- **Major 2.** The per-attempt cap is named but not chosen. It is absent from the merged build. It overlaps the run of three withdrawals, with different reset rules.
- **Major 3.** The reservation makes ADR-0098's hand-back worth nothing until 1.8 ships, and ties it to a gate that may release 1.8. Neither consequence is stated.
- **Major 4.** The acknowledgement statement, as specified, does not do what the prose says. Safety holds, but verdicts, lock order and head-of-line blocking do not behave as described.

No Blockers. 4 Majors, 7 Minors, 4 Nits.

---

## Part 1: Round-2 Blockers and Majors

**Blocker 1 (the protocol forbids, and the server punishes, the hand-backs it requires): resolved.**
- Decision 1 now allows the hand-back before `ACKNOWLEDGED`.
- Decision 12 and D006-053 exempt hand-backs from violations and from the calls-after-withdrawal alert. A stale hand-back is counted on its own (`kind="hand_back"`).
- The session flag is gone.
- T006-053 tests a give-up hand-back both before and after the withdrawal.
- One gap is new, and comes from the reservation, not from this finding: the order of the fallback's hand-backs and its new registration (Major 1).

**Major 1 (the run lock is paid for the renewal): resolved.**
- Adopted as option 1. The renewal is recorded as the reason the first round needed the run lock (option 2).
- The Context no longer claims that every write happens under the run lock.

**Major 2 (the deadline was measured with `now()`): resolved.**
- `clock_timestamp()` is used.
- The binding checks run inside the transaction, ordered with revocation by the binding's advisory lock.
- T006-051 races a revocation.
- What is left: the claim that the condition is "evaluated when the statement holds the row's lock" is not what a plain `UPDATE` does (Major 4). Safety is not affected.

**Major 3 (the attempt deadline was ignored): resolved.**
- The condition requires R before both deadlines.
- The pass withdraws at the first of them.
- T006-049 has "past a deadline".
- This also resolves my round-2 Minor 2.

**Major 4 (one busy run delays the frame and consumes attempts): partly resolved.**
- Resolved: there is one transaction, so a timeout commits nothing, and a given-up frame no longer hands back leases that were acknowledged. The generalisation of G006-041 is gone. Timeouts are classified as retryable.
- Not resolved:
  - One row locked for longer than R / 2 still fails all 64 leases on every attempt.
  - The failure row says "no other run's lock is involved", but the repeat waits on the same row.
  - The latency criterion added to the gate is close to vacuous (Minor 2).

**Major 5 (admission class `Lease` can starve heartbeats): resolved.**
- `Acknowledge` is admitted up to the discovery threshold plus half the reserve.
- The gate includes a burst test asserting that no heartbeat is shed.

**Major 6 (the time-to-live floor enforced twice; an engine taken out of service): resolved.**
- The threshold is applied at the claim. An engine with a short time-to-live marks nothing instead of being refused.
- `--dry-run` lists such engines.
- T006-047 tests 17 s and 18 s.

**Major 7 (the go/no-go measurement after the six-language work): resolved.**
- T006-056 runs right after the server tasks.
- The capability stays unlisted until T006-057.
- What is left: what a failed gate does to the reserved code and to 1.8, which already contradicts itself (Major 3).

**Round-2 Minors and Nits:**
- Minors 1, 2, 3, 4, 6, 8 and 9: resolved.
- Minor 5: partly resolved. Ratio alerts were adopted, but `SagatLeaseDeliverySlow` is still set at R, not from the baseline (Nit 2).
- Minor 7: partly resolved. A gauge of sessions by minor was added, but the fallback still lasts "until the host restarts" (Nit 3).
- Nits 1, 2 and 4: resolved.
- Nit 3: overtaken. `CANCELLED` now drives a hand-back and earns its place.
- Nit 6: resolved, except for the ordering point in Major 1.

---

## Part 2: Findings on the rewritten design

### Major 1: Whether a hand-back is honoured depends on a session row that changes under the lease

**Location:**
- ADR-0099 decision 15 (lines 126 to 136);
- `stream-completion.md`, "The hand-back code at 1.8", the table at lines 315 to 319;
- `sdks/SPEC.md` §6.2 (lines 368 to 379) and §6.5 (line 472);
- D006-061;
- T006-050.

**What is wrong.**
- The server withdraws on `TASK_NOT_STARTED` only "from a session accepted at 1.8 or later". The design never says which session, or when that is read.
- Reports carry no session. `FailStep`, like `heartbeat` (`worker.rs` lines 1826 to 1851), is authorised by the principal and the lease token. So the only session the server can consult is the lease's `leased_by`, read when the report arrives.
- That row is not bound to the lease. Registration is an upsert on (tenant, subject, worker name) that keeps the row's `id` and overwrites `accepted_protocol_minor` (`workers.rs` lines 328 to 344, `ON CONFLICT … DO UPDATE SET … accepted_protocol_minor = EXCLUDED.accepted_protocol_minor … RETURNING id`). Any later registration under the same name changes what a lease already in flight means.

**Why it matters.** Two things go wrong:
- **A hand-back the protocol requires is not honoured.** The fallback (decision 9, §6.2) "hands back what waits … and registers again offering 1.7". Nothing orders the two. Hand-backs are retried up to five times with backoff, so one still in flight when the new registration commits is judged at 1.7. It becomes an ordinary failure: the attempt is consumed, and a step with one attempt fails for good. This happens on exactly the path where a host is already in trouble.
- **The reservation's purpose is defeated.** Take two processes under one worker name during a deploy: one on an SDK below 1.8, which does not rewrite the code, and one at 1.8. Their registrations flip the row. A handler on the older SDK that ran its effects and then raised `TASK_NOT_STARTED` is honoured as "nothing ran" whenever the row last said 1.8. Preventing exactly that is why the server review reserved the code.
- The rewrite is also worded "from 1.8 on" (§6.5). Read as conditional on the negotiated minor, a 1.8 SDK that has fallen back to 1.7 stops rewriting.

**Fix:**
- Bind the reservation to the lease, as the mark is bound. The claim statement records the minor the lease was issued under, for example a `steps.lease_protocol_minor smallint` written and cleared with the lease, and the hand-back is judged by that value.
- In §6.2, the fallback first waits for the answers to its hand-backs (bounded by their retry budget), then registers again.
- Make the SDK rewrite of a handler-raised `TASK_NOT_STARTED` unconditional, whatever minor was accepted.
- Add tests:
  - a hand-back landing after the same worker name registers again at 1.7, which is still withdrawn;
  - a handler-raised code from a lease issued at 1.7 while the session row says 1.8, which is an ordinary failure.

### Major 2: The per-attempt cap is named but not chosen, is absent from the build this design sits on, and overlaps the run of three

**Location:**
- decision 15;
- D006-061 and D006-052;
- the hand-back table (line 317) and the paragraph at lines 321 to 323;
- `tasks.md` "Before the first task" (a) (lines 138 to 140), T006-050 and T006-058;
- D006-059, last sentence.

**What is wrong:**
- **The merged base has no cap and no reservation.**
  - The worktree's server build withdraws on code, requirement, `LEASED`, attempt and generation alone (`worker.rs` lines 2430 to 2436).
  - Its D006-006 still reads "No cap on withdrawals of one step".
  - Its contract test withdraws a hand-back from a 1.5 stream worker (`lease_withdrawal.rs` lines 330 to 341).
  - ADR-0098 decision 5 still honours the code from any session.
  - The design says the cap is "derived by the server build … and recorded in its decisions". In this worktree no such decision exists. Precondition (a) assumes a build that is not the one merged.
- **Ownership is circular.**
  - ADR-0099's header says it refines ADR-0098 by reserving the code.
  - Decision 15 states the server's rule as a MUST.
  - D006-059 says "none changes Architecture decision 0098's decisions".
  - The rule belongs to ADR-0098, which is still Proposed, and should be written there.
- **The two bounds overlap, with different reset rules.**
  - The run of three counts every withdrawal of a marked lease and is cleared only by an acknowledgement.
  - The cap counts hand-backs "for that attempt number". Since each withdrawal sets the attempt back, that means "since an attempt was last consumed".
  - For marked leases, a cap below 3 binds first and turns a hand-back into a consumed attempt. That is the outcome D006-052 rejected ("consuming an attempt after the bound fails a run for a delivery fault").
  - A cap of 3 or more never binds on marked leases, and is then redundant there.
- **The outcomes depend on a value nobody has chosen.**
  - Several of the design's stated outcomes are true only under the cap: the failure rows "the hand-back withdraws it (`NOT_STARTED`)", the `CANCELLED` row, and the give-up rows.
  - So are the goldens of T006-058: attempt numbers after repeated give-ups, and the `CANCELLED` hand-back.
  - The pure function's inputs ("marks, status and deadlines") and its parity test do not include the cap.
  - Where the count lives is unstated: a lease-data column, or something derived by replay.

**Fix:**
- Give each bound one cause:
  - the run of withdrawals counts only `NOT_ACKNOWLEDGED`, the silences;
  - the cap counts only hand-backs.
  - An alternating loop is still bounded: three silences, then the cap, then a consumed attempt.
- Or state the constraint this design needs: for a lease whose mark counts, the cap does not apply, or is at least 3.
- Write the reservation and the cap into ADR-0098 decision 5 and revise D006-006 in place.
- Record the cap's value and where it is stored before T006-050.
- Add the cap to the pure function and its test matrix.
- Block the T006-058 goldens on it.
- Correct the last sentence of D006-059.

### Major 3: The reservation makes ADR-0098's hand-back worth nothing until 1.8 ships, ties it to a gate that may release 1.8, and the cost is not stated

**Location:**
- ADR-0099 Consequences, "Bad" (lines 173 to 179);
- decision 1;
- D006-041 against D006-057;
- the `sdks/SPEC.md` §9 row for 1.8;
- the task order T006-048 then T006-056;
- T006-066 ("their hand-backs answered as ordinary retryable failures").

**What is wrong:**
- **Every session below 1.8 now gets an ordinary failure for a hand-back.** In practice that is every session of 0.3.0 until all six SDKs speak 1.7 and then 1.8. An SDK that sends hand-backs (§6.2: stopping, a frame over its credit, a malformed frame) consumes an attempt for work that never ran. With one attempt allowed, the step fails for good: the merged test says so at `lease_withdrawal.rs` line 344. D006-005 weighed and rejected this outcome. The reservation brings it back for all older sessions, and neither ADR says so.
- **What survives a failed gate contradicts itself.**
  - D006-057 says a failed gate leaves "the capability unlisted (the feature dormant)".
  - D006-041 and §9 say 1.8 is "released … if the build stops at its gate".
  - The server starts offering 1.8 at T006-048, before the gate at T006-056. A minor that a merged server offers cannot be released.
  - If the SDK work then stops, no host ever reaches 1.8, and the reserved hand-back is never honoured by anyone.

**Fix:**
- State the interim cost in both ADRs. The choice between "consumed attempt" and "lapse into review" for older SDKs' hand-backs goes to the owner. One option: a host below 1.8 does not hand back.
- Decouple the reservation from the gate. 1.8 ships as "the code reserved, the flag defined", whatever the gate says. The small SDK part (the rewrite, the constant, the hand-back) does not wait for T006-056. A failed gate only keeps `delivery-acknowledgement-v1` unlisted, so no lease is ever flagged.
- Correct D006-041 and the §9 row to match.

### Major 4: The acknowledgement statement does not do what its prose says (safe, but the verdicts, the lock order and the head-of-line behaviour are wrong)

**Location:**
- `stream-completion.md`, "The server's acknowledgement", steps 4 and 5 (lines 388 to 398);
- the failure row "One busy run in a frame" (line 521);
- the exactness paragraph (lines 422 to 429).

**What is wrong:**
- **(a) When the condition is evaluated.** One `UPDATE … WHERE pk = ANY(…) AND condition(clock_timestamp())` evaluates its condition before it takes each row's lock. It re-evaluates only if another transaction updated the row in the meantime. Suppose a transaction holds the row `FOR UPDATE` and then releases it without writing, as the pass does when it re-reads a lease not yet past its deadline. The acknowledgement then writes with a margin eroded by the wait. That wait is bounded only by `lock_timeout` = R / 2. Safety holds, because a withdrawal only acts past a deadline. But "evaluated when the statement holds the row's lock" is not true, and the effective margin is R − R / 2.
- **(b) Lock order.** A single `UPDATE` locks rows in the order of its plan, not in primary-key order. "Locked in primary-key order" is asserted, not built.
- **(c) Verdicts.** The verdicts for rows the update did not write come from the unlocked pre-read. Take a cancellation committed between the pre-read and the update:
  - the update does not match;
  - the pre-read said "not cancelled", so the answer is `NOT_HELD`;
  - the host drops the lease without handing it back;
  - the cancellation then waits a full earlier deadline instead of settling at once, which is what decision 7 promises.
- **(d) Head-of-line blocking.** One row held for longer than R / 2 fails the whole call. Every repeat waits on the same row. At the default time-to-live, about three attempts fit before the whole frame, up to 64 leases, is given up and handed back. That costs one withdrawal and one run increment per lease, and possibly one cap increment each.

**Fix:**
- Lock first, then decide:
  - `WITH locked AS (SELECT … FROM steps WHERE (primary key) = ANY(…) ORDER BY (primary key) FOR UPDATE SKIP LOCKED)`;
  - compute every verdict from the locked rows, with `clock_timestamp()` sampled after the locks;
  - update the provisional ones.
- For the skipped rows, add a per-task verdict (for example `ACKNOWLEDGEMENT_VERDICT_RETRY`) that the host repeats on its own. The enum is new, so this costs nothing now.
- Correct the failure row and the exactness paragraph.
- Add a barrier test with a non-writing `FOR UPDATE` holder, and one with a cancellation between the pre-read and the update.

### Minor findings

**Minor 1: The margin's reasoning covers only the expiry, and it is weakest in the incident that triggers it.**
- *Location:* "Why the deadline is the lease's own", item 3; decision 6.
- *Problem:*
  - A heartbeat does not move `attempt_deadline_at`. When the attempt deadline is the earlier one, the margin's reason ("the first heartbeat commits before the lease can expire") does not apply. Any positive margin would prevent the round-2 problem, and R costs handler time.
  - When the expiry is the earlier one, an acknowledgement that commits late commits during the same database slowness that delayed it. The host then has about two beats, at 2 s intervals, to commit within 6 s.
- *Fix:*
  - State the reason for each deadline separately.
  - Have the gate measure the heartbeat's 99th percentile during the burst, and keep R only if it is far below R / 3.

**Minor 2: The latency criterion of the gate is close to a tautology.**
- *Location:* decision 13; D006-057; T006-056.
- *Problem:* At 1.8, "ready to handler entry" is the same path as at 1.7 plus the acknowledgement call. So "rises by no more than the call's 99th percentile" holds almost by construction. It says nothing about whether the added latency is acceptable.
- *Fix:* Set an absolute or relative bound on the rise, for example a share of today's 751 to 889 ms, as an owner's choice. Or call the criterion informative.

**Minor 3: The run of three counts routine causes, and its alert fires on any increase.**
- *Location:* D006-052; `SagatLeaseWithdrawalBound` (stream-completion line 548).
- *Problem:* Rolling stops hand back waiting leases. Server-side `NOT_SENT` discards under overload withdraw marked leases. Both raise the run for healthy steps, and only an acknowledgement clears it. On a fleet that deploys often, the alert pages on routine work.
- *Fix:* Count only silences (Major 2). Alert on the ratio of skips to marked claims.

**Minor 4: `NOT_HELD` carries no reason.**
- *Location:* "Observability", `worker_task_acknowledgements_total{verdict}`.
- *Problem:* Operators cannot tell these apart, and they call for different actions:
  - "too close to the deadline", which means slowness;
  - "withdrawn";
  - "binding refused";
  - "unmarked or replaced".
- *Fix:* Add a bounded `reason` label to the `not_held` series.

**Minor 5: The lock order between the acknowledgement and the hand-back withdrawal is not specified.**
- *Location:* step 3 of the acknowledgement; D006-059 (3).
- *Problem:*
  - The acknowledgement takes the binding's shared lock, then the step rows.
  - Today's `FailStep` path takes the run lock, then `ensure_bound`. Where the new `FOR UPDATE` re-read goes is unstated.
  - If the re-read comes before `ensure_bound`, a waiting exclusive revocation creates a cycle: the acknowledgement waits for the hand-back, which waits behind the revocation, which waits for the acknowledgement. Deadlocks are retryable, but they show up as failed calls.
- *Fix:* Specify the binding checks before the row re-read in every withdrawal path. Add a barrier test with a revocation queued between them.

**Minor 6: The "four give-ups in a row" scenario cannot be one step, and its golden depends on the cap.**
- *Location:* T006-058; D006-058.
- *Problem:* After three withdrawals, the step's next lease is unmarked and is dispatched at once, so a fourth give-up for that step cannot happen. Its attempt numbers also depend on the cap.
- *Fix:* Name the steps the scenario uses. Pin the golden after the cap is recorded.

**Minor 7: How much the feature covers is not measured.**
- *Location:* G006-047; D006-057; `SagatLeasesNotAcknowledged`.
- *Problem:* Every step with `timeoutMs` under 18 s is unprotected, which may be most short activities. The loss alert is a ratio to all claims, unmarked ones included.
- *Fix:* Have T006-056 report the share of claims skipped as `short_deadline`. Take the loss ratio over marked claims.

### Nits

1. "About 6 statements per call" holds for one namespace. The per-namespace checks of `ensure_bound` repeat for each namespace in the call. State the count as a base plus a per-namespace term.
2. `SagatLeaseDeliverySlow` is still set at R, not from the baseline T006-056 measures (round-2 Minor 5).
3. The fallback still lasts "until the host restarts" (round-2 Minor 7). Allow 1.8 again at a later registration after a bounded time.
4. ADR-0099 Context, lines 13 and 14: "gap Gap 006-001" repeats the word.

---

## Part 3: Answers to the questions

**Is it now the right design?**

The core is right. The new parts, the reserved code and the cap, are attached by reference to a build that does not yet contain them, so the server's answer to a hand-back depends on a session row nobody pinned to the lease. Fix Majors 1 to 3, and give the statement of Major 4 its locking CTE, and I would approve it.

**Is it simpler or more complex than needed?**

What it adds now:
- **Wire:** 1 field, 1 call, 4 messages, 1 enum of 3 verdicts, 1 stable reason, 1 reserved code (with an SDK rewrite).
- **Storage:** 5 columns on `steps`, none on sessions; 1 migration; a change to the registry's guard.
- **Fleet control:** 1 requirement, 2 commands, and 1 refusal added to an existing command.
- **Server internals:** 1 admission class, 1 withdrawal cause, 1 SQL macro with its pure function and parity test.
- **Signals:** about 10 new metric families and 1 extended, 4 alerts, 4 SDK hooks.
- **Rules and bounds:** 8 host rules, 15 decisions, 2 loop bounds (the run of three and the cap) and 2 host fallbacks.
- **Test harness:** 1 override, 2 fault actions, 2 scenarios.

That is fewer than round 2: no session flag, no renewal, no run lock, no restore inside the acknowledgement, no capability condition on the time-to-live.

What earns its place:
- the flag and the call;
- `ACKNOWLEDGED`, `NOT_HELD`, and `CANCELLED` (now a fast cancel);
- `lease_offered_generation`, `lease_acknowledged_at`, and one withdrawal counter;
- the requirement and its guard;
- the cause;
- the capability refusal;
- the admission class;
- the margin;
- the pure function with its parity test;
- the give-up clamp and the fallbacks.

Marginal:
- `lease_offered_at` (one histogram and an alert set at R);
- `withdrawn_lease_token_hash` (keeps only the last token);
- the `provisional_overdue` gauge, which repeats the pass's lag.

Does not earn its place as written: two bounds over the same hand-back loop, with different reset rules and an unknown composition (Major 2). Missing and needed: the minor recorded on the lease (Major 1).

**Is the exactness argument sound without the run lock?**

Yes, for "exactly one of the acknowledgement and a withdrawal commits":
- every withdrawal re-reads the row `FOR UPDATE` and acts only past a deadline;
- the acknowledgement acts only while at least R remains;
- an update that commits first is re-checked by PostgreSQL against the newest row version;
- revocation is ordered by the binding's lock;
- the engine's statements keep or clear `lease_acknowledged_at`, which `upsert_step` can be made to do;
- a token match implies the step is `LEASED` or `RUNNING`, because `upsert_step` clears the token otherwise.

The prose overstates the mechanism (Major 4 (a) and (b)). The effective margin is R − R / 2. The verdicts taken from the pre-read are imprecise.

**Are the deadline and the 18 s threshold derived?**

From specified bounds, yes:
- the read within R, which is real time with no clock comparison;
- one attempt bounded to R;
- a margin of R.

The threshold is correctly applied at the claim. The margin's reasoning holds only for the expiry, and no measurement checks it against heartbeat latency under load (Minor 1). R itself is a constant of the specification, not a measurement. That is acceptable, because safety does not depend on it.

**Is the cost honest?**

Mostly. It covers the per-task statements, the transaction, the blob round trip, the fan-out case and the burst, with a 15% stop rule. Missing:
- the interim cost of the reservation: consumed attempts for every hand-back from sessions below 1.8 (Major 3);
- the head-of-line cost on one locked row (Major 4 (d));
- a real latency criterion (Minor 2);
- coverage (Minor 7).

**Does the reserved hand-back code fit?**

Not yet:
- it is judged on a mutable session row (Major 1);
- the cap is unknown, does not exist in the base, and conflicts with the run of three if it is below 3 (Major 2);
- its value to ADR-0098 is deferred until 1.8, and is lost if the gate fails (Major 3).

The rewrite of a handler-raised code is right, but it should not depend on the negotiated minor.

**What will be hard to build, test or operate?**

Build:
- the locking CTE, and verdicts decided from locked rows;
- the per-lease minor;
- the cap's storage;
- six host state machines: the acknowledgement concurrent with the next receive, give-up timers, the stop's wait, and a fallback that must drain its hand-backs before registering again.

Test:
- barrier tests on PostgreSQL's re-check of a concurrently updated row, and on non-writing lock holders;
- goldens that depend on the cap;
- the dependency on 1.7 in six SDKs.

Operate:
- the order of the two requirements;
- a cap whose value lives in another branch's decisions;
- `NOT_HELD` without a reason;
- a withdrawal-bound alert that pages on deploys.

**What fails first in production?**

1. **The first rolling deploy of 0.3.0, before any SDK reaches 1.8.** Hand-backs from older SDKs consume attempts, and steps with one attempt fail for good (Major 3).
2. **A host that falls back, or a worker name shared across a deploy.** Hand-backs are judged at the wrong minor: either consumed, or a handler's false "nothing ran" is honoured (Major 1).
3. **A large run's cancellation or reset holding step rows.** The frames touching it fail every acknowledgement attempt and are handed back whole (Major 4 (d)). `SagatLeaseWithdrawalBound` pages on routine stops (Minor 3).
4. **A database brownout.** Late acknowledgements leave about 6 s for the first heartbeat, acknowledged leases lapse into reviews, and the only signal is `NOT_HELD` with no reason (Minor 1, Minor 4).

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-2/egm-design.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (lines 1826 to 1851, 2426 to 2464, 2761 to 2815)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/durable-execution/grpc/lease_withdrawal.rs (lines 330 to 383)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/workers.rs (lines 328 to 369)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (lines 7206 to 7405)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/bindings.rs (lines 241 to 262)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs (lines 487 to 563)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql (lines 190 to 226, 524 to 526)
