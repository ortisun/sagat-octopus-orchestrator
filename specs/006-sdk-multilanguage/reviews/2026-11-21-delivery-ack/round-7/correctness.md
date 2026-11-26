**Correctness review of Architecture decision 0099 (delivery acknowledgement, worker protocol 1.8), round 7**

## Verdict

Most of round 6 is answered:

- The server's own transient answers no longer count toward the host's decline.
- The drain now waits on `worker_leases_provisional`.
- The `CANCELLED` hand-back passes the report fence.
- The server's start time replaces the timeline function.

My round-6 Major 1 is only **partly resolved**. Two mechanisms now replace the old discriminator: the engine breaker and the host's corrected third trigger. Each has a hole, and the holes do not overlap:

- The breaker needs 20 calls per engine and a 20% share of that engine's calls.
- The host's trigger asks for hand-backs answered "during" a call that has already ended. Its 60 s floor arrives after the cap whenever a lease's earlier deadline is under about 26 s.

So a failure of `AcknowledgeTasks` alone can still cycle a lease through four withdrawals and spend its attempt. One small server-side bound per attempt closes every one of these cases (Major 1).

The new parts bring in nine Minors:

- The liveness rule opens the gate before the beat has decided it. This is confirmed in code.
- The breaker's closing rule has no evidence to work from once marking stops.
- The list of reasons that mark an answer as "the server's own" differs between four texts.
- One normative sentence of §6.2 now follows the wrong rule.

**Counts:** 0 Blockers, 1 Major, 9 Minors, 9 Nits.

Nothing was built or run. Claims about the server build come from reading `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases`. Claims about PostgreSQL behaviour are from its source as I know it and are marked as such.

## Status of round-6 findings

### My report

| Round 6 | Status | Reason |
|---|---|---|
| Major 1: the third trigger is starved when flagged leases fill the host | **Partly resolved** | Server-reasoned answers no longer count, there is a breaker, give-up order is defined, the re-ask belongs to its call, and there is a 60 s floor. Four cases remain (Major 1 below). |
| Minor 1: `CANCELLED` hand-back refused by the report fence | **Resolved** | Decision 006-059 (1) covers an acknowledged lease past its expiry and with a stop asked, through `report_authority_fence` too. Task 006-050 tests both variants with declared secrets. Wording residue: Nit 9. |
| Minor 2: the drain waits for acknowledged leases | **Resolved** | Drains on `worker_leases_provisional`. The `down` refuses only on a provisional mark. The duration sentence is corrected. |
| Minor 3: the gate counts only the deadline | **Resolved** as worded (every call, non-OK share). The breaker reopens it (Minor 5). |
| Minor 4: `LANGUAGE sql` timeline function | **Resolved** by removal. `pg_postmaster_start_time()` is executable by `PUBLIC`. The `fsync` read has a smaller residue of the same kind (Nit 4). |
| Minor 5: "more in every other case" | **Resolved**: a heartbeat bound of 2 s is now required, and the sentence is corrected. Residue: the §6.2 sentence order (Minor 7). |
| Minor 6: a failure that depends on call size | **Partly resolved** | The split happens only after `ACKNOWLEDGEMENT_DEADLINE`. The round-6 case (update plus triggers past the 1 s statement timeout) ends with a statement timeout, so it is never split (Minor 3). |
| Minor 7: zero timeout, discarded connection | **Partly resolved** | Floor, 1 ms minimum and rollback are in place. "Each statement's timeouts" contradicts the single settings statement (Minor 6). |
| Minor 8: stale texts, table row, `NOT_SENT` | **Resolved** | |
| Minor 9: the floor control plane | **Resolved** (an engine of the floor build is now named). |
| Nits 1 to 10 | **Resolved** | Nit 5 by `required_since`, with residues (Nits 2 and 3). |

### Other round-6 reports, in my domain

| Finding | Status | Reason |
|---|---|---|
| Security Major 1: a burst declines every host on an engine | **Resolved** for the host. The breaker has the same reach, but only for 60 s after the burst, and a tenant can move its ratio in either direction (Nit 1). |
| Architecture Major 1, Design Major 1: the server's start time | **Resolved** | Residues: the live-engine check of `end-lease-marks` is empty in its only case (Minor 8); a memory snapshot is not listed (Nit 5). |
| Design Major 2: the drain is undefined | **Resolved** | The gauges are defined through the macro. |

