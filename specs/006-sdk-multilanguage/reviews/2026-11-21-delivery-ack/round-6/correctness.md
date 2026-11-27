# Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 6

## Verdict

The round-6 text resolves my round-5 Major 1:

- In the lost-lease pass, every outcome other than a withdrawal now ends the attempt with `expire_attempt` in the same transaction.
- The pass is stated to have no binding checks.
- A test asserts that the next pass does not pick the lease again.

It partly resolves my round-5 Major 2:

- The server's statuses now count toward the fallback when other calls are answered.
- `SagatAcknowledgementsFailing` now alerts on this call's failures.
- But the new discriminator ("another call to the same server was answered during each") has nothing to observe exactly when the failure fills the host. A host whose slots are all held by flagged leases waiting for their acknowledgement makes no other call, so it never falls back. Its leases cycle through hand-backs until the cap spends their attempts (new Major 1).

The other round-5 Majors in my domain are resolved:

- **Security Major 1**: the hand-back is honoured only while `delivery-acknowledgement-v1` is recorded.
- **Design Major 1**: `end-lease-marks` runs after every restore, and the topologies are named.
- **Architecture Major 1**: `synchronous_commit` is raised, and no lease is marked while `fsync` is off.

The changes bring in Minors:

- **`CANCELLED` on an acknowledged lease.** For a step with declared secrets, the hand-back that follows `CANCELLED` is refused by the server build's report fence when a stop is asked or the lease has just expired. The lease then lapses into a review (confirmed in code).
- **Rollback drain.** The drain now gates the removal of the requirement, and it waits for acknowledged leases, which can run for hours.
- **Timeline function.** A `LANGUAGE sql` function cannot return null when the runtime role lacks the right on the write-ahead log functions. Since every 1.8 claim now stamps the timeline, every 1.8 claim would then fail.
- **Contradictions.** Several passages were not updated.

**Counts:** 0 Blockers, 1 Major, 9 Minors, 10 Nits.

Nothing was built or run. Claims about the server build come from reading `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` at the paths given. Claims about PostgreSQL behaviour come from its source as I know it and are marked as such; Task 006-044 must check them on the pinned image.

## Status of round-5 findings

### My report

| Round 5 | Status | Reason |
|---|---|---|
| Major 1: pass outcomes that write nothing for an expired lease | **Resolved** | Covered in decision 5, in the withdrawals table (rows "at the cap" and "a requirement not recorded"), in the failure rows, in Task 006-050 ("ended by one pass … and not picked again by the next") and in the new acceptance row "The pass ends what it does not withdraw". The pass re-checks the loss under the run lock through `apply_if` with `Condition::LeaseLost` (`crates/scheduler/src/passes.rs:5629-5655`), so a lease renewed or replaced since the candidate read is left alone, correctly. Residues: one stale paragraph (Minor 8) and the acceptance wording "lapses" (Nit 4). |
| Major 2: a failure of `AcknowledgeTasks` alone never ends | **Partly resolved** | The server's statuses count when other calls are answered, and `SagatAcknowledgementsFailing` plus the `NOT_STARTED` ratio alert on it. Still open: the discriminator is starved when flagged leases fill the host (Major 1 below); any `ACKNOWLEDGED` resets the count, so a failure that depends on call size never falls back (Minor 6); there is no breaker on the server's side and no split of a large call. |
| Minor 1: the gate's duration criterion cannot fail | **Partly resolved** | Successful calls are measured, and fewer than 0.1% may end at the deadline. Calls ended by the 1 s statement or lock timeout are counted in neither figure (Minor 3). |
| Minor 2: the deadline check's claim | **Resolved** | The deadline runs from arrival, pool acquisition included. The answer is `UNAVAILABLE` (`ACKNOWLEDGEMENT_DEADLINE`). The text no longer claims that nothing commits after the host gives up. |
| Minor 3: restores that keep the timeline | **Resolved** | `end-lease-marks` runs after every restore. The topologies are named in decision 12, in Gap 006-048 and in the runbook. The drill covers it. |
| Minor 4: no margin on `duplicate` and `recovered` | **Resolved** | Residue: the `CANCELLED` path that replaces it (Minor 1). |
| Minor 5: two outcomes at the cap | **Resolved** | The asymmetry is now a stated choice, with its reason, and both orders are tested. |
| Minor 6: "earlier" against "other" | **Resolved** | "Another timeline, or none" is used throughout, and the parity test has rows for a lower, a higher and a null timeline. |
| Nit 1: the first-heartbeat derivation | **Resolved** | New residue: "more in every other case" (Minor 5). |
| Nit 2: the timeline function | **Partly resolved** | `VOLATILE` is declared. "Null, never an error" cannot be had from `LANGUAGE sql` (Minor 4). |
| Nits 3 to 6 | **Resolved** | |

