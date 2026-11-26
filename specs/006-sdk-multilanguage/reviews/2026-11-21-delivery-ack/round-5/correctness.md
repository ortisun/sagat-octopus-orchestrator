# Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 5

## Verdict

The round-5 text resolves all three of my round-4 Majors. It also resolves the round-4 Majors of the other reports that fall in my domain:

- **The cap.** It is applied in one place, the withdrawal command. Its journal count is read in the same transaction.
- **Hand-back after expiry.** A provisional lease's hand-back is now withdrawn after its expiry too.
- **Fallback.** It counts calls, ignores the server's statuses, is bounded and is alerted.
- **Engine switch.** It no longer reaches a marked lease.
- **Requirement read.** Every withdrawal of a marked lease reads `lease-withdrawal-v1` under its share lock.
- **Floor build.** The floor clamps the holder's minor. `PROTOCOL_MINOR` is 6 on the server branch (`crates/application/src/worker.rs:75`), so `HAND_BACK_PROTOCOL_MINOR = 8` (`withdrawal.rs:23`) is never reached there.
- **Timeline rule.** It is sound for a promotion and for an archive recovery.

The changes bring two new Majors:

1. Some outcomes of the lost-lease pass write nothing for a lease that has expired. That pass is the scheduler's only expiry pass, so such a lease is never ended, and it starves the window of that pass.
2. When `AcknowledgeTasks` alone fails with a retryable status, neither side stops. The host never falls back and the server keeps marking, so flagged leases cycle through four hand-backs and then spend their attempts.

**Counts:** 0 Blockers, 2 Majors, 6 Minors, 6 Nits.

Nothing was built or run. Claims about the code come from reading it, at the paths given. Claims about PostgreSQL behaviour are marked as such, from its source as I know it; Task 006-044 must check them on the pinned image.

## Status of round-4 findings

### My report

| Round 4 | Status | Reason |
|---|---|---|
| Major 1: the claim's withdrawal count is not in the restored state | **Resolved** | The claim no longer reads the count. The withdrawal command applies the cap from `lease_withdrawals`, as `withdraw_lease` takes it (`crates/domain/src/execution/command.rs:1849-1864`). The `LEASE` line keeps its statements. One new defect is at the edge: what the pass does at the cap (Major 1). |
| Major 2: a hand-back after expiry spends the attempt | **Resolved** | The expiry condition is lifted for a lease whose mark counts and that is not acknowledged (`stream-completion.md` lines 346-349, the withdrawals table at line 504). Task 006-051 has "a hand-back 1 ms after the expiry with the pass held", and the same after `CANCELLED`. The pass and the hand-back are ordered by the row lock; the loser is stale and counted apart. |
| Major 3: the fallback fires on outages and lasts until restart | **Resolved** | Calls are counted, not leases. The server's statuses never count. The decline is bounded (10 minutes, doubling to an hour) and alerted. The rule now overcorrects (Major 2). |
| Minor 1: first-heartbeat margin | **Resolved, residue** | The formula is "a third of the time left, less the attempt's duration". The field is clamped at 0, and the host beats at once under 1.5 s. The derivation's numbers describe a different formula (Nit 1). The `duplicate` verdict still has no margin, and the new `recovered` verdict inherits that (Minor 4). |
| Minor 2: rollback through the switch | **Resolved** | The switch no longer reaches a marked lease. Request (6) of Decision 006-059 asks Architecture decision 0098 to say so. |
| Minor 3: the floor reads the session row | **Resolved** | The clamp is request (3). Task 006-066 covers it with a session stored at 8, with and without a capability served. |
| Minor 4: the capability record is not a fence | **Resolved** | The record is a conditional update, committed before the serve. The root blob paths hold the run lock (`check_blob_access` opens with `Lock::Wait`, `crates/application/src/worker.rs:2125`), so the hand-back race is ordered. A barrier test covers both orders. |
| Minor 5: conformance mechanism and exclusions | **Resolved** | `RETRY` moved to server and unit tests. The exclusions are found by a run and listed by name. |
| Minor 6: a declining host's flagged lease | **Resolved** | It is handed back and never dispatched. |
| Nits 1 to 6 | **Resolved** | The clock is a column, "1.8 or later" is used throughout, `RETRY` has its own backoff, the stop wait is within `timeout`, selection is by primary key alone, and the skip-reason precedence and the upsert are fixed. |