## Blocker

None.

## Major

**Major 1. A failure of `AcknowledgeTasks` alone can still spend attempts: the breaker does not open on a quiet or partial failure, and the host's corrected trigger cannot fire for a host with full slots or for a lease whose earlier deadline is under about 26 s.** (Confirmed in the documents and by the arithmetic for (a) and (c); plausible for (b) and (d).)

- **Where:**
  - `stream-completion.md` lines 404-410 ("The breaker") and host item 5 (lines 650-662).
  - `sdks/SPEC.md` lines 373-379.
  - Decision 006-065.
  - Decision 10 of the decision record.

**(a) Low traffic: the breaker's minimum of 20 calls.**
- One lease whose every attempt ends with `ACKNOWLEDGEMENT_DEADLINE` (3 s) makes about seven attempts before its give-up at about 24 s. The §5 backoff is 200 ms doubling to 5 s, with jitter in [0.5, 1]; the attempts end at about 3, 6.2, 9.5, 13.1, 17.3 and 22.4 s, plus one more in flight.
- With a cycle of 24 to 28 s, that is 15 to 17 calls in 60 s, under 20. So the breaker never opens.
- The host treats the answer as the server's trouble and never declines.
- The lease cycles: withdrawals 1 to 4, then the fifth hand-back spends the attempt. With `maxAttempts = 1` the step fails for good on work that never started, which is round 5's failure.
- Per-call balancing across N engines splits the 17 calls N ways, which makes this worse.

**(b) A partial failure under 20% of an engine's calls.**
- Examples: one namespace's membership row lock held (lock timeout), or one binding's advisory lock queued behind a revocation.
- These hit one tenant's calls. On a shared engine that tenant is under 20% of the calls, so the breaker stays closed and that tenant's leases cycle to the cap.

**(c) The host's trigger.**
- "During each call another call … was answered (… or an `OK` answer to one of its own hand-backs)". The hand-backs of a call's own leases are sent at its give-up, which is the end of the call, so they are never "during" it.
- A host with `max_inflight` equal to the frame size has no other call in flight. Its previous cycle's hand-backs were answered before the step could be claimed again. So the case the clause was written for still never counts.
- The conformance case "with `max_inflight` equal to the frame size so that only the hand-backs are answered" would fail against a host that implements the text literally.
- The 60 s floor:
  - Give-ups come one cycle apart (about 23 s at the default time-to-live). So first to third is about 46 s, which fails the floor.
  - Only if the count runs on past three (an undefined reading) does the fourth give-up qualify, at 69 s. Its hand-back is the fourth and last permitted withdrawal, so the margin is zero.
  - For a lease whose earlier deadline is between 18 s (still marked) and about 26 s, the cycle is under 20 s. That covers a time-to-live of 18 to 25 s, or a `timeoutMs` in that range, which the pass withdraws at the attempt deadline. The fifth hand-back then exceeds the cap before three cycles span 60 s.
  - The justification "60 s is shorter than four give-ups … (about 92 s)" measures from the claim, while the floor measures from the first give-up. The comparison is wrong.

**(d) The breaker is per engine; marking happens at the claiming engine.**
- An SDK that pins its unary channel and its stream to different engines, or a pinning balancer, can have every acknowledgement land on engine B while engine A claims.
- With a failure local to B, B's breaker opens and stops B's marks. A keeps marking.
- The text says "usually reach the same engine".

**Why the tests miss it:**
- Task 006-058's shed case uses "two minutes of `ACKNOWLEDGEMENT_SHED`" on a busy harness, above 20 calls.
- No case runs one lease at a time, a time-to-live under 26 s, a failure limited to one namespace, or split channels.