### Other round-5 reports, in my domain

| Finding | Status | Reason |
|---|---|---|
| Security Major 1: a floor process serves a capability without recording it, and this build withdraws the hand-back | **Resolved** | Decision 006-064 makes the hand-back, and every withdrawal of a marked lease, read `delivery-acknowledgement-v1` under its share lock. The removal waits for them, and a floor process is admitted only after it. Residues: the test is described with an impossible actor (Minor 9), and a removal followed by a re-activation can give old leases protection again (Nit 5). |
| Design Major 1: restores that keep the timeline | **Resolved** | As my Minor 3. Gap 006-048 states what remains (a skipped step, and a logical-replication cut-over). |
| Architecture Major 1: a crash under `synchronous_commit = off` or `fsync = off` | **Resolved** | Because the write-ahead log is sequential, a committed acknowledgement flushed locally also makes every earlier claim and withdrawal durable. Residues: `fsync` can be changed by a reload between two probes (Nit 8), and the statement timeout applies to `COMMIT` (Nit 7). |

## Blocker

None.

## Major

**Major 1. The third trigger of the fallback depends on other traffic, and the failure suppresses that traffic. A host whose slots are all held by flagged leases waiting for their acknowledgement makes no other call to the server, so it never falls back. Round-5 Major 2 then remains in full.** (Confirmed in the documents.)

- **Where:**
  - `sdks/SPEC.md` §6.2, lines 373-377: "three calls in a row given up … while another call to the same server was answered during each".
  - `stream-completion.md` host item 5, lines 621-627: "(a receive, a heartbeat, a report)".
- **Why there is no other call:**
  - The loop polls only `if free > 0` (`sdks/SPEC.md` lines 344-345).
  - §10 (line 785) says "A host MUST request only available execution slots".
  - A flagged lease waiting for its verdict holds a slot: `NOT_HELD` is the verdict that says "free its slot" (line 368).
  - So once one frame fills the credit, the host has no receive, no heartbeat (nothing runs) and no report in flight during that acknowledgement call.
- **Scenario:**
  1. The defaults apply: `max_inflight = 10`, 30 s time-to-live, ready work queued, so the first frame carries 10 flagged leases.
  2. `AcknowledgeTasks` alone fails. This is any of round 5's cases: the class `Acknowledge` shed while `Lease` is admitted, a gateway answering 503 for the method, or a 64-lease call past its deadline.
  3. Every attempt fails until the give-up at about 24 s. No other call was made during the call, so it does not count.
  4. The 10 leases are handed back (withdrawn, `NOT_STARTED`), the slots free, and the next frame fills them again. That frame's call starts after the hand-backs have been answered, or overlaps them by chance, so whether it counts depends on timing.
  5. A lease that a saturated host keeps receiving reaches the cap of four withdrawals per attempt, and its next hand-back spends the attempt. With `maxAttempts = 1` that is a terminal failure for work that never started, which is exactly round 5's failure.
- **Partial outages and shedding:**
  - **Wrong fallback in a partial outage is unlikely.** With per-call balancing, a repeated attempt reaches a healthy replica. With per-connection balancing, the stream and heartbeats fail together with the acknowledgement.
  - **A fallback during shedding of the class `Acknowledge` is acceptable now that the decline is bounded and alerted**, because it removes the call's load.
  - **The window is short.** Three concurrent calls can all be given up within one give-up period (about 24 s), so a 30 s shed of this class alone flips a lightly loaded host for 8 to 12 minutes, and the fleet at once. The conformance case speaks of "two minutes", but the rule sets no minimum.