### Other round-4 reports, in my domain

| Finding | Status | Reason |
|---|---|---|
| Design Major 1: a committed acknowledgement that is later lost | **Resolved for promotion and archive recovery, partly otherwise** | The timeline rule. The same-timeline restores the runbook itself allows are not named (Minor 3). The comparison is worded "earlier" where it must be "other" (Minor 6). |
| Design Major 2: the fallback triggers | **Resolved** | Residue in Major 2. |
| Design Major 3 and architecture Major 3: the claim's cap check | **Resolved** | As my Major 1. |
| Architecture Major 1: the switch | **Resolved** | |
| Architecture Major 2: the pass and the requirement read | **Resolved for the floor race** | The fix's wording "it writes nothing, and the lease lapses" was copied into a pass that is itself the expiry path (Major 1). |
| Security Major 1: the floor decides from the session row | **Resolved** | By the clamp. |

## Blocker

None.

## Major

**Major 1. The lost-lease pass, which is the scheduler's only expiry pass, has outcomes that write nothing for an expired lease. Such a lease is never ended, heads every later window, and can starve expiry on its shards.** (Confirmed in the documents and in the structure of the code.)

- **Where:**
  - `stream-completion.md`, the withdrawals table, line 508: "`lease-withdrawal-v1` not recorded | Nothing written; the lease lapses, counted `lapsed`".
  - The failure row at line 651: "a withdrawal after the removal writes nothing (`lapsed`)".
  - Task 006-050, line 229: "after four withdrawals of the attempt a marked lease lapses (`capped`)".
  - The table row at line 503 says `expire_attempt` at the cap, but the task's wording follows the server build's `Capped`, which writes nothing (`withdrawal.rs:376-380`, `447-451`).
- **What the code does:**
  - `lost_leases` selects every `LEASED` or `RUNNING` step whose `lease_expires_at` or `attempt_deadline_at` has passed, ordered by that deadline, with a limit (`crates/infra-postgres/src/maintenance.rs:361-389`).
  - The pass's only writer is `expire_attempt` (`crates/scheduler/src/passes.rs:5601-5650`).
  - In Architecture decision 0098, "lapses" meant "left to this pass". Here the writer is this pass.
- **Scenario:**
  1. Both requirements are recorded. A lease is marked and its frame is lost.
  2. The operator rolls back: they remove `delivery-acknowledgement-v1`, then `lease-withdrawal-v1`. The second removal is allowed because no journal holds `step.lease_withdrawn` yet; this is the design's own failure row.
  3. At the lease's expiry the pass reads the requirement under its lock, finds it absent, and writes nothing.
  4. Every later pass picks the same row first, because it has the oldest deadline, and writes nothing again.
  5. The step stays `LEASED` forever. `worker_leases_marked` never reaches 0, so step 2 of the rollback never ends. With as many such rows as the window's limit, no other lease on those shards is expired.
  6. The same happens at the cap if Task 006-050's "lapses" is built as the server build's `Capped`. That is likely, since the command and its outcome are reused.
  7. In both cases the counter is incremented on every pass, which inflates the `SagatLeaseAttemptCapped` ratio.
- **Why the tests miss it:** Task 006-051's barrier test ("the removal of `lease-withdrawal-v1` held between the pass's read and its commit") asserts the journal at that moment, not that a later pass ends the lease. Task 006-050 asserts "lapses (`capped`)", which a no-op satisfies.
- **Smallest fix:**
  - State in the table, in decision 5 and in Task 006-050 that in the pass every outcome other than a withdrawal ends the attempt with `expire_attempt` in the same transaction, as today. That covers the requirement not being recorded (counted `lapsed`), the cap (counted `capped`) and a fence that still holds but cannot be withdrawn.
  - Say that the binding checks named for "every withdrawal" do not apply to the pass, which is a system actor, or that a failed check also falls through to `expire_attempt`.
  - Test, root and scoped: with the requirement removed and the pass run twice, the first pass appends the expiry and the second finds nothing. Do the same at the cap.

