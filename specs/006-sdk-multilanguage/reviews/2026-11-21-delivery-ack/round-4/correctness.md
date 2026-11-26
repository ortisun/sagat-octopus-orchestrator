# Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 4

## Verdict

The rework resolves all four Majors of my round-3 report, and the round-3 findings of the other reviewers that fall in my domain. The acknowledgement design itself holds up:

- **Row locking.** It locks rows with `FOR UPDATE OF steps SKIP LOCKED`, decides on what it locked, and answers `RETRY` for a row it could not lock. So it never waits for a row, cannot deadlock on rows, and decides on the latest committed row version.
- **Exactly one winner.** The argument that exactly one of the acknowledgement and a withdrawal commits survives `SKIP LOCKED`.
- **Hand-back reservation.** It is now read from the lease.
- **Fallback.** It keeps the minor.

Three new Majors:

1. The withdrawal count the claim is meant to check is not part of the restored run state; the server build reads it from the journal with a separate query.
2. A provisional lease handed back at or after its expiry, before the pass withdraws it, becomes an ordinary failure that spends the attempt. The host's own give-up rule makes that timing common in the very case the design exists for.
3. The fallback trigger ("three leases given up in a row") fires on transient outages, and the fallback lasts until the host restarts.

Counts: 0 Blockers, 3 Majors, 6 Minors, 6 Nits. Nothing was built or run. Claims about the code come from reading it, at the paths given. Claims about PostgreSQL behaviour are marked as such.

## Status of round-3 findings

### My report

| Round 3 | Status | Reason |
|---|---|---|
| Major 1: verdict from the unlocked pre-read; a repeat answers `NOT_HELD` for an acknowledged lease | **Resolved** | Steps 4 to 6 of "The server's acknowledgement": lock first, decide on the locked rows, then update the winners. A repeat while the first attempt commits skips the row (`RETRY`) and is answered `ACKNOWLEDGED` on the next ask. The barrier case is in Task 006-051. `CANCELLED` is decided on the locked row. The run's stop reason is read through the join, not locked, but that is linearizable: a cancellation that commits after the statement's snapshot is ordered after the acknowledgement, and it then reaches an acknowledged lease by today's path. |
| Major 2: the first heartbeat was scheduled from a configured interval or the wall clock | **Resolved, with a residual** | `lease_time_left_ms` is a duration, and the first beat comes at the earlier of the interval and a third of it, on the monotonic clock, whatever interval is configured. Host and conformance cases now cover a clock 60 s behind and a configured 10 s interval. Residual: the duration is measured before the commit and the answer's transit, and the gate's thresholds do not fit the margin (Minor 1). |
| Major 3: hand-back judged by a session minor that registering again can change | **Resolved** | `lease_hand_back_generation` is written by the claim. The fallback waits for its hand-backs' answers. Task 006-050 and the conformance case "a hand-back held across a re-registration" cover it. Residual: the floor binary still reads the session row (Minor 3). |
| Major 4: below 1.8 the required hand-back spends an attempt | **Resolved (recorded and put to the owner)** | Consequences states it and leaves it to the owner with Architecture decision 0098 (Gap 006-031 on the server branch). Task 006-066 now uses a host of the Architecture decision 0098 SDK build at 1.5, so the case can be observed. |

My round-3 Minors:

| Round-3 Minor | Status |
|---|---|
| 1 (lock order) | Resolved by `SKIP LOCKED` |
| 2 (clock with a holder that only locks) | Resolved: no wait on rows, clock after the lock |
| 3 (give-up while an attempt is in flight) | Resolved |
| 4 (run never clears) | Resolved: the run of three is dropped; the cap is per attempt |
| 5 (Task 006-066 could not fail) | Resolved |
| 6 (override and hand-backs) | Partly resolved: the scenarios "that hand back by design" are still not named (Minor 5) |
| 7 (precondition (a)) | Resolved. The server build now has the minor gate (`HAND_BACK_PROTOCOL_MINOR = 8`), the cap and an expiry check (`crates/application/src/worker/withdrawal.rs:23,190,216`). Task 006-050 owns the re-read `FOR UPDATE`. That expiry check is the root of Major 2. |
| 8 (deferred flush) | Resolved: Task 006-055 itemises the triggers |
| 9 (binding lock held while waiting on a row) | Resolved: no wait on rows |

All five Nits are resolved.

### Other reports, in my domain