- **Is "given up" well defined?** For one call, yes ("every lease given up with no verdict"). Three things are not:
  - "In a row" for calls that overlap: by start or by give-up?
  - Whether the re-ask of the `RETRY` leases is a call of its own.
  - Whether "answered" means `OK` (or a frame, or an empty window). The outage exclusion only works if a call that returned `UNAVAILABLE` is not "answered".
- **Why the tests miss it:** Task 006-058's "two minutes of `UNAVAILABLE` for this call alone while the stream is answered" presumes a stream with free credit. Nothing runs it with the credit filled by the failing frame.
- **Smallest fix:**
  - Count a given-up call when another call to the same server was answered `OK` during it, **or when its own hand-backs are answered**. A host always makes those hand-backs, and an outage of the database fails them too, so the exclusion of an outage still holds.
  - Define "in a row" by the order of give-up.
  - Make the `RETRY` re-ask part of its call.
  - Require at least 60 s between the first and the third counted give-up.
  - Add the conformance case with `max_inflight` equal to the frame size.
  - Optional, and more robust: an engine-side breaker that stops marking (skip reason `acknowledgement_failing`) while its own share of failed acknowledgement calls is above a threshold. It does not depend on what the host can observe.

## Minor

**Minor 1. `CANCELLED` on an acknowledged lease (`stop_asked` or `too_late`) is meant to become an ordinary failure. For a step with declared secrets, and for a hand-back that lands after the expiry, the server build's report fence refuses it, and the lease lapses into a review.** (Confirmed in code.)

- **Where:**
  - `stream-completion.md` step 5, lines 505-510.
  - Decision 7 of the decision record.
  - Decision 006-051.
  - Decision 006-059 (1), which relaxes `cancel_requested` and the stop reason only "on a `LEASED` lease that is not acknowledged", and lifts the expiry only "for a lease whose mark counts" and that is not acknowledged.
- **Code:**
  - `WorkerService::fail` calls `checked_secret_report` with `cancellation_acknowledgement = error_code == "CANCELLED" && !retryable` (`crates/application/src/worker.rs:2448-2456`).
  - For a step with declared secrets and a vault, that runs `report_authority_fence` (`crates/application/src/worker/secrets.rs:278`, `287`).
  - The fence refuses `cancel_requested`, a run stop reason and an expired lease (`worker.rs:2917-2930`) with `StaleAttempt`.
  - A hand-back that meets `STALE_ATTEMPT` "ends … without an error" (`sdks/SPEC.md` line 417).
  - The pass then runs `expire_attempt`. For a declared `Activity` that is an unknown outcome, so a review "whether or not the run asked it to stop" (`crates/domain/src/execution/command.rs:926-931` and the comment there).
- **Scenario:**
  1. An `Activity` step declares secrets.
  2. Its acknowledgement commits and the answer is lost.
  3. The run is cancelled.
  4. The host repeats; the server answers `CANCELLED` (`stop_asked`).
  5. The host hands back with `TASK_NOT_STARTED`. It is refused as stale, and the lease lapses into a review.

  In round 5 the repeat answered `ACKNOWLEDGED`. The handler started, its first heartbeat carried the stop, and its `CANCELLED` report settled the step as cancelled. So round 6 turns that case into a review. The `too_late` variant behaves the same whenever the hand-back lands after the expiry, and the reason the answer was lost (a slow path) makes that likely.
- **Fix:**
  - Extend request (1) of Decision 006-059 and the lifted expiry to a `TASK_NOT_STARTED` on a `LEASED` lease that is acknowledged, or whose mark is of another timeline, until the pass has ended it. It stays an ordinary failure, and under `cancel_requested` `end_attempt` already turns it into `StepCancelled` (`command.rs:926`).
  - Test the hand-back after `CANCELLED` (`stop_asked`) and after `CANCELLED` (`too_late`, 1 ms past the expiry) on an acknowledged lease with declared secrets, root and scoped.

