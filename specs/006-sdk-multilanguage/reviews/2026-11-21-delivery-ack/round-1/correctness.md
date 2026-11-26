# Review of ADR-0099: delivery acknowledgement at worker protocol 1.8 (design round 1)

## Verdict

The central exactness argument holds for root leases, but only on two conditions the documents do not state yet. Those are Major 2 (where the withdrawal reads the marks) and Major 3 (whether a hand-back is accepted). The argument is: the acknowledgement and the withdrawal both take the run lock, and both check the lease before writing.

- **Lock order is safe.** Both writers open through `begin` with `Lock::Wait`, which takes the continuation-chain lock and then the run lock (`crates/infra-postgres/src/execution.rs:1563-1577`). Their lock order is the same. Each run of an acknowledgement call is its own transaction and commits before the next one opens, so one call never holds two run locks and cannot deadlock.
- **Replay still matches the projection.** Nothing in `crates/domain` reads `lease_expires_at` or `attempt_deadline_at` (searched), so new columns on the step row that the replay never derives do not break that equality.

I found no Blocker. I found nine Major findings, eleven Minor and seven Nit. Three are confirmed in the code:

- Major 1: the document says the acknowledgement loads no run state, but "open the run as Heartbeat does" loads all of it.
- Major 3: the hand-back after a cancellation is refused by the current lease check for any step that declares secrets.
- Major 8: three existing conformance scenarios run with a 3 s time-to-live, which the 7 s threshold excludes from 1.8; one of them is the negotiation scenario the plan says will be accepted at 1.8.

The other Major findings are plausible failure sequences, each with a stated path. Nothing was built or run; every claim about the code comes from reading it.

## Major

**Major 1 — Decision 3 contradicts itself: "open the run as Heartbeat does" loads the whole run, and the cost comparison with variant 2 rests on the opposite.** (confirmed in code)

- **Where:**
  - ADR-0099 decision 3 ("MUST NOT load the run's state") and considered option 1 ("r transactions that load no run state").
  - `stream-completion.md`, "The server's acknowledgement" steps 1 and 4.
  - `research.md` D006-011 ("no run-state load") and D006-014.
- **What is wrong:**
  - Step 1 says to open the run with `begin(tenant, run, Lock::Wait)`. That is `PgStore::begin` (`crates/infra-postgres/src/execution.rs:1563-1577`). It calls `begin_row` (lines 282-460), which reads every step row of the run, the definition, the sagas, the collections, the loops, the scope sagas and the chain. It then restores a full `RunState`.
  - Heartbeat's `open` is exactly this (`crates/application/src/worker.rs:2547-2562`).
  - The checks the design keeps also need the definition: `ensure_bound` and `managed_database` call `tx.parts()` (`worker.rs:2715-2769`). So "the checks of Heartbeat" cannot run on a bare run lock either.
- **Why it matters:** the cost argument has to be restated before it can decide anything.
  - The ADR's reason to prefer option 1 over variant 2 is that variant 2 "loads the run's state", while option 1 does not. Both load it.
  - The derivation of A has the same slip: "a completion ... opens the run the same way, then loads the run's state". The bound on A survives, because a completion still does more, but the wording is wrong.
  - At low load a frame holds the tasks of one run, often one task. The difference "per frame against per task" then vanishes too, which leaves the unchanged goldens as the real advantage.
  - T006-054 asserts only "fewer statements than `FIRST_HEARTBEAT`", which a full `begin` still satisfies. So no test would expose this.
- **Fix:** choose one of:
  - (a) Specify a lean store entry `begin_lease`: the chain lock and then `SELECT … FROM runs … FOR UPDATE` in the same order as `begin`, the definition from the cache for the binding check, and one `UPDATE steps … WHERE lease_token_hash = $ AND status IN ('LEASED','RUNNING') RETURNING …`. Pin its statement count in T006-054.
  - (b) Keep `begin`, delete "loads no run state" from the ADR, the research notes and the stream contract, and restate the comparison with variant 2 honestly. Variant 2 additionally applies a transition, appends `step.started` and writes the outbox.