**Smallest fix:** make the bound independent of what anyone observes.
- Add a step column `lease_unacknowledged_withdrawals`. The `NOT_ACKNOWLEDGED` withdrawal sets it to `+ 1` in its own update; it resets with the attempt. The claim marks only while it is below 2.
- This costs no statement: it is the row the claim already writes. It is not the journal read that option 7 rejected.
- After two lost acknowledgements of one attempt, the next lease is unflagged and runs with today's meaning. No failure of any kind can then spend an attempt by cycling.
- The breaker and the host's trigger become optimisations. Still:
  - count a call's own hand-backs answered after its give-up;
  - measure the 60 s from the first counted call's start;
  - say that the count continues past three.
- Tests:
  - a single cycling lease at an 18 s time-to-live, with every attempt answered `ACKNOWLEDGEMENT_DEADLINE`: the third claim is unflagged, no attempt is spent;
  - the same with a namespace lock held.

## Minor

**Minor 1. The liveness rule ("serves no worker call until a beat succeeds") opens the gate before the beat has decided it, and measures the window from receipt.** (Confirmed in code.)

- **Where:** `stream-completion.md` lines 411-414; Decision 006-059 (7); Task 006-046.
- **Code:**
  - `Member::beat` calls `self.gate.heard()` right after `heartbeat()` returns, before `decide()` (`crates/application/src/instances.rs:647-650`).
  - `decide()` then reads the requirements in another statement (`instances.rs:681`, `?`). If that read fails, the gate is left open with a fresh `heard()`.
  - `instance_heartbeat` updates `last_seen` without checking any requirement (migration `20270113000000_instance_registry.up.sql:250-277`).
- **Scenario:**
  1. An engine of the floor build is silent past 30 s, and an activation records `delivery-acknowledgement-v1`.
  2. On reconnect its heartbeat succeeds and the requirements read times out.
  3. For up to one beat (10 to 12 s) it serves `ResolveSecrets` unrecorded, on leases claimed after `required_since` by this build's engines.
  4. A hand-back to this build is then withdrawn: this is security Major 1 of round 5.
- **Second effect:** `heard()` is stamped at receipt, later than the database's `last_seen`. So the engine believes it is live for the tail of its heartbeat call after a census has excluded it.
- **Fix:**
  - "A beat succeeds" must mean the heartbeat and the gate's decision under the epoch it returned. Better, `instance_heartbeat` returns the missing requirements itself, as `instance_register` does, so one statement both marks the engine live and closes its gate.
  - Measure the window from the instant before the beat is sent.
  - Test the requirements read failing after a successful heartbeat.

**Minor 2. The liveness rule refuses every worker call, `Heartbeat`, `CompleteStep` and `FailStep` included. A registry-only failure then lapses leases into reviews.** (Plausible.)

- **What changes:** the fail mode of Architecture decision 0082 kept serving through a plain connection failure "because stopping the fleet would turn a short outage into a longer one" (`instances.rs:27-30`).
- **When it bites:** the beat fails while the request pool works, for example a beat connection that is stuck or a pool that is exhausted separately.
- **Consequence:** the new rule refuses heartbeats, so `Activity` leases expire as unknown outcomes. "Attempted at once when its connection returns" has no trigger in the beat loop; recovery can take up to one beat.
- **Fix:** refuse only what the hazard needs, which is capability calls (`ResolveSecrets`, `ResolveDatabase`, blob transfers) and new claims. Keep heartbeats and reports.

**Minor 3. "The server's reason" is defined four different ways, and the split misses the case it was added for.** (Confirmed in the documents.)

- **The four definitions:**

  | Document | What counts as the server's own answer |
  |---|---|
  | Decision record, decision 10, line 142 (breaker) | shed and deadline |
  | `stream-completion.md` line 407 (breaker) | shed, deadline, lock or statement timeout, store unavailable |
  | `stream-completion.md` line 652 (host) | shed, deadline, store unavailable |
  | `sdks/SPEC.md` line 373 (host, normative) | `ACKNOWLEDGEMENT_SHED` and `ACKNOWLEDGEMENT_DEADLINE` only |

- **Why it matters:** the server puts a machine code in `x-sagat-error` on every error (`crates/api-grpc/src/status.rs:29-37`). A lock timeout or store error is `UNAVAILABLE` with code `UNAVAILABLE`.
  - Under `SPEC.md` it is "a status without the server's reason", and it counts toward the decline.
  - Under the host item of `stream-completion.md` it is waited out.
  - Six SDKs will diverge.