**Minor 2. The rollback's drain, which now has to finish before the requirement is removed, waits for acknowledged leases. Those are renewed by heartbeats and can run for hours. The `down` migration has the same wait.** (Confirmed in the documents.)

- **Where:** Rollback step 2, line 651 ("`worker_leases_marked` is 0 … that takes the longest earlier deadline in flight plus the period of the lost-lease pass"); step 4, line 663.
- **What is wrong:**
  - A mark counts while the step is `LEASED` or `RUNNING` with its generation, acknowledged or not.
  - The floor binary keeps the columns, because its statements do not know them.
  - So a long `RUNNING` activity keeps `worker_leases_marked` above 0 for as long as it runs, past any "earlier deadline".
  - Yet nothing about an acknowledged mark needs this build or the requirement. It is an attempt; its hand-back and its expiry have today's meaning on the floor too.
- **Consequence:** in an incident, an operator cannot wait hours, so they remove the requirement first. That ends the protection of the provisional leases, which is the order the text warns against.
- **Fix:** drain on `worker_leases_provisional` reaching 0 (a provisional lease is never renewed, so this takes at most the longest time-to-live plus the pass), and have the `down` refuse only while a provisional mark counts. Correct the duration sentence.

**Minor 3. The gate counts only the deadline among failed calls. A call ended by the 1 s statement or lock timeout is in neither figure, so the duration criterion still passes by construction.** (Confirmed in the documents.)

- **Where:** decision 13; step 6 of "Why the deadline is the lease's own"; Decision 006-057; Task 006-056.
- **What is wrong:**
  - A slow update with its triggers reaches the 1 s statement timeout well before the 3 s deadline, and answers `UNAVAILABLE` without being "ended by the deadline".
  - Successful calls are at most 3 s plus the commit by definition.
- **Fix:** bound the share of calls not answered `OK` (deadline, lock or statement timeout, store unavailable; `RETRY` is `OK`) below 0.1%, and report the 99th percentile over every call, measured to its end.

**Minor 4. `sagat_database_timeline()` is `LANGUAGE sql` and must "return null, never an error" when the right on the write-ahead log functions is missing. A SQL function cannot do that. Because every 1.8 claim now stamps the timeline, such a role would make every 1.8 claim fail, not only marking.** (Plausible, from PostgreSQL behaviour.)

- **Where:** `stream-completion.md` line 418; Task 006-044 line 172; Decision 006-062.
- **Why** (PostgreSQL behaviour):
  - The `EXECUTE` right of a called function is checked when the executor initialises the expression (`ExecInitFunc`), for every branch of a `CASE`, not when the branch is evaluated.
  - A simple `VOLATILE` SQL function may also be inlined into the claim statement.
  - A guard such as `CASE WHEN has_function_privilege(...) THEN pg_walfile_name(...) END` therefore still raises "permission denied" in the claim. (The recovery guard does work, because `pg_walfile_name` raises its recovery error only when evaluated.)
  - The design stamps `lease_claim_timeline` for every claim of a 1.8 session, marked or not, so the engine's probe and its `no_timeline` skip do not shield the claim.
- **Fix:** make it `LANGUAGE plpgsql`. Its `IF NOT has_function_privilege(...) OR pg_is_in_recovery() THEN RETURN NULL` plans the inner statement only when it is reached, and the function is never inlined. Keep the Task 006-044 test with a role without the right. It would have caught this.

**Minor 5. "And more in every other case" (line 581) is false: `p` is not bounded by the call's 3 s.** (Confirmed by the arithmetic.)

- **The arithmetic:**
  - The derivation itself holds: the beat leaves within (T − p)/3 of the answer and has at least 2(T − p)/3 left.
  - But `p` runs from the clock sample to receipt, and includes the commit and the answer's transit.
  - The text puts the commit outside the bound (line 479), and nothing bounds the transit.
  - With T = 6 s and p = 4.5 s the beat has 1 s, under the gate's 2 s heartbeat bound. With p = 5.5 s it has 0.33 s.
