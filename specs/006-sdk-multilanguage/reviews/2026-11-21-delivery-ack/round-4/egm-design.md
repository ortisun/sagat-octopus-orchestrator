# Design critique, round 4: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

**Review status: complete.** No target standard was supplied, so I reviewed against the persona's audit vectors and my round-3 findings.

I read:
- the persona;
- the round-4 diff;
- my round-3 report;
- Architecture decision 0099 in the worktree, and Architecture decision 0098 decisions 5 and 7 on its branch;
- `stream-completion.md`, "Delivery acknowledgement (protocol 1.8)";
- `sdks/SPEC.md` §5, §6.2 to §6.5 and §6.8;
- `research.md`: Decision 006-040 to Decision 006-061 and Gap 006-040 to Gap 006-047 here, and Decision 006-030 to Decision 006-039 with Gap 006-030 and Gap 006-031 on the server branch;
- `tasks.md`, phase "Delivery acknowledgement".

I also checked the code the new text relies on:
- the cap on the server branch: `crates/domain/src/execution/command.rs` lines 1831 to 1864; `crates/application/src/worker/withdrawal.rs` lines 171 to 264; `crates/infra-postgres/src/execution.rs` lines 7713 to 7733;
- how a poll reads the session: `crates/application/src/worker.rs` lines 944 to 1014;
- which step rows a run transaction writes: `execution.rs` lines 5336 to 5346;
- the table `steps`, its triggers and the foreign keys that reference it;
- `docs/runbooks/disaster-recovery.md` §5 to §7.

## Verdict

The core design is right and now built the way its text says:
- the acknowledgement locks rows with `SKIP LOCKED`, decides on what it locked, and answers `RETRY` for a busy row;
- the hand-back is bound to the lease;
- there is one bound on repeated withdrawals;
- the minor waits for the gate;
- a host that falls back keeps its minor.

All four round-3 Majors are resolved; Major 3 leaves a Minor behind. Three new Majors remain:
- **Major 1.** The exactness argument quietly assumes that a committed acknowledgement is never lost. On a failover to an asynchronous standby, or after a point-in-time recovery, a lost acknowledgement becomes a false "nothing ran". Today the same loss falls safely to the unknown outcome.
- **Major 2.** The fallback is triggered by the server's own trouble, counts leases rather than calls, lasts until the host restarts, and has no alert. One database brownout can switch the feature off for the whole fleet, silently.
- **Major 3.** The claim's cap check relies on a count the claim does not have. Getting it adds a journal read to the hot path that the budget pins as unchanged, and the check repeats one the domain already makes.

No Blockers. 3 Majors, 5 Minors, 4 Nits.

---

## Part 1: The round-3 Majors

**Major 1 (the hand-back judged on a session row that changes under the lease): resolved.**
- `lease_hand_back_generation` is written by the claim and read by the hand-back under the run lock (decision 2, decision 9, Decision 006-059 (3)).
- The fallback waits for the answers of its hand-backs before it registers again (`sdks/SPEC.md` §6.2, lines 371 to 373).
- The SDK rewrites a handler-raised `TASK_NOT_STARTED` at every minor (§6.5, lines 511 to 513; decision 15).
- Both tests I asked for are in Task 006-050, and the scenario holds a hand-back across a re-registration.
- In a mixed fleet before activation, engines of the 0098 build still read the session's minor. Before activation no lease is marked, so this is no worse than Decision 006-030 accepts.

**Major 2 (the cap not chosen, absent from the base, overlapping the run of three): resolved as a design.**
- The run of three is gone (option 7).
- The cap exists on the server branch: `MAX_WITHDRAWALS_PER_ATTEMPT = 4` and decision 7 of Architecture decision 0098.
- The pure function's test covers "the attempt's withdrawals" (Task 006-050).
- The scenario uses three different steps.
- Where the cap is checked, at the claim, has a false premise and a hot-path cost. That is new Major 3.