**Major 2 — Where the withdrawal reads the marks is ambiguous, and the acknowledgement accepts a lease that has expired but has not yet been reaped. Together they allow a withdrawal of a lease whose handler is running, and no planned test interleaves them.** (plausible; the acceptance of an expired lease is confirmed in code)

- **Where:**
  - `stream-completion.md`, "The withdrawal", first bullet ("Its `EXPIRED` arm also returns the two marks").
  - ADR-0099 decision 4.
  - `tasks.md` T006-050 and T006-051.
- **What the code does:**
  - The acknowledgement keeps Heartbeat's check, `lease_matches_instance` (`execution.rs:4999-5019`). That check tests the token hash and `status IN ('LEASED','RUNNING')` but not the expiry. So an acknowledgement that arrives after `lease_expires_at`, before the lost-lease pass runs, commits and renews the lease.
  - The lost-lease pass reads its candidates without the run lock (`maintenance.rs:361-424`).
  - Under the lock it re-checks only the deadlines (`passes.rs:473-485`, and 4484-4497 for paths).
  - The acknowledgement changes neither the step status nor the run revision. So `begin_path`'s revision check (`passes.rs:4473`) does not detect it either.
- **Failing scenario:** a step with `timeoutMs = 1000`, time-to-live 30 s.
  1. The claim commits at t = 0. The host's acknowledgement is delayed, and the lease expires at t = 30.
  2. At t = 30.2 the candidate query returns the lease as `EXPIRED`, provisional and unacknowledged.
  3. At t = 30.3 the acknowledgement commits. It is accepted because there is no expiry check. It moves the attempt deadline to 31.3 and renews the lease to `least(60.3, 31.3) = 31.3`. The host starts the handler, and the step stays `LEASED` until the first heartbeat at about t = 30.8.
  4. Suppose the pass's worker for that candidate takes the run lock at t = 31.4 (a long `each` batch, or lock waits). `Condition::LeaseLost{Expired}` holds, because 31.3 is not after 31.4.
  5. If the build decides "withdraw or expire" from the marks the candidate returned, as the bullet invites, it withdraws a running, acknowledged lease. The step returns to `READY` with the same attempt and no review, which breaks the exactness claim and constitution principle III.
- **Why the tests do not catch it:** T006-051's two barrier orders always leave the renewed expiry in the future when the withdrawal re-checks. So a stale-marks implementation passes both. The interleaving "candidate read, acknowledgement, renewed expiry passes, apply" is not planned.
- **A second effect, even with correct reads:** the same lease, acknowledged with a 1 s timeout, is then ended with cause `Expired`, not `TimedOut`. The `TIMED_OUT` row was skipped when it was provisional, so the failure is recorded as `LEASE_EXPIRED`, not `ATTEMPT_TIMEOUT`.
- **Fix:**
  - (a) State normatively that the provisional, unacknowledged condition is part of the re-check under the lock: a new `Condition::LeaseNotAcknowledged`, read from the row inside the locked transaction. State also that it is part of the withdrawal's fenced `UPDATE` (`AND lease_acknowledged_at IS NULL`). Then remove "returns the two marks" from the candidate query, or mark it advisory only.
  - (b) Make the acknowledgement answer `NOT_HELD` for a provisional lease whose `lease_expires_at` has passed. Nothing is written, so exactness is kept. This makes "the acknowledgement deadline is the expiry" literally true, and it makes the conformance step "held acknowledgement released after `expireLease`" (T006-056) deterministic. Today that step's result depends on whether the pass ran before the release.
  - (c) Add the interleaving above to T006-051, held with a barrier after the candidate query.

**Major 3 — The hand-back after `ACKNOWLEDGED` with `cancel_requested = true` is refused by today's lease check for a step that declares secrets. The lease then lapses into a review, against decision 8 and the failure table.** (confirmed in code for the current report path)