**Major 2. When `AcknowledgeTasks` alone fails with a status the server returns as retryable, nothing stops it on either side. Every flagged lease waits out its give-up, is handed back up to four times, and then spends its attempt.** (Plausible in operation; confirmed in the documents that no rule stops it.)

- **Where:**
  - `sdks/SPEC.md` §6.2, lines 372-375: "a call that met `UNAVAILABLE`, `RESOURCE_EXHAUSTED`, a server deadline or `RETRY` never counts".
  - `stream-completion.md` host item 5 (lines 582-587) and the failure row at line 646.
  - "Negotiation and activation": the engine marks on the requirement, the switch and the timeline alone.
  - "Observability", lines 672-679.
- **Scenarios** (each one fails only this call, while heartbeats and receives are answered):
  - A 64-lease call whose work before the update passes the new 3 s call deadline, or whose update and its triggers pass the 1 s statement timeout. Each attempt repeats the same work and fails the same way.
  - A defect, or a contended lock, in the new binding and membership path of `AcknowledgementTx`.
  - A gateway that answers an unrouted method with HTTP 503, which gRPC maps to `UNAVAILABLE`.
  - The class `Acknowledge` shed under load while the class `Lease` is admitted, as the reserve is designed to do.
- **What happens:**
  1. The host never counts these calls, so it never falls back.
  2. The server keeps marking.
  3. Each flagged lease waits until its give-up (24 s at the default 30 s time-to-live), is handed back (a run transaction, a journal event, a new claim), and is claimed and flagged again.
  4. After four rounds its hand-back spends the attempt. With `maxAttempts = 1` that is a terminal failure: the `onError` route or the compensation of earlier steps runs, for work that never started.
  5. Under load each round adds a hand-back transaction and a claim, so the overload feeds itself.
- **Detection is late:**
  - `SagatLeasesNotAcknowledged` counts the cause `NOT_ACKNOWLEDGED`, but these leases come back as `NOT_STARTED`.
  - `SagatLeaseDeliverySlow` has no samples, because nothing is acknowledged.
  - Only `SagatLeaseAttemptCapped` fires, after four rounds per attempt.
  - Nothing alerts on `grpc_requests_total{method="AcknowledgeTasks", code}`.
- **Why the tests miss it:** the only negative case is "a 30 s `UNAVAILABLE` outage … does not fall back" (Task 006-058). Nothing asserts that a lasting failure of this call alone ends.
- **Smallest fix:**
  - The rule's own discriminator, "other calls to the same server were answered meanwhile", already separates an outage (heartbeats fail too) from a failure of this call alone. Apply it to the server's statuses too, over a longer run, for example five calls or two minutes. Now that the decline is bounded and alerted, a fallback during shedding only sheds more load.
  - Add a server-side breaker: an engine stops marking (skip reason `acknowledgement_failing`) while the share of its `AcknowledgeTasks` calls that end in a transient error over the last minute is above a threshold.
  - On a server deadline or a statement timeout, the host halves the call at its next attempt.
  - Add alerts on that error ratio, and on `NOT_STARTED` withdrawals of marked leases.
  - Tests: a conformance case where the call fails with `UNAVAILABLE` for two minutes while the stream is answered, which declines or stops marking and spends no attempt; and a server case with a 64-lease call held past its deadline, which is split and then acknowledged.

## Minor

**Minor 1. The gate's call-duration criterion can no longer fail.** (Confirmed in the documents.)

- **Where:** step 1 of "The server's acknowledgement" (lines 450-453); Task 006-056; decision 13.
- **What is wrong:**
  - The new deadline rolls a call back at 3 s and answers a retryable status.
  - "The call is within 3 s at the 99th percentile" is then met by construction, unless only successful calls are measured.
  - `worker_task_acknowledgement_call_seconds` also includes calls that were rolled back.