**Major 3 (the interim cost of the reservation; a failed gate that contradicted itself): resolved, with a Minor residue.**
- The contradiction is gone: the server offers 1.8 and lists the capability only from Task 006-057, and a failed gate keeps the minor at 7 (decision 13, Decision 006-041, Decision 006-057, the §9 row).
- The interim cost (a hand-back below 1.8 spends the attempt) is stated in the Consequences, in Decision 006-059 and in Gap 006-031 on the server branch, and is sent to the owner. That is the right owner and the right place.
- I had proposed decoupling the reservation from the gate. It now appears only as an owner option after a failed gate, and that option's cost is understated (Minor 4).

**Major 4 (the acknowledgement statement did not do what its text said): resolved.**
- The acknowledgement locks first (`FOR UPDATE OF steps SKIP LOCKED`), then decides on the locked rows, with `clock_timestamp()` sampled after the locks.
- `RETRY` is answered for each skipped row.
- A non-writing holder and a repeat while the first attempt commits are both barrier tests (Task 006-051).
- `cancel_requested` is read under the row lock.
- The failure rows and the exactness paragraph are corrected.
- One overstatement remains on the run's stop reason (Nit 3).

**Round-3 Minors and Nits:**
- Minors 1 to 7: resolved.
- Nits 1, 2 and 4: resolved.
- Nit 3 (the fallback lasts until restart): not resolved. With the new triggers it is now part of Major 2.

---

## Part 2: Findings

### Major 1: A committed acknowledgement that is later lost becomes a false "nothing ran"

**Location:**
- Architecture decision 0099, decision 4 (lines 71 to 75) and decision 12 (lines 112 to 118);
- `stream-completion.md`, the exactness paragraph (lines 462 to 472) and the failure table (lines 555 to 582, which has no row for this);
- Decision 006-044 and Decision 006-053;
- `docs/runbooks/disaster-recovery.md`, lines 68 to 69 (asynchronous standby, "a risk to take on purpose") and lines 79 to 93 (point-in-time recovery, and what to do after it).

**What is wrong.** Take a claim that committed and reached the standby, followed by an acknowledgement that committed on the primary but did not:
1. The host was answered `ACKNOWLEDGED` and started the handler.
2. The primary dies and the standby is promoted. A point-in-time recovery to a moment between the two commits has the same result.
3. The database now holds a provisional lease.
4. At its earlier deadline the pass withdraws it as `NOT_ACKNOWLEDGED`: the step goes back to `READY`, no attempt is consumed, no review is opened, and the journal records that nothing ran.
5. The task runs again with the same attempt number and the same idempotency key.

**Why it matters:**
- Today, losing a recent commit is safe. A lost heartbeat or a lost completion leaves the step `LEASED`, which lapses into an unknown outcome and a consultation or review (feature 037). Architecture decision 0098's two withdrawals are also safe: if their commit is lost, the lease lapses.
- 1.8 is the first mechanism where losing a recent commit produces a confident wrong answer. That is exactly what Requirement 037-006 exists to prevent.
- After a point-in-time recovery the wrong answer is certain: the restored leases are past their deadlines in wall time, so the first pass withdraws them all.
- After a failover the outcome depends on timing. If the host's next heartbeat reaches the new primary before the deadline, it is served (a violation, but the step becomes `RUNNING` and ends in a review). If it does not, the lease is withdrawn. Either way the protocol-violation and calls-after-withdrawal alerts fire for honest hosts.
- The trust assumption the owner is asked to accept (decision 12) is about the host. Nothing asks the owner to accept this assumption about the database.

**Fix (at least the first two):**
- State durable commit as part of the exactness claim and of decision 12: synchronous standby, and no point-in-time recovery past the marks without the step below.
- Add a step to the runbook's §7, "After a restore or a failover": stop marking, and turn every mark claimed before the recovery point into today's meaning (it lapses, an unknown outcome). This needs one explicit statement on the static check's allow-list.
- Better, make it mechanical:
  - record a recovery epoch that the restore and failover procedure advances (for example from the PostgreSQL timeline, where the deployment can read it, or a row the runbook updates);
  - stamp the epoch on the mark;
  - a mark from an older epoch never counts, the same way an older generation never counts.
- Add a failure row, and a case to the disaster-recovery drill: an acknowledgement lost on an asynchronous failover leads to a lapse and a review, never a `NOT_ACKNOWLEDGED` withdrawal.