- **Where:**
  - ADR-0099 decision 8.
  - `stream-completion.md`, the failure-table row "The run's cancellation is requested before the acknowledgement".
  - `research.md` D006-021.
- **What the code does:**
  - `fail` (`worker.rs:2332-2411`) passes every `FailStep` through `checked_secret_report`. For a step with declared secrets and a configured vault, that calls `report_authority_fence(…, error_code == "CANCELLED" && !retryable)` (`worker/secrets.rs:265-293`).
  - `TASK_NOT_STARTED` is retryable, so the flag is false.
  - `attempt_authority_fence` (`worker.rs:2813-2840`) then requires `!s.cancel_requested()` and `stop_reason().is_none()`, so it answers `StaleAttempt`.
- **Failing scenario:** a run whose activity declares `secrets: ["api-key"]`.
  1. The run is cancelled after the claim and before the acknowledgement.
  2. The acknowledgement answers `ACKNOWLEDGED`, `cancel_requested = true`, and renews the lease.
  3. The host hands the lease back with `TASK_NOT_STARTED`, and the hand-back is refused.
  4. The lease is acknowledged, so the withdrawal at expiry does not apply. The lease lapses: an unknown outcome on a stopping run, which goes to review under principle III, for work that never ran.
- **Why the tests do not catch it:** D006-021 defers this to "the ADR-0098 build", which does not exist (no `lease_withdrawn` or `TASK_NOT_STARTED` anywhere in `crates/`). T006-050 tests only the expiry withdrawal of a cancelled run. The conformance step "held acknowledgement released after `cancel`" uses no declared secret.
- **Fix:**
  - (a) Require that `TASK_NOT_STARTED` on a `LEASED` step is accepted regardless of `cancel_requested` and the stop reason. It is the one report that proves nothing ran. Add a T006-050 or T006-051 case with declared secrets.
  - (b) Better and cheaper: when the step is provisional and `cancel_requested` holds, let the acknowledgement withdraw the lease in its own transaction (cause `NOT_STARTED`, the settle cancels the step) and answer `NOT_HELD`. This writes an event only on this rare path, and removes a round trip and a failure window.

**Major 4 — Scoped (collection and loop) leases are not designed at all, yet a 1.8 session is a superset of 1.5.** (plausible; the separate code paths are confirmed in code)

- **Where:** every server section of `stream-completion.md`, and ADR-0099 decisions 2, 3, 4 and 7.
- **What is wrong:** the design names only root functions: `begin`, `verify`, `lease_matches_instance`, `ensure_bound`, `Condition::LeaseLost`, `expire_attempt` and the `plan.rs` drain. Scoped instances run through other paths:
  - Heartbeat goes through `heartbeat_path` with `begin_current_path` and `ensure_path_bound` (`worker/scoped.rs:93-151`).
  - The lost-lease pass goes through `apply_path_if` with `load_path` (unlocked), `begin_path` (a revision check) and `expire_path_instance` (`passes.rs:4430-4497`, 5656-5675).
  - The lease columns are written by a separate statement (`execution/path.rs:1227-1238`).
  - Capabilities are checked by `live_path_fence` and `lease_unexpired`.
  - The root `drain` skips scoped instances. A collection or loop is "drained by the scheduler under its own fence" (`plan.rs:363-409`), so "the settle cancels a `READY` step of a stopping run" is unproven for an iteration.
  - The event of the withdrawal is written as `(node, attempt, generation, cause)`, with no address. It would have to be an addressed event.
- **Why it matters:** the branch's own work (root collections) makes scoped leases common. A builder following the text either leaves scoped leases unmarked, a silent gap not listed among the gaps, or marks them without the path withdrawal, so they lapse as `outcome="lapsed"`. The T006-047, 050, 051, 052 and 053 matrices name no scoped case.
- **Fix:** add a section on scoped instances, with each named path, the addressed event, the cancellation of an iteration in a draining collection or loop, and scoped rows in each task's evidence. Or decide explicitly that scoped leases of 1.8 sessions are not provisional, and add that as a gap with its consequence.