- **Fix:** the gate measures successful calls only, and also requires that the share of calls ended by the deadline or a timeout stays below a stated bound (for example 0.1%). Record both figures in the report.

**Minor 2. The deadline check does not do what step 1 claims ("so no acknowledgement commits after the host has given its attempt up").** (Confirmed by the arithmetic.)

- **The arithmetic:**
  - The server's 3 s run from its own receipt of the request. After the check come the update (up to 1 s), the `COMMIT` (which may wait for a synchronous standby) and the transit of the answer.
  - The host's 6 s bound started before it sent the request.
  - An inbound transit above about 2 s, or a slow commit, lands the commit after the host's attempt bound.
- **Consequence:** this is the documented row "answer lost after its commit". The hand-back then spends the attempt, so this is a Minor.
- **Related:** the retryable status for the server's deadline is not named. The fallback rule distinguishes "a server deadline" from the host's own bound, and gRPC libraries report both as `DEADLINE_EXCEEDED`.
- **Fix:**
  - Take the deadline as the smaller of 3 s and the request's propagated `grpc-timeout` less 2 s, and write "rarely", not "no acknowledgement".
  - Answer the server's deadline with `UNAVAILABLE` or `ABORTED`, with a reason in the status details, never `DEADLINE_EXCEEDED`.

**Minor 3. Restores that keep the timeline are named only as "a logical dump", but the runbook itself allows two more.** (Plausible; PostgreSQL behaviour.)

- **Where:** decision 12 (line 135 of the decision record); Gap 006-048; the failure row at line 655; `docs/runbooks/disaster-recovery.md` sections 3 and 7.
- **What is wrong:**
  - PostgreSQL chooses a new timeline only at the end of an archive recovery.
  - The runbook's section 3 allows "your platform's snapshot tool" for base backups.
  - A `pg_basebackup -X stream` backup can be started without `recovery.signal`: that is crash recovery to the end of the backup, on the same timeline.
  - A volume snapshot or a managed service's snapshot restore is also crash recovery, on the same timeline.
  - Each of these loses the commits made after its point.
- **Scenario:** with the restore objective of up to an hour, every restored provisional lease is past its expiry. The first pass withdraws those whose acknowledgement was lost, so handlers that ran are journaled as not run.
- **Fix:**
  - State the rule as "every restore that does not end an archive recovery with a promotion", and name the three cases in decision 12, Gap 006-048, the failure table and the runbook step.
  - Make the drill restore a snapshot without archive recovery and check that the step ends the marks.

**Minor 4. The `duplicate` verdict, and the new `recovered` verdict, carry no margin.** (Confirmed in the documents.)

- **Where:** step 5 (lines 476-480) answers `ACKNOWLEDGED` for "already acknowledged with the same token, or a mark of an earlier timeline with the same token" without the one receive bound a winner needs.
- **What happens:**
  - With `lease_time_left_ms` clamped to 0, the host starts a handler whose first heartbeat cannot commit before the expiry. That is exactly the reason decision 6 gives for the margin.
  - After a failover, every recovered lease that is past its expiry but not yet reaped is run. It is then reaped as an unknown outcome while it runs, so an external effect happens after the platform has stopped tracking it.
- **Fix:** answer `ACKNOWLEDGED` on both paths only with at least one receive bound left before both deadlines; otherwise answer `NOT_HELD` (`too_late`). The outcome is an unknown outcome either way, but no handler starts on a lease the platform is about to end. Add the rows to the parity test.

**Minor 5. At the cap, the same knowledge ends in a review or in a retry, depending on which of two concurrent transactions holds the row first.** (Confirmed in the documents.)

- **Where:** the table rows at lines 503 and 505.
- **The two orders**, for a provisional lease past its expiry at the cap:
  - If the pass is first, it calls `expire_attempt`: an unknown outcome, so consultation or review for an `Activity`.
  - If the hand-back is first, it is an ordinary retryable failure, with no review.