### Major 2: The fallback is triggered by the server's own trouble, counts leases instead of calls, lasts until restart, and nothing alerts on it

**Location:**
- `sdks/SPEC.md` §6.2 (lines 371 to 373);
- `stream-completion.md`, "The host at 1.8", items 4 and 5 (lines 514 to 525), and "Observability" (lines 584 to 604);
- Decision 006-048;
- the fallback case of Task 006-058.

**What is wrong:**
- **Server-caused give-ups count.** The third trigger is "three leases in a row given up with no `ACKNOWLEDGED` between them". A lease is given up whatever stopped its acknowledgement:
  - `UNAVAILABLE` during a brownout;
  - `RESOURCE_EXHAUSTED` when the `Acknowledge` class is shed;
  - `DEADLINE_EXCEEDED` while the database is slow;
  - repeated `RETRY` on a row held by a long transaction, such as the cancellation or reset of a large collection.

  None of these says the host cannot acknowledge. All of them are the server's.
- **It counts leases, not calls.** A frame of 64 flagged leases whose one call cannot get through for about 18 s gives up 64 leases "in a row" at once. A single network blip between one host and its proxy trips the fallback.
- **It lasts until the host restarts** (round-3 Nit 3, still open). With the triggers above, one 20 s database incident can switch acknowledgement off for every host in the fleet, for days.
- **Nothing alerts on it.** `worker_sessions{protocol, declines}` exists, but none of the four alerts reads it. After the incident, `SagatLeasesNotAcknowledged` goes quiet because nothing is marked any more, so silence looks like health.
- **It adds load during the incident.** Every host hands back up to 64 leases (eight at a time, each spending one of the attempt's four withdrawals) and calls `RegisterWorker`, all while the database is already in trouble.

**Fix:**
- Keep the permanent fallback only for evidence that the path does not exist: `UNIMPLEMENTED`, or the refusals of item 5.
- A give-up counts toward the fallback only if no attempt for that lease was answered `RETRY`, a status that §5 repeats, or a shed. Count by frame or call, not by lease: three frames in a row.
- Make the fallback bounded: offer to acknowledge again on a later registration after a jittered interval (for example 10 minutes), except after `UNIMPLEMENTED`.
- Add an alert on `worker_sessions{declines="true"}` above zero for 15 minutes, and a server-side counter of declining registrations.
- Extend the scenario: a 64-lease frame whose call is held past its give-up does not trigger the fallback; three frames refused `UNIMPLEMENTED` do.

### Major 3: The claim's cap check relies on a count the claim does not have, costs a journal read on the hot path the budget pins as unchanged, and repeats the domain's own check

**Location:**
- Architecture decision 0099, decision 2 (lines 51 to 62: "read from the run state the claim already restored");
- `stream-completion.md`, "The marks", second bullet (lines 389 to 394: "the count the domain's withdrawal command reads, from the run state the claim already restored");
- Decision 006-052;
- Task 006-047 ("the unchanged `LEASE` line of Task 006-055") and Task 006-055;
- the acceptance matrix row "Cost".

**What is wrong:**
- **The count is not in the run state.** On the server branch, `withdraw_lease` takes `withdrawn_before` as an argument "because the state is restored from the projection, which does not keep it" (`command.rs` lines 1838 to 1842). Every caller reads it with its own query over `run_events` (`lease_withdrawals`, `execution.rs` lines 7713 to 7733), called at `withdrawal.rs` lines 229 and 262.
- **So the claim needs one more read per claimed lease.** It can be an extra statement, or a correlated subquery folded into the lease statement. An extra statement moves the `LEASE` line, which Task 006-047 and Task 006-055 pin as unchanged; a subquery keeps the count but adds a journal index probe per lease to the claim's hottest statement, which neither the budget nor the feature 038 model includes.
- **The check is redundant.** The withdrawal command already refuses at the cap, in its own transaction, under the run lock. If the claim marked every lease and let the withdrawal decide at the cap, every outcome would be the same as with an unmarked lease:
  - provisional and past its deadline at the cap: the lease expires as today, an unknown outcome;
  - a hand-back at the cap: an ordinary failure;
  - acknowledged: an attempt.

  What the claim-time check buys is a capped lease dispatched without waiting for the acknowledgement call. The price is a journal read on every claim, one matrix dimension in Task 006-047, one skip reason, and an alert.

**Fix (either):**
- **Simpler.** Drop the check from the claim. Add a row to the withdrawal table: "provisional, past a deadline, the attempt at the cap: `expire_attempt` as today, counted `capped`". Drive `SagatLeaseAttemptCapped` from the `capped` outcome of `worker_leases_undelivered_total` for the cause `NOT_ACKNOWLEDGED`. Remove `attempt_capped` from the skip reasons.
- **If the check is kept.** Correct "from the run state", specify the read (a subquery in each of the three lease statements, or one batched read per run transaction), cover it in the macro's parity test, and re-pin the `LEASE` line and the cost prediction.

### Minor 1: What a host does with a flagged lease it receives after declining is not specified

- **Location:** `sdks/SPEC.md` §6.2 (lines 371 to 373); `stream-completion.md`, host item 5; Task 006-058, the `UNIMPLEMENTED` case.
- **Problem:**
  - A poll reads the session once, at its start (`worker.rs` line 956), and then claims across several run transactions. A stream window keeps claiming for up to 5 s. So flagged leases can still arrive after the host has registered again with `declines_acknowledgement`.
  - The MUST of decision 1 still forbids running them. But the fallback text only hands back "what waits", and does not say whether a flagged lease in the fallback state triggers a second fallback.
  - An SDK that implements "declined" as "dispatch everything" runs an unacknowledged lease, which is then withdrawn and run again.
- **Fix:**
  - Specify that a flagged lease is never dispatched without `ACKNOWLEDGED`, whatever the host has declined.
  - In the fallback state it is handed back and triggers nothing further.
  - The host closes its receive before it registers again.
  - Add the case to the scenario: a frame in flight across the re-registration.

### Minor 2: Only each statement of the acknowledgement transaction is bounded, so it can commit after the host's attempt has ended

- **Location:** `stream-completion.md`, "The server's acknowledgement", step 1 (lines 414 to 415); Gap 006-041.
- **Problem:**
  - `lock_timeout` and `statement_timeout` of 3 s apply to each of about eight statements, so the transaction can outlive the host's 6 s attempt bound.
  - The host then treats the attempt as failed and may give the lease up.
  - The late commit makes the lease acknowledged, and the hand-back becomes an ordinary failure that spends the attempt.
  - Exactness holds, but the path of Gap 006-041 widens exactly when the database is slow.
- **Fix:**
  - Bound the whole transaction below half the receive bound: the application's deadline from the call's timeout, cancelling the statement when it passes, or `transaction_timeout` where the server version has it.
  - Add a test with a slow binding lock plus a slow locking read, whose total exceeds the attempt bound, and which commits nothing.

### Minor 3: The repeat after `RETRY` is not specified

- **Location:** `sdks/SPEC.md` §5 (line 280); `stream-completion.md`, host item 3.
- **Problem:**
  - "With the backoff of §5" leaves open which counter `n` is used and whether the leases answered `RETRY` in one frame are repeated together.
  - §5's `maxDelay` is a user option (default 5 s). A larger configured value can push the next ask past the give-up.
  - Six SDKs will choose six different answers, and repeating per lease multiplies calls.
- **Fix:**
  - One call for all the `RETRY` leases of a frame.
  - Its own short backoff (for example 200 ms doubling to 1 s), capped by the time left before the earliest give-up.
  - Not affected by the user's retry options.

### Minor 4: The owner's option after a failed gate costs more than it says

- **Location:** Decision 006-057 (last sentences); Owner question 006-002; Task 006-056.
- **Problem:**
  - "Ship 1.8 as 'the code reserved, the flag never set'" still needs the full host procedure in six SDKs. A later activation flags leases to every host already at 1.8, so a host without the procedure would run unacknowledged leases.
  - So that option costs nearly all of Task 006-058 to Task 006-065, for a feature that stays off.
- **Fix:** State the cost. Alternatively define a smaller host profile for that case, with its own later minor for the acknowledgement, and put it in the owner question.

### Minor 5: The cap is shared with new causes, and the wording overstates what happens at the cap

- **Location:** Decision 006-052; Decision 006-059 (5); Decision 006-031 on the server branch.
- **Problem:**
  - Decision 006-052 says that an attempt meeting a fifth cause "falls back to today's meaning rather than consuming an attempt". That is true only for silence. A hand-back at the cap spends the attempt (Architecture decision 0098 decision 7).
  - 1.8 adds three more ways to spend the shared four: silence, give-ups and `CANCELLED` hand-backs. The server branch's derivation is unchanged: request (5) has not been answered there yet.
- **Fix:**
  - Correct the sentence.
  - Make the server build's answer to request (5) a precondition, as request (2) already is, since the goldens of Task 006-058 depend on it.

### Nits

1. Architecture decision 0099, Context (lines 10 to 13), decision 9 and the hand-back section all place the cap of four in "decision 5" of Architecture decision 0098. On its branch the cap is decision 7.
2. `lock_matched` uses `FOR UPDATE`, which also conflicts with the `FOR KEY SHARE` locks that foreign-key checks take on `steps` (`blobs` (tenant_id, step_id), `approval_progress`, the database executor's table). That causes avoidable `RETRY` answers. The update changes no key column, so `FOR NO KEY UPDATE OF steps SKIP LOCKED` is enough, and it still conflicts with every withdrawal's `FOR UPDATE`.
3. Decision 006-051 says `CANCELLED` is "read under the row lock". The run's stop reason comes from the `runs` row, which `FOR UPDATE OF steps` does not lock. This is harmless (it is the same as a stop that lands just after the acknowledgement), but the text should say so.
4. `platform_requirement_dependencies` is a table with grants, a seed, a `down` and a guard that reads it both ways, all for one row. A constant pair inside the guard function would do. Keep the table only if a named later feature adds rows, and say which.

---

## Part 3: Answers to the questions

**Is it now the right design?**

Yes, in its core:
- a flag on the lease;
- marks bound to the lease's generation;
- an acknowledgement that writes one column under row locks it never waits for;
- withdrawal at the lease's own earlier deadline;
- the hand-back reserved per lease;
- one bound;
- a fallback that keeps the minor.

It is not yet right at its edges: durability (Major 1), the fallback's triggers and stickiness (Major 2), and where the cap is checked (Major 3). With those fixed I would approve it.

**Is it simpler or more complex than needed? What it adds now:**
- **Wire:** 2 fields (the flag and the decline), 1 call, 4 messages, 1 enum of 4 verdicts, 1 stable reason, the code reserved at 1.8.
- **Storage:** 6 columns on `steps`, 1 on `worker_sessions`, 1 table, a new body for the registry's guard, 1 migration with a refusing `down`.
- **Fleet control:** 1 requirement, 1 entry in the generic command, a dependency in both directions.
- **Server internals:**
  - 1 admission class and 1 withdrawal cause;
  - 1 port of 5 methods;
  - 1 SQL macro with its pure function and parity test;
  - 2 fence variants and 1 static check.
- **Signals:** about 12 new metric families and 1 extended, 4 alerts, 4 SDK hooks.
- **Host rules:** about 10: the flag, the poll bound, the concurrent acknowledgement, 4 verdicts, repeats, the clamped give-up, the fallback with 3 triggers, the stop's wait, the first-heartbeat rule.
- **Bounds and fallbacks:** 1 bound, 1 fallback.
- **Test harness:** 1 override, 2 fault actions, 2 scenarios.

Against round 3: storage grew by 1 column on `steps`, 1 on sessions and 1 table, and the wire by 1 verdict and 1 field. The rules shrank: one bound fewer, no fall to the older minor, no head-of-line blocking. Overall it is about as large as round 3, and simpler to reason about.

What does not earn its place:
- the claim-time cap check (Major 3);
- the dependency table, as a table (Nit 4).

Still marginal:
- `withdrawn_lease_token_hash`, which keeps only the last token;
- the `provisional_overdue` gauge.

`lease_hand_back_generation` and `declines_acknowledgement` both earn their places.

**Is the exactness argument sound?**

Yes, on three conditions:
- every writer that ends a provisional lease re-reads the row and decides with the pure function (the static check and the barrier tests enforce this);
- the host keeps its promise (decision 12);
- every committed acknowledgement survives. This one is unstated, and it fails on an asynchronous failover and on a point-in-time recovery (Major 1).

Within those conditions:
- the acknowledgement acts only with at least one receive bound left, and a withdrawal by the pass only past a deadline, so the two windows never overlap;
- both act only while holding the row lock;
- the clock is sampled after the lock;
- deadlock is impossible, because the acknowledgement never waits after it has taken row locks.

**Does `SKIP LOCKED` with `RETRY` behave well?**

Mostly:
- **A row held by a long transaction** (a large run's cancellation or reset): only that lease gets `RETRY`, and the rest of the frame proceeds. The host repeats until its give-up. The hand-back then waits on the row (the withdrawal path does not skip), and is withdrawn as `NOT_STARTED` once the holder commits. That costs one of the attempt's four withdrawals and, as specified, counts toward the fallback (Major 2).
- **A withdrawal in progress:** `RETRY`, then `NOT_HELD` on the next ask, because the token has been cleared. Correct, at the price of two calls.
- **A repeat while the first attempt commits:** `RETRY`, then `ACKNOWLEDGED`, counted `duplicate`.
- **A holder that locks and writes nothing** (the pass deciding to do nothing): `RETRY`, then `ACKNOWLEDGED`. The margin is intact, because the clock is sampled after the lock.
- **Spurious skips:** foreign-key share locks (Nit 2).
- **What is missing:** the repeat's schedule (Minor 3) and a bound on the whole transaction (Minor 2).
- Run transactions write only the steps they touch (`execution.rs` lines 5336 to 5346), so sibling transitions in a fan-out do not hold provisional rows.

**What will be hard to build, test or operate?**

Build:
- six host state machines: the acknowledgement concurrent with the next receive, the `RETRY` sub-loop, give-up timers that never fire during an attempt, a fallback that drains its hand-backs, the stop's wait, and the first-heartbeat rule;
- the parity between the SQL macro and the pure function;
- a static check of SQL assignments that a statement built at run time could slip past (it needs its negative tests).

Test:
- barrier tests where the acknowledgement never waits, so the barrier must be held on the other transaction;
- the failover and recovery case (Major 1);
- fallback tests with faults that distinguish the server's trouble from the host's inability.

Operate:
- the order of the two requirements;
- a cap of four shared by seven causes;
- the drain time of a rollback;
- declining sessions that accumulate silently (Major 2).

**What fails first in production?**

1. **The first database brownout, failover or proxy restart after activation.** Acknowledgement calls fail or are shed. Whole frames are given up, and hosts fall back fleet-wide until they restart, with no alert, after a storm of hand-backs and re-registrations (Major 2).
2. **A failover to an asynchronous standby, or a point-in-time recovery.** Acknowledged work is recorded as never run and runs again with no review, and honest hosts trigger violation alerts (Major 1).
3. **At build time, before production:** Task 006-047 and Task 006-055 meet the journal read the claim needs for the cap, and the pinned `LEASE` line (Major 3).
4. **Before 1.8 reaches the hosts:** hand-backs from older SDKs spend attempts. This is stated, and it is the owner's decision.
5. **Steps with an earlier deadline under 18 s:** they stay unprotected. This is now reported.

## Files referenced

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-3/egm-design.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (lines 64 to 93)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (lines 944 to 1014)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (lines 5336 to 5346)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261009000100_execution.up.sql (lines 111 to 172)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261018000000_blobs.up.sql (line 74)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql (lines 190 to 257, 524 to 526)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (decisions 5 and 7, lines 40 to 49)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/specs/006-sdk-multilanguage/research.md (lines 422 to 536)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/command.rs (lines 1831 to 1864)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs (lines 171 to 264)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs (lines 7713 to 7733)