| Finding | Status | Reason |
|---|---|---|
| Architecture Major 2 and design Major 2: cap and run of three not reconciled | **Partly resolved** | One bound now: the claim marks only under the cap. But the count's source is named wrongly (Major 1). |
| Architecture Major 6: nothing checks the invariant that lets the run lock go | **Resolved** | Task 006-047 has the static check of the assignments and the barrier with a run transaction loaded before an acknowledgement. `upsert_step` (`crates/infra-postgres/src/execution.rs:7378-7391` of the server branch) computes `lease_token_hash` in SQL from the current row, so "keep (`ELSE` the column)" is the existing pattern. |
| Architecture Major 5: fallback to 1.7 | **Resolved** | `declines_acknowledgement` keeps the minor. The trigger itself is the new Major 3. |
| Architecture Blocker 1: rollback floor in prose only | **Resolved, conditional on precondition (a)** | The registry floor relies on Decision 006-032 (`lease-withdrawal-v1` cannot be removed once a journal holds the event). On the server branch `WithdrawalCause` is still `NotSent` and `NotStarted` only (`crates/domain/src/execution/event.rs:438-443`), so the precondition is open, as stated. |
| Security Major 1: session row decides the hand-back | **Resolved** | As my Major 3. |
| Security Major 2: hand-back after a capability was served | **Resolved in text** | Residual: the record is not specified as a fence (Minor 4). |
| Design Major 4: the statement did not do what its prose said | **Resolved** | By the lock, then decide, then update order. |

## Blocker

None.

## Major

**Major 1. The claim cannot read the attempt's withdrawal count "from the run state the claim already restored": that state does not hold it. The server build reads it from the journal with a separate statement.** (Confirmed in code.)

- **Where:**
  - Decision 2 of the ADR, line 58.
  - `stream-completion.md` lines 389-390: "the count the domain's withdrawal command reads, from the run state the claim already restored".
  - Decision 006-052, line 505.
  - Task 006-047, and the unchanged `LEASE` line.
- **What the code does:**
  - `withdraw_lease` takes the count as a parameter "because the state is restored from the projection, which does not keep it; it is read in the same transaction, under the run lock" (`crates/domain/src/execution/command.rs:1838-1857`, server branch).
  - Every caller runs `lease_withdrawals`, a `count(*)` over `run_events` for `event_type = 'step.lease_withdrawn' AND attempt = $4` (`crates/infra-postgres/src/execution.rs:7715-7733`; callers at `withdrawal.rs:229,262,356,429`).
- **Why it matters:**
  - The design's guarantee "a marked lease's withdrawal is always within the cap" holds only if the claim's count is this same journal count.
  - As written, an implementer either:
    - adds a statement per claimed lease, which breaks the `LEASE` budget line that Tasks 006-047 and 006-055 say stays unchanged; or
    - takes the count from a source that does not exist (for example, a counter in the projection that the journal does not derive).
  - If the two counts ever differ, `withdraw_lease` returns `WithdrawalsSpent`, the outcome is `Capped`, and "a lease the server did not send is left to lapse" (Decision 006-031). For a provisional lease that is a lapse into consultation or review, against decision 5's "MUST be withdrawn".
  - **Example:** four `NOT_SENT` withdrawals of attempt 1, then a claim whose count source missed one of them. The lease is marked and lost on the wire. The pass is refused at its deadline, and the step goes to review for work no host started.
- **Why the plan misses it:** the matrix of Task 006-047 ("three and four withdrawals of the attempt") checks the claim's verdict, not that the claim's count and the withdrawal command's count are the same query. The parity test compares the pure function with the SQL macro, not the inputs each is fed.
- **Smallest fix:**
  - Name the source in decision 2, `stream-completion.md` and Decision 006-052: the same journal count over the instance index. Fold it into each of the three lease statements as a scalar subquery inside the marking `CASE`, so it adds no top-level statement and no written row.
  - Return the marking verdict with `RETURNING`, so that the flag on the lease and the skip reason `attempt_capped` come from what the statement wrote.
  - State that the `LEASE` line keeps its statement and row counts but not its work.
  - Add a test in `integration_durable_execution`, root and scoped: a lease marked at count 3 is withdrawn as the fourth, and the pass is never `Capped` for a marked lease.