- **Consequence:** an acknowledged lease that expires while its handler starts is an unknown outcome, a review for an `Activity`. That is the outcome the margin exists to prevent.
- **Fix:** the host compares `lease_time_left_ms` minus its attempt's duration with one heartbeat bound (2 s). Below it, the host treats `ACKNOWLEDGED` as `CANCELLED` (`too_late`) and hands the lease back: an ordinary failure, not a review. Correct the sentence.

**Minor 6. Any `ACKNOWLEDGED` resets the fallback count, so a failure that depends on call size never ends.** (Plausible.)

- **Scenario:** a 64-lease call whose update and triggers pass the statement timeout, while the single-lease calls of the same host succeed. Every large frame's leases are handed back and claimed again until the cap spends their attempts. The small successes keep the count at 0.
- **What changed and what did not:** `SagatAcknowledgementsFailing` now detects this (more than 5% of calls not `OK`). Nothing ends it automatically.
- **Fix:** on `ACKNOWLEDGEMENT_DEADLINE` or a timeout, the host splits the leases of that call in two for its next attempt (they are independent rows). Reset the count only with an `ACKNOWLEDGED` for a call at least as large as the failing one. Add a server test: a 64-lease call held past its statement timeout is split and then acknowledged.

**Minor 7. "The smaller of 1 s and the time left" can round to 0, and 0 disables both timeouts. The discarded connection's backend then waits without a bound, holding its locks.** (Plausible, from PostgreSQL semantics.)

- **Where:** step 1, line 481.
- **Why:**
  - `lock_timeout` and `statement_timeout` are integers in milliseconds: a value under 0.5 ms rounds to 0, and 0 means no timeout.
  - The deadline answers the host by discarding the connection.
  - A backend that is waiting for a lock does not notice the client has gone (`client_connection_check_interval` is 0 by default). While it waits it keeps the binding's and the namespace's shared locks and any row locks it already holds.
- **Fix:**
  - Do not begin the transaction when less than a floor is left (for example 50 ms), and answer `UNAVAILABLE` (`ACKNOWLEDGEMENT_DEADLINE`).
  - Never set a timeout below 1 ms.
  - When the deadline falls between statements, send `ROLLBACK` on the live connection rather than discarding it. Discarding reconnects under exactly the load that caused the deadline, and can briefly hold more backends than the pool's size.

**Minor 8. A paragraph left from round 5 contradicts Decision 006-064, and the table omits the timeline condition for an unmarked 1.8 lease.** (Confirmed in the documents.)

- **The contradiction:**
  - `stream-completion.md` lines 356-359 ("The engine's switch does not reach a marked lease"): "the hand-back and the pass's withdrawal of a lease whose mark counts need only `lease-withdrawal-v1`".
  - Lines 396-400, decision 4, Decision 006-047 and Decision 006-064 say both requirements.
  - A builder who follows the first text brings back security Major 1.
- **The table:** the row at line 546 ("A 1.8 lease that is not marked … withdrawn … only while `delivery-acknowledgement-v1` is recorded") does not carry decision 9's "or of a lease claimed on another timeline", although `lease_claim_timeline` is stamped for every 1.8 claim.
- **Fix:**
  - Rewrite lines 356-359 to say both requirements.
  - Add "and claimed on the current timeline" to line 546.
  - Consider one exception that the stale line points to: a `NOT_SENT` withdrawal of a marked lease. The token never left the server, so no capability can have been served. As written it needs both requirements, so after a removal it lapses into a review, while an unmarked `NOT_SENT` lease is withdrawn. Let it need only `lease-withdrawal-v1`, as Architecture decision 0098 has it.

**Minor 9. The mixed-fleet test that proves the security Major's fix names an actor that cannot exist.** (Confirmed in code.)