- **Why it matters:**
  - Both rest on the same trust assumption ("nothing ran"), under which the pass's withdrawal below the cap records a known outcome.
  - Principle I holds (the first commit wins).
  - The host's give-up puts the hand-back right around the expiry, so both orders occur regularly.
- **Fix:**
  - Choose one outcome. Either at the cap the pass ends a provisional lease as an ordinary retryable failure with a code of its own (the attempt spent, no review), which matches the hand-back and decision 12. Or state why a review is wanted, and make the hand-back at the cap match it.
  - Test both orders.

**Minor 6. "A mark of an earlier timeline" must mean "another timeline, or none".** (Confirmed in the wording; the implementation risk is plausible.)

- **Where the wording differs:**
  - "Earlier" in decision 2 (line 66), "The timeline", step 5, Decision 006-062 and the failure table.
  - "The database's current one" in "The marks" and "another timeline" in Task 006-047.
- **What goes wrong with "earlier":**
  - A logical restore into a fresh cluster (timeline 1) of rows stamped 3 is not "earlier".
  - The null that `end_lease_marks()` writes is not "earlier" either.
  - A `<` comparison counts both as provisional. The first is the false "nothing ran" the rule exists to prevent; the second defeats the runbook step itself.
- **Fix:**
  - Use one wording everywhere: "a timeline other than the current one, or none".
  - The macro counts a mark only when the stamped timeline equals the current one, so a different value or a null never counts.
  - Add parity-test rows for a greater timeline and for a null.

## Nit

1. **The first-heartbeat derivation.**
   - `stream-completion.md` item 3 and Decision 006-046 describe a beat at (6 − p)/3, which is not the specified "a third of the time left, less the attempt's duration".
   - With that formula and 6 s left, the beat goes at once whenever the attempt took more than 0.5 s, and its margin is 6 − p, which is 3 s at p = 3 s. Above the 1.5 s threshold the margin is at least two thirds of the time left, which is 4 s.
   - Both are safer than the 2 s stated. Correct the text and the gate's justification.
2. **`sagat_database_timeline()`.**
   - Declare it volatile: an immutable declaration would let the planner fold it into a cached generic plan.
   - If the call fails between two probes (a revoked privilege, or a server entering recovery, where `pg_walfile_name` raises an error), the whole claim statement fails. Delivery then stops, not only marking, for up to one registry heartbeat. Say so, or guard the call.
3. **Failure row, line 646.** "Or lapsed if it cannot": a provisional lease whose hand-back cannot land is withdrawn by the pass (`NOT_ACKNOWLEDGED`); it does not lapse.
4. **`worker_leases_expired_total{mark="acknowledged"}`.** It lumps recovered marks with acknowledged ones, while the acknowledgement counter keeps `recovered` apart. Add `mark="recovered"`, so the reviews after a failover can be attributed.
5. **The withdrawals table has no row for a hand-back of a mark of an earlier timeline.**
   - "Not marked → as Architecture decision 0098 decides" could be read to cover it, and that rule would withdraw it, because `lease_acknowledged_at` is empty.
   - Task 006-050 (line 226) and the 037 clarification make it an ordinary failure. Add the row.
6. **"A sibling process under the same worker name"** (host item 1). Registration replaces the session's token hash (`crates/infra-postgres/src/workers.rs:338-343`), so two processes under one name cannot both hold a session. Drop the clause, or explain the case it means.

## The checks you asked for

- **The timeline after a promotion and after a point-in-time recovery** (PostgreSQL behaviour, from its source):
  - `pg_walfile_name` takes the timeline from the write-ahead log insertion timeline, not from its argument. The end of recovery sets that timeline before any write is allowed.
  - The function raises an error during recovery, so a hot standby's probe fails, which is the right direction.
  - An archive recovery ends on a new timeline, one above any it replayed. So restored rows never carry a timeline equal to the new one.