**Major 2. A provisional lease handed back at or after its expiry, before the pass withdraws it, becomes an ordinary failure that spends the attempt. The host's give-up rule regularly puts the hand-back there in the lossy-network case.** (Confirmed in code for the hand-back branch. That the ordinary failure is then accepted is plausible: `verify` checks only the token.)

- **Where:**
  - `stream-completion.md`, the withdrawals table (line 451: "a hand-back under the reserved code (the rule above)").
  - The failure-table row "The acknowledgement never leaves the host" (line 555: "the hand-back withdraws it (`NOT_STARTED`)").
  - Host item 4 (line 515).
  - `sdks/SPEC.md` §6.2.
  - The 037 clarification: "neither is such a lease handed back with `TASK_NOT_STARTED` … is an attempt".
- **What the code does:**
  - `root_hand_back` and `path_hand_back` require `lease_unexpired`, which checks `lease_expires_at > clock_timestamp()` (`withdrawal.rs:212-216,240-249`; `crates/infra-postgres/src/execution.rs:4846-4856`).
  - Otherwise the result is `Err(FenceMissed)`, and the report falls through to `apply_scoped_or_root_failure` (`crates/application/src/worker.rs:2477-2527`).
  - `verify` checks only the token (`worker.rs:2935-2951`; `lease_matches_instance` at `execution.rs:4999-5019` has no expiry).
- **Scenario** (synchronized clocks, time-to-live 30 s, acknowledgements timing out with no answer):
  1. The host's give-up instant is its expiry minus 6 s. It never gives up while an attempt is in flight and waits for it up to 6 s.
  2. Attempts run back to back (6 s each plus a backoff of at most 5 s), so one is usually in flight at the give-up instant. It ends at up to the expiry itself.
  3. The hand-back is then sent and lands after the expiry.
  4. If the pass has not run yet, the outcome is an ordinary retryable failure, and the attempt is spent. With `maxAttempts = 1` it is terminal: the `onError` route or the compensation of earlier steps runs, for work that never started. Below the cap and before the pass, the design promised a withdrawal.
- **Same path:** `CANCELLED` is answered with no margin (step 5 requires one receive bound only for a win), so its hand-back can also land after the expiry.
- **Why the plan misses it:** no task has a hand-back of a provisional lease after its expiry and before the pass. Task 006-051 has "an acknowledgement after a deadline and before the pass", not a hand-back.
- **Smallest fix:**
  - In the pure function, a `TASK_NOT_STARTED` on a lease whose mark counts and that is not acknowledged is withdrawn whether or not its deadlines have passed. The pass would withdraw it anyway, and nothing ran.
  - Add this as request (6) of Decision 006-059: relax `lease_unexpired` for a counting mark.
  - Add a barrier test, root and scoped, with the pass held: a hand-back 1 ms after the expiry is withdrawn, and the attempt is not spent. Add the same for a hand-back after `CANCELLED`.
  - Alternatively, stop the host from starting an attempt in the last receive bound before its give-up. That costs one attempt of the three that fit: (30 − 6 − 12) / 6 = 2.

**Major 3. The fallback trigger "three leases in a row given up with no `ACKNOWLEDGED` between them" fires on transient outages, and the fallback lasts until the host restarts. One database failover or one shedding episode turns delivery acknowledgement off on every affected host, silently.** (Confirmed in the documents; plausible in operation.)

- **Where:** `sdks/SPEC.md` §6.2 (line 373, "until it restarts"); `stream-completion.md` host item 5 (lines 519-521); Decision 006-048 ("The fallback lasts until the host restarts").
- **Scenario:**
  - A PostgreSQL failover of 30 s. The store is unavailable, so every acknowledgement is answered retryable.
  - Alternatively, the class `Acknowledge` is shed with a retry hint of up to 60 s (§6.2 caps a server-derived wait at 60 s, above the 18 to 24 s window).
  - A host holding three or more flagged leases repeats as specified, gives each up at its window, and hands them back. Their withdrawals count toward the per-attempt cap.
  - It then registers again with `declines_acknowledgement = true`, for its whole remaining life.
  - Every host that saw the outage does the same. After recovery the fleet runs unprotected (Gap 006-001 is back) until each process restarts.
  - Nothing alerts. `worker_sessions{protocol, declines}` is a gauge without an alert, and the SDK counter `acknowledgementFallbacks` is local to the host.
- **Why the plan misses it:**
  - The conformance case "three different steps whose acknowledgements fail until they are given up (the fallback after the third)" asserts the trigger, not its limits.
  - No case checks that an outage the host would retry does not turn the protection off for good.