- **Where:** acceptance matrix line 755 and Task 006-066: "`ResolveSecrets` served by a floor control plane".
- **Why it cannot exist:**
  - The worker gRPC service is built only in the execution engine (`apps/execution-engine/src/lib.rs:799-813`). The control plane has none.
  - "Negotiation and activation" itself says a control plane "serves no worker call".
  - The real hole is a floor **engine** behind the same address serving the capability while this build's engine receives the hand-back.
- **Fix:** name a floor engine, with the stream on one engine and `ResolveSecrets` routed to the other. Run it before the activation and after the removal.

## Nit

1. **Decision 7 and step 5 differ.** Decision 7 makes an acknowledged or recovered lease answer `CANCELLED` only when "less than the margin" is left. Step 5 and Decision 006-051 also answer `CANCELLED` when a stop is asked. Align decision 7.
2. **Function count.** The decision record's Consequences counts "two functions (the timeline and the end of the marks)". Task 006-044 also adds `delivery_acknowledgement_required()`, which makes three, plus the trigger's function of Architecture decision 0100.
3. **A stale field `requires`.** Line 749 ("the trigger's pair equals the entry's `requires`") and Task 006-057 (tasks.md line 293, "its activation entry, with `empty_fleet: false` and `requires`") contradict Architecture decision 0100 decision 1 ("the activation entries … read it and do not repeat it").
4. **Acceptance wording.** The "Withdrawal" row (line 746) says that at the cap "a marked lease lapses at its deadline", which is round 5's wording. Write "is ended by the pass with `expire_attempt`".
5. **A removal followed by a re-activation gives old leases protection again**, against "removing the requirement ends the protection of the marks in flight". Withdrawals read the requirement at their own time, so a lease claimed before the removal is honoured again once it is re-recorded, even if a floor engine served it in between without recording. The window is small: a provisional lease is not renewed, and an unmarked `LEASED` lease expires within one time-to-live. Two fixes:
   - Honour a mark or a reservation only when the lease was claimed after the requirement's current recording. This is one comparison with the requirement row read under its share lock.
   - Make runbook step 5 unconditional for a binary of this build, rather than "only where the requirement is recorded". The command is idempotent.
6. **Statuses outside both lists** of §5 and of the refusals: a second `UNAUTHENTICATED`, a non-retryable `INTERNAL`, `OUT_OF_RANGE`, `DATA_LOSS`. They are neither repeated nor counted as refusals. Say that the leases wait for their give-up and that the call then counts under the third trigger.
7. **The commit is not outside the bound.** Line 479 says "a commit waiting on a synchronous standby is outside the bound", but the transaction's `statement_timeout` also applies to `COMMIT`. A cancelled wait for a synchronous standby leaves the commit committed locally, with a warning, and the client sees success (PostgreSQL behaviour). Say so. The timeline covers the failover that would lose it.
8. **`fsync` can change between probes.** It can be changed by a configuration reload, so a lease can be marked during the heartbeat interval after it is switched off. State that window.
9. **Lock contention in a burst.** Every withdrawal of a marked lease takes `FOR SHARE` on the same requirement rows. A burst of provisional expiries (a worker fleet cut off) creates one multixact per locker on two rows. Task 006-056 should measure the pass's withdrawal rate in such a burst.
10. **`end_lease_marks()` can touch history.** It nulls `lease_claim_timeline` "where it is not". If a finished step keeps the column ("keep them (`ELSE` the column)"), that touches historical rows and may break "one bounded transaction". Restrict it to `LEASED` and `RUNNING` rows.

## The checks you asked for

- **The fallback's third trigger:** it can fail to fire when it must (Major 1, Minor 6). A wrong fallback in a partial outage is unlikely. A fallback during shedding of the class `Acknowledge` is acceptable now that it is bounded and alerted, but the window should have a minimum span (Major 1, fix). "Given up" is defined for one call; "in a row", the re-ask and "answered" are not.
- **The pass's fall-through, case by case:**
  - At the cap: `expire_attempt` (`capped`).
  - A requirement not recorded: `expire_attempt` (`lapsed`).
  - Acknowledged: `expire_attempt`.
  - Another timeline or none: `expire_attempt` (`recovered`).
  - `RUNNING` with an unacknowledged mark: `expire_attempt`.
  - Replaced or renewed since the candidate read: nothing, which is correct because `Condition::LeaseLost` is re-checked under the run lock.
  - Each case is correct. No expired lease is left for a later pass. Residues: Minor 8 and Nit 4.