- **The split:** it happens only "after `ACKNOWLEDGEMENT_DEADLINE`". A 64-lease update past the 1 s statement timeout answers a statement timeout well before the 3 s deadline, so it is never split. That is the round-6 Minor 6 scenario.
- **Fix:**
  - Give every server-made transient ending a reason from one closed list (`ACKNOWLEDGEMENT_SHED`, `ACKNOWLEDGEMENT_DEADLINE`, `ACKNOWLEDGEMENT_STORE`). The host detects them by `x-sagat-error` value. Write the same list in all four places.
  - Split after any of them.

**Minor 4. The breaker's closing rule has nothing to judge once it is open.** (Confirmed in the documents.)

- **What happens when it opens:** no lease is flagged, so no `AcknowledgeTasks` call is made apart from the re-asks of leases already flagged, which drain within one give-up.
- **"Closes after 60 s below the threshold"** is then:
  - either never true, if "below" needs 20 calls, so the protection stays off until a restart;
  - or true by default, if fewer than 20 calls counts as below, so it closes while the cause persists, reflags, reopens, and hands back a batch on every cycle.
- **Fix:** a half-open state. While open, flag one claim in N (or one a second), and close only after 20 such calls below the threshold. Define the state with fewer than 20 calls. Test both a cause that persists and one that ends.

**Minor 5. The breaker can make the gate pass with the feature off.** (Confirmed in the documents.)

- **How:** Task 006-056 runs "a burst that fills the admission reserve", which sheds the class `Acknowledge` by design and so opens the breaker. Once flags stop, the criteria "no heartbeat shed while acknowledgements are admitted" and "no more than 15% below 1.7" become easier.
- **Ambiguity:** "fewer than 0.1% of calls answered other than `OK`" does not say whether the burst's sheds are included. If they are, the gate fails by construction; if not, the throughput run is not checked for the breaker.
- **Fix:**
  - Require `worker_acknowledgement_breaker_transitions_total` to stay 0 and report the marked share of claims in the transitions runs. A run in which the breaker opened is not evidence.
  - Say that the 0.1% applies to the transitions and timed 64-lease runs, not to the burst.

**Minor 6. "Each statement's timeouts are the smaller of 1 s and the time left" contradicts "one statement sets, for the transaction".** (Confirmed in the documents.)

- **Where:** `stream-completion.md` lines 498 and 504; Decision 006-043; Task 006-049.
- **Why:** set once at the start, a statement that begins at 2.9 s still has a 1 s timeout, so the deadline does not fall between statements. Neither does the `COMMIT`, which the text says the timeout bounds.
- **Cost of the alternative:** setting the timeouts before each statement adds about four round trips. A `set_config` inside a statement does not change that statement's own timeout, because it is armed at the statement's start (PostgreSQL behaviour). Either way, the "about eight statements" count and the 104 against 94 round trips change.
- **Fix:** choose one.
  - Either per-statement settings, pipelined and counted in the budget.
  - Or one setting, with the call allowed to end up to 1 s past the deadline, and the check "before the commit is sent" kept explicit.

  Then correct the 3 s claim and the test "a pool held full for 5 s answered within 3.5 s" accordingly.

**Minor 7. In `sdks/SPEC.md` §6.2 the new 2 s rule was inserted before "It then waits a random delay … registers again … declining", so the fallback procedure now reads as the consequence of an `ACKNOWLEDGED` with under 2 s left.** (Confirmed in the documents, lines 379-383.)

- **Fix:** move the 2 s sentence to the `ACKNOWLEDGED` verdict earlier in the paragraph, and start the fallback with "On fallback the host …", as `stream-completion.md` item 5 does.

**Minor 8. `end-lease-marks` refuses "while any engine's registry beat is newer than the server's start time". That is always true in the one case the command exists for.** (Confirmed by reasoning.)

- **Where:** `stream-completion.md` lines 452-458; Task 006-054.
- **Why:**
  - A logical restore into a server that keeps running restores `serving_instances` rows whose `last_seen` is later than that server's old start time.
  - Engines still connected also beat later.
  - So the command always refuses. The operator learns to pass `--expect-live 0` by routine, and the safety check protects nothing.
  - In every other case the incarnation has already ended the marks.