- **Smallest fix:**
  - Count toward the fallback only leases given up while every attempt ended with no answer at all, and while other calls to the same server succeeded in the same period. That is the signal of a path that swallows `AcknowledgeTasks`.
  - Never count leases whose attempts met `UNAVAILABLE`, `RESOURCE_EXHAUSTED` or a store-unavailable status.
  - Make the decline expire: register again without it after a bounded period (for example 10 minutes), logged once.
  - Add an alert on live sessions that decline.
  - Add conformance cases: a 30 s `UNAVAILABLE` outage of the call with three flagged leases in flight does not fall back; a fallback probes again after its period.

## Minor

**Minor 1. The margin argument for the first heartbeat counts time the margin does not have, and the gate's thresholds do not fit inside it.** (Confirmed by the arithmetic in the documents.)

- **Where:** decision 6; `stream-completion.md` "Why the deadline is the lease's own", item 3 (line 483-484: "comes at most 2 s after the answer and has the rest of the margin to commit"), item 6, and the gate of decision 13.
- **The arithmetic:**
  - The acknowledgement wins only with at least 6 s left at the clock sample. After the sample come the update, the commit (with its deferred per-row flush) and the answer's transit, together call them `p`.
  - `lease_time_left_ms` is measured at the update, before most of `p`. So the host schedules its first beat at about 2 s after receipt, while only about 6 − `p` s is really left.
  - The lease is safe only if `p` + 2 + `h` < 6, where `h` is the heartbeat's own round trip and commit.
  - The gate allows a call of up to 3 s at the 99th percentile and a heartbeat of up to 2 s at the 99th percentile: 3 + 2 + 2 = 7 > 6.
  - Outcome at that tail: an acknowledged lease whose first heartbeat commits after the expiry. That is an unknown outcome, consultation or review, for a handler that is running.
- **Duplicate verdict:** "already acknowledged with the same token" is answered `ACKNOWLEDGED` with no margin at all. Its `lease_time_left_ms` can be 0 or negative (a lease past its expiry and not yet reaped), and §6.4 does not say what the host does with that.
- **Fix:**
  - The host schedules the first beat at a third of `lease_time_left_ms` minus the elapsed time of that attempt on its monotonic clock. That is conservative and needs no comparison of clocks.
  - With it, the condition becomes `h` < (6 − `p`) × 2/3, which the gate's 2 s meets at `p` = 3 s.
  - Clamp the field at 0 and say that the host beats at once when it is under 1.5 s.
  - Add a host unit test with a slow answer.

**Minor 2. Rollback through `SAGAT_LEASE_WITHDRAWAL=off` changes the meaning of the marks in flight, which contradicts step 2 of the rollback and decision 10.** (Confirmed in the documents and in `withdrawal.rs:151-160`.)

- **Where:** `stream-completion.md` "Rollback" step 1 (line 538-539: "or set `SAGAT_LEASE_WITHDRAWAL=off` on every engine"); step 2 ("every marked lease ends … with its meaning unchanged"); decision 10 ("a mark already written keeps its meaning until its lease ends").
- **What happens:**
  - With the switch off on every engine, `gate_outcome` returns `Disabled`.
  - A provisional lease lost on the wire lapses into review, instead of being withdrawn.
  - Its hand-back is an ordinary failure that spends the attempt.
- **Fix:** rollback stops marking only by removing the requirement (the switch stays on until `worker_leases_marked` is 0). Alternatively, state that the switch ends the protection of marks in flight, and test it in the rollback case of Task 006-066.

**Minor 3. The rollback floor and a mixed fleet decide hand-backs from the session row, and Decision 006-059 (3) asks the floor build for something it cannot do.** (Confirmed in code; the occurrence during a rolling deploy is plausible.)

- **Where:** Decision 006-059 (3); precondition (a); `HAND_BACK_PROTOCOL_MINOR` and `lease_holder_minor`, which joins `worker_sessions.accepted_protocol_minor` (`crates/infra-postgres/src/execution.rs:7737-7759`).
- **What is wrong:**
  - `lease_hand_back_generation` is created by this build's migration (Task 006-044). So "the reservation of decision 5 is read from `lease_hand_back_generation` once this build writes it", asked of the Architecture decision 0098 build before it merges, cannot be done on that branch. Task 006-050 does it here.
  - The floor binary therefore always reads the session row.
  - From Task 006-057, 1.8 sessions exist while floor engines still run (1.8 is offered "on the proofs of its build alone", before any activation). The same unflagged hand-back is withdrawn on a floor replica and spends the attempt on a new replica (no `lease_hand_back_generation` if a floor engine claimed it). After a rollback, round-3 Major 3 returns on the floor.