**Major 5 — The rollback procedure is unsafe as written.** (plausible)

- **Where:**
  - ADR-0099, "Rollback".
  - `research.md` G006-008.
  - `stream-completion.md`, the failure-table row "The requirement is removed while provisional leases exist".
- **Problem (a): an older binary cannot read journals that hold the cause.** The text says "remove the requirement first … Then roll the binary back", and in the same paragraph that a journal holding `NOT_ACKNOWLEDGED` "cannot be replayed by a binary older than this decision". Removing the requirement does not remove those events. Every run that ever had such a withdrawal becomes unreadable to the rolled-back binary: its history, a reset that copies the journal, and any replay. ADR-0082 already says a rollback below a recorded requirement is unsupported. This procedure rolls back below data the requirement protected.
- **Problem (b): sessions stuck at 1.8 hand back every lease.** Sessions accepted at 1.8 keep their minor "until they register again", and nothing makes them register again.
  - After the binary rollback, every `AcknowledgeTasks` call answers `UNIMPLEMENTED`. The host rule for "any other status" is to hand every lease of the call back.
  - The fleet then claims and hands back in a loop and does no work.
  - On a binary before ADR-0098, `TASK_NOT_STARTED` is "an ordinary retryable failure" (ADR-0098 decision 5). It consumes an attempt, and a step with `maxAttempts: 1` fails for good.
- **Fix:**
  - State that a rollback below this decision is supported only to a build that decodes the cause. In practice ADR-0098 and ADR-0099 ship the codec together, and the cause is known in the ADR-0098 build.
  - Require the host, on `UNIMPLEMENTED` from `AcknowledgeTasks`, to stop dispatching, hand nothing back, and register again (negotiate again). Or require the server to invalidate 1.8 sessions when the requirement is removed.
  - Add a `provisional-lease-rollback` case to T006-065.

**Major 6 — A binary that does not know the columns leaves stale marks. After a new upgrade they make a legacy lease look provisional.** (plausible)

- **Where:** ADR-0099 decision 2 ("Every write that clears or replaces a lease clears both columns"), the `down` rule, and `stream-completion.md`, "The provisional mark".
- **What is wrong:** only the new build's statements clear the marks (`execution.rs:7036-7058` and `7359-7385`, `path.rs:1227-1238`). The `down` migration refuses while any mark exists, so after the documented rollback the columns stay.
- **Failing scenario:**
  1. A provisional lease is unacknowledged when the binary is rolled back.
  2. The old binary lets it lapse and later leases the step again to a 1.7 host. The old binary does not touch the columns, so the row is now `LEASED`, with `lease_offered_at` set and `lease_acknowledged_at` empty, for a lease that was never provisional.
  3. The fleet is upgraded again and the requirement recorded again.
  4. The 1.7 host's secret and blob calls are refused with `TASK_NOT_ACKNOWLEDGED`.
  5. If that host dies after its handler ran, the pass withdraws the lease as `NOT_ACKNOWLEDGED`: no review, the same attempt run again, against principle III.
- **Why the tests do not catch it:** T006-065 tests only the forward cut-over.
- **Fix:** make the mark self-validating. Options:
  - A `BEFORE UPDATE` trigger that clears both columns whenever `lease_token_hash` changes and the statement did not set them. This works under every binary.
  - Or store the generation the mark belongs to, and treat a mark as valid only when it equals the row's `generation`.
  - Add the rollback and roll-forward case to T006-065.

**Major 7 — The derivation of the 7 s threshold leaves out terms, and the histogram called "the measured A" measures something else.** Safety is not affected; liveness at the threshold is. (arithmetic confirmed; behaviour plausible)

- **Where:**
  - `stream-completion.md`, "Why the deadline is the lease expiry", items 2, 3 and 5.
  - ADR-0099 decision 5.
  - `research.md` D006-016.
  - "Observability" (the histogram "the measured A of the derivation").
  - `tasks.md` T006-064.