- **Fix:** refuse while `pg_stat_activity` shows a session of the runtime role, which the owner can read and which is the direct evidence of a live engine. Alternatively, use the registry's own 30 s window, measured on the server after the restore.

**Minor 9. The `down` "drops the functions and the table only when it is empty". `platform_operator_actions` is append-only and gains a row on every run of `end-lease-marks`.** (Confirmed in the documents, `tasks.md` line 181.)

- **Consequence:** after any restore the `down` leaves the table in place. A later `up` then fails on `CREATE TABLE`, so the migration is not reversible as its checklist requires.
- **Fix:** create the table in a migration of its own that has no `down` of the feature (it is deployment-level evidence, like `platform_requirement_history`), or make the `up` idempotent for it.

## Nit

1. **The breaker's reach.**
   - Considered option 8 rejects counting shedding toward the host because "any tenant [could] switch the protection off for every host on an engine". The breaker has the same reach, every host on the engine, though only until 60 s after the burst.
   - Its ratio can be moved either way by a tenant: flooding sheds the others' calls, and cheap `NOT_HELD` calls with forged tokens dilute the share.
   - State this. A breaker keyed by namespace would also cover Major 1 (b).
2. **The acknowledgement and `required_since`.** `stream-completion.md` says a lease claimed before the current recording is `recovered` for the acknowledgement, but the acknowledgement's statements do not read `required_since`.
   - Either add an unlocked scalar subquery on `platform_requirements` to the locking read.
   - Or state that the acknowledgement treats it as provisional. That is safe, since withdrawals decide alone.
3. **`lease_claimed_at` against `required_since`.** Both are `clock_timestamp()` on one server, and an equal value does not count, which is safe.
   - The unsafe direction needs the wall clock to step backward across a removal and a re-activation.
   - After a failover, a new primary whose clock is behind the old one leaves new leases uncounted (the safe direction).
   - State both.
4. **"An unreadable value counts as off"** cannot be written in SQL: `current_setting` raises for a setting the role may not see, whatever `missing_ok` says (PostgreSQL behaviour), and the claim statement would then fail. `fsync` is visible to every role in stock PostgreSQL, so say that, and keep the Task 006-044 check.
5. **Memory snapshots.** A virtual machine snapshot restored with its memory, or a checkpoint-and-restore of the process, keeps the postmaster and its start time while it loses commits. List it beside the logical restore as covered only by the runbook step.
6. **Rollback order.**
   - Step 4 (switch back) after step 3 (binary rolled back, read at start) needs a second restart of every engine. Fold it into step 3's configuration.
   - While the switch is off for the drain, the `NOT_SENT` withdrawals of Architecture decision 0098 stop, so those leases lapse into reviews. State this, or add a switch that stops marking alone.
7. **The host's count.**
   - Say whether a given-up call that does not qualify (one attempt reasoned) breaks the row or is skipped.
   - Say whether an `ACKNOWLEDGED` that the 2 s rule treats as `CANCELLED` resets the count.
8. **Breaker state on restart.** The breaker lives in memory, so an engine restarted or added in a scale-out during the cause starts closed and flags until it has seen 20 calls. State this.
9. **Decision 006-059 (1)** asks the build of Architecture decision 0098 to relax the fence "for an acknowledged lease", a notion that build does not have. Say the relaxation is this build's, or phrase it as a `LEASED` lease whose hand-back the floor treats as an ordinary failure.

## The checks you asked for

- **The breaker:**
  - Thresholds: 20% of at least 20 calls means at least 5 failing calls.
  - It does not open for one cycling call (about 17 calls a minute) or for a share under 20% (Major 1 (a) and (b)).
  - It is per engine while marking happens at the claiming engine (Major 1 (d)).
  - It has no closing evidence (Minor 4) and can hide the gate's results (Minor 5).
  - Leases already flagged are handed back once each; those claimed again while it is open are unflagged, so no step accumulates withdrawals because of it.
  - Its interaction with the host's fallback is consistent, apart from the list of reasons (Minor 3).