- **Fix:** move request (3) out of Decision 006-059 into Task 006-050. State the per-replica difference in Consequences and in "Rollback", as a known outcome either way. Add the mixed-fleet case to `provisional-lease-cutover`.

**Minor 4. "A served capability records `lease_capability_generation` in the transaction it already commits" is not a fence as written.** (Plausible.)

- **Where:** decision 8; Decision 006-050; Task 006-052.
- **What is wrong:**
  - For the record to make "a hand-back after a capability was served" decidable, it must commit before anything is handed over, conditionally on the lease. The serve must be refused when the update matches no row.
  - Otherwise a hand-back that holds the row first withdraws the lease while the capability is still served. That is a free re-run for a client that read a secret or a blob.
  - It is unclear which transaction "already commits" for each method. For `ReadBlob` it can only be the audit of `audit_blob_read`, which runs after `service.read` (`crates/api-grpc/src/service.rs:1356-1379`, server branch), and `WriteBlob` and `AppendStepLogs` differ again.
  - Written on every call, it adds a row lock and the per-row quota trigger to every log append and blob read.
- **Fix:** one conditional `UPDATE … WHERE` token and generation match `AND lease_capability_generation IS DISTINCT FROM generation`, before the serve. Zero rows on a lease that is no longer held means refuse. Name the transaction per method. Add a barrier test: a capability call against a hand-back, both orders.

**Minor 5. The conformance plan has a case with no mechanism, and leaves its exclusions unnamed.** (Confirmed in the documents.)

- **Busy row:** "a busy row (`RETRY`, then `ACKNOWLEDGED`)" (Task 006-058, line 267) needs a database session that holds the step row. The harness actions listed are `hold` and `dropFrame`, both at the transport. Either add an action that holds the row `FOR UPDATE` until released, or keep `RETRY` at the server level only (Task 006-049 already has it).
- **Exclusions:** "the scenarios that hand back by design are named" in Decision 006-058, but neither Task 006-058 nor Task 006-065 names them. `worker-shutdown`, which is pinned with a 300 ms stop, is likely one of them.
- **Missing cases:** the cases of Majors 2 and 3.

**Minor 6. What a declining host does with a flagged lease is not specified.** (Plausible.)

- `poll_bounded` reads the session at the start of each claim (`crates/application/src/worker.rs:992`). A claim in progress when the host registers again can still flag a lease for it.
- §6.2 rule 5 covers leases "waiting for its acknowledgement when the host … falls back", not ones received after.
- **Fix:** a host that declines hands back any flagged lease it receives, and the conformance `UNIMPLEMENTED` case asserts it.

## Nit

1. **Statement count.** "About eight statements" (1 + 1 + 2 + 2 + 1 + 1) does not say where `clock_timestamp()` is sampled "after that statement". If it is a statement of its own, the count is 9, giving 106 against 94 round trips, about 11%. Say it is a column of `lock_matched` (which, with `SKIP LOCKED`, never waits between that sample and the lock), or count it.
2. **"1.8 or later".** Decision 2 (line 55) and `stream-completion.md` line 386 say "accepted at 1.8", while decision 1 and the hand-back say "1.8 or later". Minors are cumulative; write "1.8 or later" everywhere.
3. **`RETRY` and the host's counters.** Say whether `RETRY` advances the backoff exponent of §5, and that it is not "a call refused" for the fallback.
4. **Stopping.** §6.8 does not say whether the 6 s wait for a verdict counts inside `stop(timeout)` or in addition to it.
5. **`lock_matched` selection.** It should select by primary key only, so that "absent" means "skipped". With the token in its `WHERE`, a row changed after the snapshot is excluded by PostgreSQL's recheck of a concurrently updated row and also answered `RETRY`. That is harmless (the next ask answers `NOT_HELD`) but miscounted.
6. **Skip reasons and re-registration.** `worker_lease_marks_skipped_total{reason}` needs a precedence when several reasons hold. Task 006-048 should test that a registration without the field, after a declining one, clears `declines_acknowledgement` (the upsert at `crates/infra-postgres/src/workers.rs:338-343` must set it from `EXCLUDED`).