- **Missing term (a): one call is many transactions.** A is one transaction. One call holds up to 64 tasks over up to 64 runs, each in its own transaction, in request order, one after the other. So L = R + Σ A, not R + A. At the measured worst 99th percentile:

  | Frame | Calculation | L |
  |---|---|---|
  | 64 runs | 6 + 64 × 0.103 | 12.592 s |
  | 10 runs | 6 + 10 × 0.103 | 7.03 s |

  Both exceed the 7 s threshold.
- **Missing term (b): the measurement excludes the network and the retries.** A comes from a scenario whose "workers are tasks in this process" and where "a network between the services and the database" is "not measured" (`scalability-ha.md:243`, 296). It excludes the worker-to-server round trip, transport security, the backoff of §5 after one failed attempt, and waits on the run lock (`Lock::Wait`) behind a long transaction.
- **Missing term (c): the histogram measures the whole delivery.** `worker_task_acknowledgement_delay_seconds` is acknowledgement time minus offer time. That includes the time between commit and read (up to R), so it is about the whole delivery, not A. Re-deriving R + ⌈A⌉ from it in T006-064 counts R twice.
- **Fix:**
  - Define A_call as the duration of one acknowledgement call for a full frame, measured on the host from send to answer, across a real network.
  - Set the threshold to R + ⌈99th percentile of A_call⌉ + one backoff step.
  - Or run a call's per-run transactions with bounded concurrency.
  - Rename the histogram to the delivery delay and add a host-side or server-side call-duration histogram as A.

**Major 8 — The conformance plan cannot pass as written.** (confirmed in code)

- **Where:**
  - `tasks.md` T006-063 ("`protocol-negotiation` offers 1.8 and is accepted at 1.8 with the requirement") and T006-056.
  - `research.md` D006-024.
  - The acceptance matrix rows "Six SDKs" and "Older minors".
- **What is wrong:**
  - **Three scenarios cannot be offered 1.8.** `tests/contract/sdk-conformance/scenarios/protocol-negotiation.json:4`, `worker-stream-faults.json:5` and `worker-transport-faults.json:5` set `leaseTtlSeconds: 3`. Under the design's own rule (no 1.8 below 7 s) these three can never be offered 1.8. The negotiation scenario's planned expectation contradicts the threshold.
  - **There is no way to run the suite at 1.8.** 52 scenario files pin `protocolVersion` (74 occurrences, mostly "1.4"). "Every other scenario passes at 1.8 with its existing golden, unblessed" has no mechanism. Editing the pins edits the scenarios. The matrix also says "the whole suite passes at its pinned minors" in the same delivery, which is a different run.
- **Fix:**
  - Add a harness override (for example `SAGAT_SDK_PROTOCOL_OVERRIDE=1.8`), with the server's time-to-live floored at 7 s, and an explicit list of exempt scenarios (the three above).
  - Run the suite twice: pinned, and at 1.8.
  - Give the negotiation probe its own server block with a time-to-live of at least 7 s, plus a case under 7 s that expects 1.7.

**Major 9 — A host whose clock is skewed can hand back every lease without acknowledging it.** (plausible)

- **Where:** `sdks/SPEC.md` §6.2 ("until the leases have expired by the host's clock") and `stream-completion.md`, "The host at 1.8" item 4.
- **What is wrong:** each attempt, and the decision to give up, is bounded by `expires_at_epoch_ms` against the host's wall clock. Take a host whose clock runs 30 s ahead, with a 30 s time-to-live.
  - Every lease looks expired on receipt, so the host hands everything back and the worker does nothing.
  - Its "give up" log line is the only signal.
  - Today such a host still works: §6.3 clamps the heartbeat interval and lets an unsynchronized host set it explicitly.
- **Why this is safe to fix:** the server alone decides exactness, so the host gains nothing by giving up early.
- **Fix:** have the host repeat until a verdict (a lost lease ends as `NOT_HELD` after the withdrawal), bounded only by a monotonic cap counted from receipt, for example 60 s. Or count "time left" on the monotonic clock from receipt, using the lease's remaining time at receipt as the bound. Add a skewed-clock unit test to T006-057 to T006-062.