- **Removal of `delivery-acknowledgement-v1`:**
  - Ordering: each withdrawal reads the requirement under its share lock. A removal waits for those transactions, and a floor process is admitted only after it commits. Removals hold no run lock, so there is no lock cycle.
  - Rollback order: right in principle, but its drain waits for acknowledged leases (Minor 2).
  - `down` migration: the same wait (Minor 2).
  - Re-activation: Nit 5.
- **`CANCELLED` for acknowledged and recovered leases, and what the host then does:** it hands the lease back. Without declared secrets and before the expiry, that is an ordinary failure, or `StepCancelled` under a stop, as intended. With declared secrets, or after the expiry, it is a review (Minor 1).
- **The deadline and the `synchronous_commit` raise:**
  - Resolved as round-5 findings.
  - New: the zero timeout and the discarded connection (Minor 7); the commit and the statement timeout (Nit 7).
  - A committed acknowledgement flushed locally makes every earlier write-ahead record durable. So with `synchronous_commit = off` for the rest of the platform, a lost claim or withdrawal is always followed by the loss of everything after it, which the acknowledgement path tolerates.
- **Contradictions between documents:**
  - Lines 356-359 against Decision 006-064 (Minor 8).
  - Line 546 against decision 9 (Minor 8).
  - Decision 7 against step 5 (Nit 1).
  - The field `requires` against Architecture decision 0100 (Nit 3).
  - The "floor control plane" against the code and against "serves no worker call" (Minor 9).
  - Line 746 against the new pass row (Nit 4).
  - Runbook section 6 ("where it does not change") against step 5 ("after every restore"). The second is a superset, so this is harmless.
- **Arithmetic:**
  - Seven columns on `steps`: correct.
  - Seven alerts: correct.
  - Five round-5 Majors (2 + 1 + 1 + 1): correct.
  - 104 against 94 round trips is unchanged: the `synchronous_commit` raise shares the settings statement, and the pass's two requirement reads share one statement.
  - The trigger name `platform_requirements_guard_dependency` sorts after `platform_requirements_guard`, so it fires after it.
  - 2(T − p)/3 is right, but "more in every other case" is not (Minor 5).
  - "Two functions" is wrong (Nit 2).

## Evidence to ask for once built

Each is a test to add, then its run:

- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with `max_inflight` equal to the frame size and two minutes of `UNAVAILABLE` for this call alone: the host falls back and no attempt is spent (Major 1).
- `cargo test -p sagat-tests --test integration_durable_execution`, with:
  - the hand-back after `CANCELLED` (`stop_asked`, and `too_late` 1 ms past the expiry) on an acknowledged lease with declared secrets, root and scoped (Minor 1);
  - a 64-lease call held past its statement timeout, then split (Minor 6);
  - a call with 0.3 ms left that never sets a timeout of 0 (Minor 7);
  - every 1.8 claim working as a role without the right on `pg_walfile_name` (Minor 4).
- The rollback case of `provisional-lease-cutover` with a heartbeating acknowledged lease in flight: the drain ends with that lease still running (Minor 2).
- The mixed-fleet case with two engines, the stream on one and `ResolveSecrets` on the floor engine (Minor 9).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0100-a-fleet-requirement-may-require-another-and-the-registry-refuses-either-order.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 217-760)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§5, §6.1, §6.2, §10)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md, tasks.md, reviews/2026-11-21-delivery-ack/round-5/*.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md; docs/runbooks/disaster-recovery.md
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round5-to-6.diff
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs (2385-2544, 2882-2951)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/secrets.rs (263-293)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs (130-290)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/command.rs (900-1000, 1190-1230)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/scheduler/src/passes.rs (5601-5660)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/maintenance.rs (361-394)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql (67-85)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/status.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/execution-engine/src/lib.rs (799-813)