## The checks you asked for

- **`SKIP LOCKED` and the `RETRY` loop.**
  - Every holder of a step row I found in the code is short and run-scoped: a run transaction that touches the step (only changed nodes are in `touched`, `execution.rs:5353`), a withdrawal's re-read, a revocation (ordered first by the binding lock). The triggers on `steps` that fire on `status` do not fire for the acknowledgement's update. The quota flush enqueues nothing because `child_input` and `correlation_key` do not change. So `RETRY` cannot starve in practice.
  - A lease kept in `RETRY` until its give-up is handed back, and that is decided under the row lock. If the holder was a withdrawal, the next ask answers `NOT_HELD` and the host drops the lease. A hand-back sent anyway is answered stale and counted apart.
  - The pass holds the row only after the deadline, by which time the host has already given up.
  - The real defect at the edge of this loop is Major 2.
- **Exactness.**
  - Holds. In PostgreSQL's read-committed mode a locking read of a row updated and committed after the snapshot returns the latest version.
  - The acknowledgement writes only rows it holds, and every withdrawal of a counting mark re-reads under the same row lock.
  - The unlocked join to the run's stop reason only orders a later cancellation after the acknowledgement.
- **The clock.**
  - Sampling after `lock_matched` is sound, because the statement never waits for a row.
  - The withdrawal's re-read can carry an earlier clock after a wait on a holder that only locked. That is conservative: it withdraws later, never wrongly.
  - The remaining clock defect is the time-left accounting (Minor 1).
- **The mark conditions.**
  - The cap is the right one, but its count is not where the design says (Major 1).
  - The path claim has the same problem (`PathExecutionState` holds no count either).
- **The new columns against every writer.**
  - The three lease statements, the acknowledgement, the withdrawals, the revocation (which writes `lease_expires_at` only), reset (new rows) and registration are all covered by the allow-list.
  - A mark left by a binary that does not know the columns never counts, because withdrawals keep the generation, the step is `READY`, and every claim moves the generation.
  - Open: the capability writer (Minor 4).
- **The fallback and stop rules.** Major 3, Minor 6, Nits 3 and 4.
- **The conformance plan.** Minor 5. The "three with a 3 s time-to-live" are `protocol-negotiation`, `worker-stream-faults` and `worker-transport-faults`; `protocol-negotiation` is excluded on its own.
- **Arithmetic.** All correct as stated:
  - 38 + 50 = 88 statements and 3 transactions, so 94 round trips with `BEGIN` and `COMMIT`.
  - 96 + 8 = 104 round trips.
  - 94 / 104 = 0.904, so about 10% fewer transitions a second.
  - (30 − 6 − 6) / 6 = 3; 3 × 6 = 18.
  - The only open count is the clock sample (Nit 1).
- **Contradictions between documents.**
  - Decision 2, `stream-completion.md` and Decision 006-052 against `command.rs:1841` (Major 1).
  - The failure table and the 037 clarification against the hand-back's expiry check (Major 2).
  - Rollback step 1 against step 2 and decision 10 (Minor 2).
  - Decision 006-059 (3) against Task 006-044 and Task 006-050 (Minor 3).

## Evidence to ask for once built

Each is a test to add, then its run:

- `cargo test -p sagat-tests --test integration_durable_execution`, with:
  - the claim count against the withdrawal count (Major 1);
  - a hand-back 1 ms past the expiry, with the pass held (Major 2);
  - root and scoped rows for each.
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with the outage case that must not fall back (Major 3).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/round-3/ (all four reports)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20261024000000_durable_quotas.up.sql (190-372, 524-527)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/integration/claim-transition-efficiency/work_budget.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/ (lost-responses, worker-transport-faults, the `leaseTtlSeconds` and `timeoutMs` values)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/specs/006-sdk-multilanguage/research.md (Decision 006-030 to Decision 006-039, Gap 006-030, Gap 006-031)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/command.rs (1831-1941)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/event.rs (438-452)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs (929-1049, 2044-2122, 2400-2544, 2935-2951)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs (4827-4867, 4999-5034, 5320-5362, 7222-7391, 7713-7770)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/pool.rs (54-129)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/ (the triggers and foreign keys on `steps`)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/service.rs (1326-1385)