- **The host's rule:**
  - "In a row" in give-up order with a 60 s floor is coherent only if the count runs on past three. Even then, at the default time-to-live it fires at the fourth give-up, the last permitted withdrawal, and too late under about 26 s (Major 1 (c)).
  - The hand-back evidence cannot be "during" the call.
  - The reset on `ACKNOWLEDGED` is coherent with the rest; Nit 7 covers the edge cases.
- **`pg_postmaster_start_time()`:**
  - It changes on every case claimed (promotion, point-in-time recovery, snapshot or base backup started as a server, managed restore, cut-over, return of an old primary).
  - Whether a crash restart that keeps the postmaster changes it is left to Task 006-044. Either answer is safe: the acknowledgement and the capability record flush, and a write-ahead record that reached the kernel survives a crash of PostgreSQL alone. That is also why the `fsync` check matters only for a crash of the operating system, which restarts the postmaster anyway.
  - A mark written just before a restart stops counting and keeps today's meaning.
  - It changes wrongly only on a planned restart, which is accepted.
  - It stays wrongly unchanged on a memory snapshot (Nit 5) and on a logical restore (step 5).
  - A standby equals its primary only if both started in the same microsecond.
- **`required_since`:**
  - The census runs before `required_since` in the same transaction, and a registration waits on the epoch lock. So a claim between `required_since` and the commit is safe, apart from the liveness window of Minor 1.
  - A claim in the same instant does not count.
  - Clocks: Nit 3.
- **The `CANCELLED` relaxation and the report fence:** correct. Under a stop, `end_attempt` turns the hand-back into `StepCancelled` (`crates/domain/src/execution/command.rs:926`).
- **The rollback:** the drain is correct now. Residues are Nit 6 and the `down` (Minor 9).
- **Contradictions:**
  - The list of server reasons in four texts (Minor 3).
  - Timeouts set once against per statement (Minor 6).
  - The §6.2 sentence order (Minor 7).
  - Option 8 against the breaker (Nit 1).
- **Arithmetic:**
  - Seven columns on `steps`: correct.
  - Two functions plus one table: correct.
  - (6 − 3) × 2 / 3 = 2: correct.
  - "60 s is shorter than four give-ups (about 92 s)" compares from the wrong origin; first to third is about 46 s (Major 1 (c)).
  - A single failing lease makes about 15 to 17 calls in 60 s, under the breaker's 20.

## Evidence to ask for once built

Each is a test to add, then its run:

- `cargo test -p sagat-tests --test integration_durable_execution`:
  - one lease at an 18 s time-to-live with every attempt answered `ACKNOWLEDGEMENT_DEADLINE`: no attempt spent (Major 1);
  - a 64-lease call past its statement timeout, split (Minor 3);
  - a statement that starts with 0.5 s left ending by the deadline (Minor 6);
  - `end-lease-marks` after a logical restore with no engine connected, not refused (Minor 8);
  - `down` then `up` after one `end-lease-marks` (Minor 9).
- An engine test: a heartbeat answered and then the requirements read failing, with no worker call served until the gate is decided (Minor 1).
- `SAGAT_SDK_SCENARIO=delivery-acknowledgement cargo test -p sagat-tests --test sdk_conformance --locked -- --ignored --nocapture`, with `max_inflight` equal to the frame size and an unreasoned 503: falls back with no attempt spent (Major 1 (c)).
- The Task 006-056 runs reporting the breaker's transitions and the marked share (Minor 5).

## Files consulted

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§5, §6.1 to §6.5)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md, tasks.md, reviews/2026-11-21-delivery-ack/round-6/*.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md; specs/037-compensation-safety/spec.md
- /private/tmp/claude-501/-Users-marcelo-Documents-Projects-sagat-octopus-orchestrator--claude-worktrees-prompt-master-temporal-workflow-05b1ec/8580ef8b-73f3-4ee0-848d-35f946ce8b5a/scratchpad/delivery-ack-round6-to-7.diff
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/instances.rs (1-75, 380-700)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/status.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/api-grpc/src/limits.rs (536-607)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/execution-engine/src/cli.rs (line 76)