- **Can the timeline be fooled?**
  - By a same-timeline restore (Minor 3), and by the "earlier" wording (Minor 6).
  - No production code sets `synchronous_commit` (the grep finds it only in `infra/dr/drill.sh`, the performance tests and drafts), so a crash loses nothing.
  - The acknowledgement's 1 s statement timeout can end a commit's wait for a synchronous standby, leaving it committed locally only. The timeline rule covers the failover that would lose it.
  - Stamping a timeline the engine probed earlier, instead of calling the function in SQL, errs in the safe direction: the mark is unprotected, never wrongly counted.
- **A mark of an earlier timeline, path by path:**
  - The acknowledgement answers `ACKNOWLEDGED` (`recovered`) and writes nothing.
  - The pass calls `expire_attempt`.
  - The hand-back is an ordinary failure (Nit 5).
  - A capability call is served and recorded.
  - A heartbeat is not counted as a violation.
  - The gauges go through the macro. `worker_leases_marked` excludes these marks, so a rollback may finish while such leases are still in flight. That is harmless, because they keep today's meaning on the floor too.
- **The hand-back after expiry, against the pass and the cap.** Exactly one of the two commits; the loser is stale and counted apart. At the cap: Minor 5.
- **The cap at the withdrawal.** Major 1 and Minor 5. A `NOT_SENT` withdrawal of a marked lease at the cap writes nothing and leaves the lease to the pass, which is correct only once Major 1 is fixed.
- **The fallback:**
  - It can fail to fall back when it should: Major 2.
  - It can fall back wrongly when a slow server ends attempts at the 6 s bound while heartbeats are answered. That is bounded to 10 minutes and alerted: acceptable.
  - A single first fallback (at most 12 minutes) does not reach the alert's 15 minutes; the second, of 20 minutes, does. That matches the alert's stated intent.
- **The first-heartbeat arithmetic.** Safe; Nit 1.
- **The deadline check.** Minors 1 and 2.
- **The capability record per method.** Resolved. Under the run or path lock, the conditional update cannot find the lease changed after its own fence check, and the race with a hand-back is ordered by that lock.
- **Arithmetic:**
  - 88 statements + 3 × 2 = 94 round trips, and 94 + 8 + 2 = 104. The eight statements are 1 + 1 + 2 + 2 + 1 + 1.
  - The admission threshold leaves max(1, ceil((L − D) / 2)) places to the class `Lease` for reserves of 1, 2 and 3; with a reserve of 0 it is the discovery band.
  - (30 − 6 − 6) / 6 = 3.
  - (6 − 3) × 2 / 3 = 2 is right as arithmetic but describes a different formula (Nit 1).
- **Contradictions between documents:**
  - Line 508 against line 503 and Task 006-050 (Major 1).
  - "Earlier" against "current" and "another" (Minor 6).
  - The table's "Not marked" row against Task 006-050 and 037 (Nit 5).
  - Line 646's "lapsed" against the pass (Nit 3).

## Evidence to ask for once built

Each is a test to add, then its run:

- `cargo test -p sagat-tests --test integration_durable_execution`, with:
  - a marked lease past its expiry with the requirement removed, and the same at the cap, each ended by one pass and not picked again (Major 1);
  - the `duplicate` and `recovered` verdicts with less than one receive bound left (Minor 4);
  - the parity rows for a greater timeline and a null (Minor 6);
  - root and scoped rows for each.
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with a two-minute `UNAVAILABLE` of the call alone (Major 2).
- One run of `infra/dr/drill.sh`, extended with a snapshot restore without archive recovery (Minor 3).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 217-700)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-040 to Decision 006-063, Gap 006-040 to Gap 006-048)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (lines 131-300)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§5, §6.2, §6.4, §6.8, §9)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md (lines 148-166)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/constitution.md (principles I, III, VII, VIII)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-4/ (all four reports)
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round4-to-5.diff
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs (75, 2040-2130, 2314-2543, 2880-2951)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/command.rs (1804-1941)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/maintenance.rs (361-424)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs (5025-5035, 7737-7759)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/workers.rs (313-370)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/scheduler/src/passes.rs (5601-5650)