## Minor

**Minor 1 — The acknowledgement can extend the attempt deadline of a running attempt.**
- **Where:** `stream-completion.md`, "The server's acknowledgement" step 3.
- **What is wrong:** the deadline is moved whenever `lease_acknowledged_at` is empty and `lease_offered_at` is set, with no status condition. A non-conforming host can heartbeat first (the step becomes `RUNNING`; decision 7 serves it), work for ten minutes, then acknowledge. That moves `attempt_deadline_at` ten minutes later and doubles its `timeoutMs`.
- **Fix:** move the deadline only when the step is `LEASED`, which is the document's own definition of "provisional and unacknowledged". Or cap the move at the lease time-to-live.

**Minor 2 — The 7 s threshold is checked where the session registers, not where the lease is claimed.**
- **Where:** `stream-completion.md`, "Negotiation and activation", and T006-048.
- **What is wrong:** `SAGAT_LEASE_TTL_SECS` is set per engine (`apps/execution-engine/src/settings.rs:464`). A session accepted at 1.8 by an engine with 30 s can be served by an engine with 5 s, which marks provisional leases with 5 s, less than R.
- **Fix:** also refuse to mark when the claiming engine's time-to-live is under the threshold, and test it.

**Minor 3 — One run that cannot be opened fails the whole call.**
- **Where:** `stream-completion.md` step 4 and host item 4.
- **What is wrong:** examples are an unreadable definition or a corrupt row. The host repeats a retryable `INTERNAL` until expiry, so the other 63 leases of the frame are never acknowledged. The claim deliberately skips such a run (`worker.rs:1015-1096`).
- **Fix:** answer `NOT_HELD` for a run that fails for a reason other than the store being unavailable, and continue with the other runs.

**Minor 4 — Folding the binding and permission refusals into `NOT_HELD` loses an audit.**
- **Where:** decision 3.
- **What is wrong:** Heartbeat turns `NotBound` and `Forbidden` into a call status, which `observe` audits as an access denial (`service.rs:380-398`). Folding them into a per-task verdict silently drops that audit.
- **Fix:** keep the call-level `principal.authorize` as a status, and audit a per-task binding refusal as Heartbeat does.

**Minor 5 — Decision 8 says too much: not every withdrawn step of a stopping run is cancelled.**
- **Where:** decision 8, D006-021, and the two failure-table rows about cancellation.
- **What is wrong:** `drain` skips recovery obligations and saga-owned steps (`plan.rs:354-362`). A withdrawn compensation or reconciliation lease of a stopping run goes to `READY` and is leased again, which is correct. Saying "the settle cancels the step" without that qualification is wrong.
- **Fix:** qualify both rows with that exception.

**Minor 6 — The arithmetic of the "acknowledgement slow" alert does not match its stated reason.**
- **Where:** `stream-completion.md`, "Observability", the alert `SagatLeaseAcknowledgementSlow`.
- **What is wrong:**
  - Half of (30 − 6) is 12 s. At a 12 s delay the margin left is 30 − 12 = 18 s, three quarters of it, not half. The margin is halved at R + (time-to-live − R)/2 = 18 s.
  - At a 7 s time-to-live the threshold is 0.5 s, below the healthy 99th percentile of 751 to 889 ms, so the alert would fire all the time.
- **Fix:** correct the formula or the sentence, and give the alert a floor.

**Minor 7 — The evidence does not show what it is cited for (constitution principle VIII).**
- **Where:** the ADR status line; ADR considered option 4; `stream-completion.md` derivation item 3.
- **What is wrong:**
  - The status line says the design was reviewed by four reviewers on 2026-11-21, a day after today. The cited directory `specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/` does not exist.
  - "Three observed cases since feature 037" are the cases ADR-0098 fixes. Its context says the server knew in each of them that "the frame never left the process". They are not losses on the wire, the loss 1.8 addresses; no wire loss has been observed.
- **Fix:** state the review as pending, and describe the frequency of wire losses as unknown.

**Minor 8 — Option 4 is rejected for a property the ADR then recommends.**
- **Where:** ADR considered option 4.
- **What is wrong:** option 4 is rejected because a deadline near 7 s "withdraws healthy deliveries when the database is slowest". The same paragraph recommends lowering the time-to-live to 7 s for faster recovery, which produces exactly that deadline for every lease, and triples the heartbeat rate.
- **Fix:** state the trade-off once, consistently.

**Minor 9 — The codec test in T006-045 has nothing to run against.**
- **Where:** `tasks.md` T006-045.
- **What is wrong:** "a codec of the ADR-0098 build without the cause refuses it" needs an archived ADR-0098 build, and none exists or is planned. If ADR-0098 and ADR-0099 both ship in 0.3.0, no released binary lacks the cause. The decoding argument for the requirement then concerns only intermediate builds of `develop`.
- **Fix:** say this, and keep the requirement for its other reason: every live replica must serve `AcknowledgeTasks`.

**Minor 10 — The activation window is incomplete.**
- **Where:** G006-008 and the failure table.
- **What is wrong:**
  - (a) With a 7 s time-to-live, a lease marked by an engine that knows the requirement can expire under the pass of another engine of the build that has not learned it yet (up to about 12 s, ADR-0082 decision 4). It then lapses into review. The table names `lapsed` only after a removal.
  - (b) It is not stated that every engine of the build serves `AcknowledgeTasks` whether or not it knows of the requirement. If an engine that has not learned of it refused the call, 1.8 hosts would hand leases back.
- **Fix:** state both.

**Minor 11 — `lease_offered_at` is not "the commit time".**
- **Where:** decision 2 and "The provisional mark".
- **What is wrong:** the claim's statements bind the transaction's sampled time, `clock_timestamp()` read after the lock (`execution.rs:313`, bound at 7097). So the offer time precedes the commit, and the attempt-deadline move includes the claim's own transaction time.
- **Fix:** say "the claim transaction's time", or accept and state the difference.

## Nit

1. **The two §6.2 sentences on extra leases disagree.** `sdks/SPEC.md` §6.2 still says that a poll answer carrying more than `max_tasks` "leaves their leases to lapse", next to "a lease the host will not run is handed back". They now contradict each other.
2. **The "forged token" row is wrong for the real lease.** The row "a forged or old token … the lease is withdrawn at its expiry" is inaccurate: the real holder may acknowledge it.
3. **"Also after the first heartbeat" cannot happen.** In the row "A duplicate acknowledgement (same lease, also after the first heartbeat)", a conforming host never acknowledges after its first heartbeat.
4. **Attempt and generation are not checked.** The acknowledgement checks only the token hash, not the request's `attempt` and `generation`. This is equally strong, but should say so.
5. **A late verdict after a hand-back must be ignored.** The host must ignore an `ACKNOWLEDGED` that arrives after it decided to hand the lease back (§6.8 stop, or host item 4). A repeated call should drop the leases already handed back.
6. **The `acknowledged` label inflates.** For leases that were never provisional, every repeat is counted `acknowledged`, never `duplicate`.
7. **"Strict superset" overstates.** "A strict superset of the statements" (considered option 3) should read "more statements", which is what T006-054 asserts.

## Files consulted

- **The design:**
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md`
- **ADRs and principles:**
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/constitution.md`
- **Code:**
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/maintenance.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/secrets.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/scoped.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/scheduler/src/passes.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/plan.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/src/execution/command.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/service.rs`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs`
- **Tests and reports:**
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/integration/claim-transition-efficiency/work_budget.rs` (lines 223-225: 38/2/5, 50/1/11, 47/1/9; the design's figures match)
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/performance/reports/scalability-ha.md` (completion 99th percentile 6 to 103 ms; matches)
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/performance/reports/sdk-streaming.md` (751 to 889 ms; matches)
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/protocol-negotiation.json`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/worker-stream-faults.json`
  - `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/worker-transport-faults.json`
