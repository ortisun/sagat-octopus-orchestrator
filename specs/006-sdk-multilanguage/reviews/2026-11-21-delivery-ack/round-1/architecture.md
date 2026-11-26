# Architecture review: delivery acknowledgement design (ADR-0099, worker protocol 1.8)

## Verdict

The design is not ready for acceptance: three findings are blocking.

The basic safety argument holds for root leases written by writers that know the new columns. The acknowledgement and the withdrawal both take the run lock and both check the lease before writing, so only one of them can commit. Choosing a provisional mark on the step row over the "first heartbeat before the handler" variant is sound for three reasons: no journal event is added, it keeps hand-back available, and the first heartbeat does strictly more work.

Three things fail against the code:

- The acknowledgement transaction, as specified, has to load the run's state.
- The provisional mark is not tied to the lease it describes, so the documented rollback can make the server withdraw a lease that is running.
- A host that never acknowledges creates a loop with no bound.

Several stated advantages and derivations also need correcting before acceptance.

Counts: 3 Blockers, 10 Majors, 11 Minors, 5 Nits.

---

## Blockers

### Blocker 1. Decision 3 cannot be built as written: "open the run as `Heartbeat` does" loads the run's state

- **Location.** ADR-0099, decision 3. `stream-completion.md`, "The server's acknowledgement", steps 1 to 4. `research.md`, D006-014.
- **What is wrong.** The decision says the server opens the run with `begin(tenant, run, Lock::Wait)`, applies the checks of `Heartbeat`, and "MUST NOT load the run's state". In the code these are the same thing:
  - `PgStore::begin` (`crates/infra-postgres/src/execution.rs:1563`) goes to `begin_row` (`execution.rs:282-411`). That function reads the run row `FOR UPDATE`, every step row of the run, the pinned definition, sagas, collections, loops, scope sagas and the chain, and rebuilds `RunState`.
  - The checks named in the design need that loaded state. `ensure_activity_bound` (`crates/application/src/worker.rs:2742-2769`) reads `tx.parts()` to find the node's activity, and `managed_database` reads the definition.
  - Scoped leases open through `begin_current_path` (`crates/application/src/worker/scoped.rs:99-110`), which loads the path state.
- **Why it matters.** The cost argument against variant 2 (D006-011), the line "a strict superset of the statements of option 1", and the planned `ACKNOWLEDGEMENT` budget line all assume a transaction that loads nothing. For a large collection or loop run, loading every step row on each acknowledgement is the dominant cost. As written, a normative MUST conflicts with the mechanism the same decision prescribes.
- **Smallest fix.** Pick one of these and write it into decision 3:
  - (a) Add a store port, for example `RunStore::lock_run(tenant, run, Lock::Wait)`. It takes the continuation-chain lock and then the run row lock, in the same order as `begin` (so the reaper, the withdrawal and the reports cannot deadlock against it), and loads nothing. The binding check then reads the node's activity from the definition cache. Name the port, its lock order, and a test that the acknowledgement's statements contain no `SELECT ... FROM steps WHERE run_id` scan.
  - (b) Drop "loads no run state", keep `begin`, and restate the cost honestly against variant 2: the difference is the transition and the journal append, not the state load.

### Blocker 2. The provisional mark is not tied to its lease, so a rollback breaks the exactness claim

- **Location.** ADR-0099, decision 2 ("Every write that clears or replaces a lease clears both columns") and "Rollback". `stream-completion.md`, "The provisional mark". Task T006-044.
- **What is wrong.** "Provisional and unacknowledged" means `LEASED`, `lease_offered_at` set and `lease_acknowledged_at` empty. Only writers that know the two columns clear them. The ADR's own rollback procedure keeps the new schema (the `down` migration refuses while a mark is present) and runs the ADR-0098 binary over it.
  - That binary's claim, report, reset and withdrawal statements (for example the generic step update at `execution.rs:7036-7079`, `execution.rs:7375`, `execution/path.rs:1229`) do not touch the columns.
  - So a step whose provisional lease ended under the old binary keeps `lease_offered_at` set, and the next lease the old binary writes inherits a stale "provisional and unacknowledged" mark.
  - On roll-forward and a new activation, the lost-lease pass withdraws that lease as `NOT_ACKNOWLEDGED`. It may belong to a 1.7 host whose handler is running. The work then runs again with the same attempt number and idempotency key, and no review is opened: exactly the harm feature 037 exists to prevent.
  - The stale marks also make the `down` migration refuse indefinitely.
- **Why it matters.** It breaks decision 4's guarantee ("a lease the server withdraws was not run") and the architecture principle "a rolled-back reader keeps working" (`docs/engineering/architecture-principles.md`, "Evolving safely").
- **Smallest fix.** Make the mark check itself against the lease. Store the generation of the lease that was offered (`lease_offered_generation bigint`) next to `lease_offered_at`. A lease is provisional only when `status = 'LEASED' AND lease_offered_generation = generation AND lease_acknowledged_at IS NULL`. Any lease written by an older binary has a new generation, so a stale mark no longer counts.
  - Alternative: a `BEFORE UPDATE` trigger in the migration that clears both marks whenever `lease_token_hash` changes or the status leaves `LEASED`/`RUNNING`. It runs whichever binary writes the row, and per `work_budget.rs:32-35` it does not move the top-level statement budget.
  - Either fix also serves as the fitness function for "every write clears the marks" (see Major 7).
  - Add a test: write a lease with the old column list after a mark, then check the lost-lease pass does not withdraw it.

### Blocker 3. A host at 1.8 that does not acknowledge causes an endless withdraw-and-lease-again loop

- **Location.** ADR-0099, decisions 1 and 4. `stream-completion.md`, "The host at 1.8", item 4 ("Any other status: run nothing, log once, hand every lease of the call back"). D006-006 (carried over from ADR-0098: "No cap on withdrawals of one step").
- **What is wrong.** D006-006 said withdrawals cannot repeat without new polling. That reasoning does not carry over here, because a host that fails to acknowledge keeps polling. Cases that loop with no bound:
  - **A proxy refusing the new method.** A gRPC gateway or proxy that does not allow `AcknowledgeTasks` answers `UNIMPLEMENTED` or `PERMISSION_DENIED`. The host hands every lease back, the server withdraws each one (`NOT_STARTED`) without consuming an attempt, and the same session leases them again at once.
  - **A rolled-back binary.** The ADR's rollback runs an old binary while sessions accepted at 1.8 are still alive; their `AcknowledgeTasks` calls get `UNIMPLEMENTED`.
  - **A buggy or hand-written client** (G006-006).
  - **Overload.** Acknowledgements are cancelled before they commit, the leases expire and are withdrawn, and the work is claimed again with no backoff. That adds load exactly when the system is overloaded, which is the argument the design itself uses against a short deadline (option 4).
- **Why it matters.** The run never progresses, fails or opens a review, and the retry budget is bypassed. That is a liveness failure with no ceiling. ADR-0029 decision 6 already set the precedent for this case: `UNIMPLEMENTED` on a newer method makes the host fall back.
- **Smallest fix.** State two rules in the ADR:
  - (a) **Host.** `UNIMPLEMENTED` on `AcknowledgeTasks` makes the host register again offering 1.7 (bounded, logged once). After the hand-back, no other status may lead straight to another poll at the same minor.
  - (b) **Server.** A step withdrawn as `NOT_ACKNOWLEDGED` becomes `READY` with a `next_attempt_at` backoff that grows with consecutive withdrawals of the same step. After a stated count (for example 3), the next one is recorded as a retryable failure with a known outcome (code `NOT_ACKNOWLEDGED`, nothing ran) that consumes the attempt, so the node's retry policy bounds the loop.
  - Rule (b) changes the attempt accounting promised in decision 4, so it must be written in the ADR; it may also need the owner (see "Items for the owner").
  - Add both rules to the failure table and to the scenario `delivery-acknowledgement`.

---

## Majors

### Major 1. Scoped leases (collection items, loop bodies) are not designed

- **Location.** ADR-0099, decisions 3, 4 and 7. `stream-completion.md`, "The withdrawal", which names only `Condition::LeaseLost` and `expire_attempt`. Tasks T006-047, T006-050 and T006-051.
- **What is wrong.** A session at 1.8 is also at 1.5 or later, so it receives scoped leases. The code handles those on separate paths:
  - the heartbeat uses `heartbeat_path` and `begin_current_path` (`worker/scoped.rs:93-145`);
  - the lost-lease pass uses `apply_path_if` and `expire_path_instance` for paths that are not root (`crates/scheduler/src/passes.rs:5633-5676`);
  - failures go through `report_path` (`worker.rs:2357-2369`).
  
  Neither the acknowledgement, nor the withdrawal, nor the timeout skip, nor the capability refusal is specified for these paths. ADR-0098's event fields (`node`, `attempt`, `generation`, `cause`) carry no instance path, though the domain addresses scoped events with `AddressedRunEvent::in_path`.
- **Why it matters.** A lost frame carrying a collection item would still lapse into a review, or would be handled by a path nobody has reviewed. Collections are the newest primitive (this repository's current branch closes root collections).
- **Smallest fix.** Add a "Scoped instances" subsection naming the path variant of each rule and the addressed form of the event (or state that ADR-0098's build provides it). Add a scoped case to the matrices of T006-047, T006-050, T006-051 and T006-053, and a collection item to the scenario `delivery-acknowledgement`.

### Major 2. The acknowledgement time A measures the wrong thing, and the 7-second threshold is mis-derived

- **Location.** ADR-0099, decision 5. `stream-completion.md`, "Why the deadline is the lease expiry", items 2, 3 and 5. D006-016.
- **What is wrong.**
  - A is taken from the 99th percentile of one completion transaction (0.103 s). But one call is "for each run of the call, in request order", one transaction per run. A full frame of 64 leases over 64 runs is 64 transactions in series: 64 × 0.103 s = 6.592 s, so L = 6 + 6.592 = 12.592 s. That is above the 7-second threshold and 42% of the default 30-second lease time-to-live.
  - Each of those transactions waits on `Lock::Wait` behind the run lock. In a fan-out run the run lock is shared by every sibling completion, so queueing for it is the dominant term. The measurement contains none of this, nor network time, admission wait, or host scheduling.
  - The rounding R + ⌈A⌉ = 7 s rounds the figure up to a whole second; it gives no safety margin.
- **Why it matters.** At a configured time-to-live between 7 and about 13 seconds, healthy deliveries of full frames are withdrawn under load. With Blocker 3 unfixed, that becomes the loop. The owner's instruction was "no calculation errors" (OD-2026-11-20-6).
- **Smallest fix.**
  - Define the measured quantity as the server-observed `lease_acknowledged_at - lease_offered_at`, which the histogram `worker_task_acknowledgement_delay_seconds` already records. It contains R and A together.
  - Derive the threshold from its 99.9th percentile at maximum credit (64) against a fan-out run, on the hardware of `scalability-ha`, plus a stated margin.
  - Either bound how many runs one call may touch, or process them with a stated bounded concurrency.
  - State how long an acknowledgement may hold its `Lease`-class admission ticket (`crates/api-grpc/src/limits.rs:500-509`, `548-556`), since it shares that reserve with `Heartbeat` and `CompleteStep`.
  - Make T006-064 measure exactly this.

### Major 3. The lease time-to-live gate is in negotiation only, although the time-to-live is set per process

- **Location.** ADR-0099, decisions 2, 5 and 9. `stream-completion.md`, "Negotiation and activation".
- **What is wrong.** `protocol_ceiling` refuses 1.8 when the engine's own time-to-live is under 7 seconds. But a session accepted at 1.8 by one engine polls any engine. Decision 2 has the claiming engine mark the lease provisional whatever its own `SAGAT_LEASE_TTL_SECS` is, so an engine with a 5-second time-to-live issues provisional leases that healthy hosts cannot acknowledge in time.
  - Decision 9 also has every engine list `delivery-acknowledgement-v1` unconditionally. That contradicts ADR-0082 decision 7: "the capabilities of a process are what its configuration runs".
- **Smallest fix.** An engine lists `delivery-acknowledgement-v1` only when its time-to-live is at least the threshold. The activation then refuses while a live engine is below it, and an engine restarted below it after activation is refused, as ADR-0082 does for `schedules-v1`. As defence in depth, also require the threshold in the claim's marking condition. Add both to T006-046 and T006-047.

### Major 4. Changing what `timeoutMs` means needs the owner, and "at most R" is wrong

- **Location.** ADR-0099, decision 6. D006-019. `stream-completion.md`, failure table, row "The step's attempt deadline passes before the acknowledgement".
- **What is wrong.**
  1. **It is outside the delegation.** OD-2026-11-20-12 records that delivery acknowledgement was put to the owner "because it changes the guarantees of the worker protocol, which the delegation does not cover". What `timeoutMs` measures belongs to the published workflow document (its reference is in `docs/reference/workflow-document.md`). Changing it so that the same published version behaves differently depending on which SDK minor took the lease is the same kind of guarantee change. "Reported to the owner afterwards" is not enough under that reading.
  2. **The bound is wrong.** D006-019 says delivery adds "at most R". The acknowledgement is retried until expiry and may commit at any time before it, so the attempt deadline can move by up to the lease time-to-live (30 seconds by default), not by R = 6 seconds.
  3. **An alternative was not weighed.** A provisional lease whose attempt deadline passes before its acknowledgement could end as an attempt that timed out with a known outcome (nothing ran): it consumes the attempt under the retry policy, opens no review, and keeps today's meaning of `timeoutMs`.
- **Smallest fix.** Correct the bound. Add the alternative to the considered options. Ask the owner (see "Items for the owner"). Add a documentation task for what `timeoutMs` means at 1.8.

### Major 5. The golden-stability evidence cannot be produced as planned

- **Location.** ADR-0099, "Enforced by" ("every earlier golden unchanged"). D006-024. T006-063. `stream-completion.md`, acceptance matrix, row "Six SDKs".
- **What is wrong.**
  - 52 of the 78 scenarios in `tests/contract/sdk-conformance/scenarios/` pin their workers at `"protocolVersion": "1.4"` or `"1.5"`. They never run at 1.8, so "every other scenario passes at 1.8 with its existing golden" is impossible without editing them, and editing them changes what they test.
  - `protocol-negotiation.json`, `worker-transport-faults.json` and `worker-stream-faults.json` set `"leaseTtlSeconds": 3`, below the 7-second threshold, so 1.8 is never offered there.
  - The ADR says every earlier golden stays unchanged, while T006-063 expects the negotiation golden to change.
- **Why it matters.** Golden stability is one of the four advantages claimed for the chosen variant, and this is its only evidence.
- **Smallest fix.**
  - State what is evidence: the 26 unpinned scenarios run at 1.8 by SDK default and keep their goldens (name them), and the pinned ones remain evidence for older minors.
  - Optionally add a harness switch that raises pinned 1.5 scenarios to 1.8, for one extra pass.
  - Say that the three 3-second scenarios run below 1.8 by construction.
  - Change "Enforced by" to "every earlier golden but `protocol-negotiation` unchanged".

### Major 6. The throughput cost has no acceptance bound

- **Location.** ADR-0099, "Consequences" ("not measured yet and must be before the release"). T006-064.
- **What is wrong.**
  - In the transitions scenario each task is in a different run, so the number of runs per call roughly equals the number of tasks. Every task gains one transaction under the run lock and one call, on top of `LEASE` (38 statements, 2 transactions) and `COMPLETION_WITH_A_NEXT_STEP` (50, 1).
  - With Blocker 1 unresolved, that transaction also loads the run.
  - Feature 038 is the context where throughput is being recovered.
  - T006-064 records a number but sets no bound and no consequence for missing one.
- **Smallest fix.** State the bound (for example, no more than a named percentage below 1.7 in the transitions scenario, same machine and day) and what happens if it is missed (1.8 ships without the requirement recorded by default, or the design is revisited). Say whether release 0.3.0 activates `delivery-acknowledgement-v1` by default or leaves it to the operator; that is a product default (see "Items for the owner").

### Major 7. The new rules have no fitness functions

- **Location.** ADR-0099, "Enforced by". `docs/engineering/architecture-principles.md` (no row added). AGENTS.md rule 21.
- **What is wrong.** Three rules the design relies on have no check that fails when they are broken:
  - (a) **Every lease write clears the marks.** At least four statements write leases today (`execution.rs:7036`, `execution.rs:7375`, `execution/path.rs:1229`, `bindings.rs:245`), and the rule is prose only.
  - (b) **Every capability call on a provisional lease is refused.** The five methods are listed by hand. Drafts 020, 021, 025 and 026 will add calls at 1.9 and later; nothing fails if one of them forgets the refusal.
  - (c) **The happy path writes no journal event and loads no run state.**
- **Smallest fix.**
  - For (a), the structural fix of Blocker 2.
  - For (b), a descriptor-driven contract test. It lists every `WorkerService` method whose request carries a lease token, except `Heartbeat`, `CompleteStep`, `FailStep` and `AcknowledgeTasks`, and fails when one of them is not refused on a provisional lease.
  - For (c), the `ACKNOWLEDGEMENT` budget line asserting zero journal and outbox inserts, plus a statement listing with no step scan (Blocker 1).
  - Add the rows to the table in `architecture-principles.md` in the same change.

### Major 8. The build plan has ordering gaps and missing dependencies

- **Location.** `tasks.md`, phase "Delivery acknowledgement", paragraph "Before the first task" and tasks T006-053 and T006-057 to T006-063.
- **What is wrong.**
  - T006-053 must refuse `AppendStepLogs`, which does not exist until T041-7. The plan still lets T006-053 start before T041-7.
  - Minors are cumulative, so an SDK at 1.8 must also implement 1.7 (T041-15 to T041-20, step logs in the six SDKs). T006-057 to T006-062 do not depend on those tasks.
  - The host procedure depends on the six-SDK hand-back of ADR-0098. That work is on `bugfix/withdraw-leases-sdks` (worktree `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-sdks`), which is not named as a precondition. Only the server pull request #260 is.
  - ADR-0098's own acceptance is not a precondition, although ADR-0099 refines it.
  - T006-063 is listed under both "the shared conformance owner" and "the integrator", against "Shared files have one owner" (`stream-completion.md`, "Implementation ownership and integration order").
- **Smallest fix.** Move T006-053's `AppendStepLogs` case after T041-7. Make T006-057 to T006-062 depend on their T041-15 to T041-20 counterparts and on the merge of `bugfix/withdraw-leases-sdks`. Add "ADR-0098 accepted" to precondition (c). Give T006-063 one owner.

### Major 9. The ADR claims reviews that have not happened

- **Location.** ADR-0099, "Decided by": "reviewed by the correctness, security, architecture and design reviewers on 2026-11-21 (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`)".
- **What is wrong.** Today is 2026-11-20, that directory does not exist, and this review is the first of them.
- **Why it matters.** A record that claims evidence that does not exist breaks AGENTS.md rules 11 and 17, and misleads the owner at acceptance.
- **Smallest fix.** Write "the correctness, security, architecture and design reviews are still to run", as ADR-0098 does, and fill in the dates and the directory when they exist.

### Major 10. Cancellation before the handler starts depends on an unstated behaviour of ADR-0098's build

- **Location.** ADR-0099, decision 8. D006-021 ("to be confirmed against its build, task T006-050"). Failure table, row "The run's cancellation is requested before the acknowledgement".
- **What is wrong.** The host is told `cancel_requested = true` and hands the lease back with `FailStep(TASK_NOT_STARTED)`. Today's authority checks refuse reports other than an explicit cancellation acknowledgement on a cancel-requested step: `attempt_authority_fence` requires `allow_cancellation_report || !s.cancel_requested()` (`worker.rs:2827`), and the cancellation exemption is computed only for `error_code == "CANCELLED" && !retryable` (`worker.rs:2402`). If ADR-0098's build keeps that check for `TASK_NOT_STARTED`, the hand-back is refused as `STALE_ATTEMPT`, the lease lapses, and decision 8's "known and immediate" does not hold.
- **Smallest fix.** State it as a requirement on ADR-0098's build ("a `TASK_NOT_STARTED` report on a `LEASED` step is accepted when cancellation is requested"), with a named test in T006-050, rather than "to be confirmed". As a fallback, have the verdict stay `ACKNOWLEDGED` with `cancel_requested` and let the lost-lease pass withdraw the lease at expiry.

---

## Minors

### Minor 1. Option 5 is rejected for an incomplete reason
- **Location.** ADR-0099, option 5. D006-011.
- **What is wrong.** The ADR says a write without the run lock "would let a withdrawal overwrite an acknowledgement it did not see". A withdrawal whose step update also checks `lease_acknowledged_at IS NULL` cannot do that: the row lock and PostgreSQL's re-check under the default isolation level serialize the two writers. The real reason is different: the renewal written by a repeated acknowledgement can race the expiry path, which decides from a read under the run lock and writes through the generic step update, which does not check the expiry.
- **Fix.** Give that reason.

### Minor 2. The dependency between the two requirements lives only in the command-line tool
- **Location.** ADR-0099, decision 9.
- **What is wrong.** `docs/runbooks/instance-registry.md` section 5 documents an emergency removal by SQL (`SELECT * FROM platform_requirement_remove(...)`, with "the feature's own check ... goes here"), which bypasses the command's refusal. This is also a change to ADR-0082's model (requirements that depend on each other), while the ADR says it only "uses" ADR-0082.
- **Fix.** Gate `NOT_ACKNOWLEDGED` on both requirements being recorded. Add the dependency to the runbook's removal procedure. Say "refines ADR-0082" if the check moves into `platform_requirement_remove`.

### Minor 3. `GET /version` would advertise 1.8 when it is not offered
- **Location.** `crates/api-http/src/routes/identity.rs:68`.
- **What is wrong.** It reports `PROTOCOL_MINOR` as the maximum. After T006-048 that reads 1.8 even when the requirement is not recorded or the time-to-live is too short, and `sagat doctor` would then mislead.
- **Fix.** Report the ceiling as currently gated, or document that it is the build's maximum. Add it to T006-048.

### Minor 4. The acknowledgement leaves no evidence once the lease ends
- **Location.** ADR-0099, decision 2.
- **What is wrong.** The marks are cleared when the lease ends. When an acknowledged lease lapses into a review (gaps G006-004 and G006-005), nothing records that it was acknowledged, though constitution principle VI says an operator must be able to explain a run from evidence written with the decision.
- **Fix.** Record the acknowledgement's presence in the expiry's failure evidence, behind the same requirement, or name this as a gap.

### Minor 5. Forged tokens can take 64 run locks per call
- **Location.** `stream-completion.md`, "Wire" and "The server's acknowledgement", step 1.
- **What is wrong.** A worker with forged tokens can make one call take up to 64 run locks in its tenant, blocking other workers' completions. `Heartbeat` allows one per call.
- **Fix.** Before taking any lock, read the matching token hashes in one statement with no lock, and lock only runs with a candidate match. This also makes `NOT_HELD` cheap. It belongs to the security reviewer as well.

### Minor 6. The slow-acknowledgement alert depends on the configured time-to-live
- **Location.** `stream-completion.md`, "Observability", alert `SagatLeaseAcknowledgementSlow`.
- **What is wrong.** The threshold "half of the time-to-live minus R" is 12 s only at the default 30 s. A rule with 12 s hard-coded is wrong at any other setting.
- **Fix.** Export the configured time-to-live as a gauge and compute the threshold in the rule. Add a `promtool` case at a non-default time-to-live in T006-055.

### Minor 7. The new error is not in the contract's error list
- **Location.** `specs/001-platform-foundation/contracts/worker.proto:11-23`. `sdks/SPEC.md` (canonical errors). T006-043.
- **What is wrong.** `FAILED_PRECONDITION` with the reason `TASK_NOT_ACKNOWLEDGED` is not in the proto's documented error list, and no SDK canonical error is named for it.
- **Fix.** Add both in T006-043.

### Minor 8. 1.8 is reserved before the ADR is accepted
- **Location.** `sdks/SPEC.md` §9, register row 1.8. D006-012.
- **What is wrong.** The register rule for the drafts is "takes the next free one when its contract merges". 1.8 is reserved by a design whose ADR is still Proposed.
- **Fix.** State the condition that releases the reservation (removed if ADR-0099 is not accepted), and amend the "next" row so the rule reads the same for everyone.

### Minor 9. Replay and Studio coverage for the new cause are missing
- **Location.** T006-045.
- **What is wrong.** `crates/domain/tests/legacy_replay.rs` (the row "Committed journals keep their meaning") gets no fixture holding `NOT_ACKNOWLEDGED`. The Studio's closed list of event causes is not mentioned.
- **Fix.** Add both to T006-045.

### Minor 10. The withdrawal rule is spread across SQL statements
- **Location.** `stream-completion.md`, "The withdrawal".
- **What is wrong.** The "provisional and unacknowledged" condition is meant to appear in about six SQL statements (claim, acknowledgement, the two branches of the lost-lease query, the capability queries, the `NOT_SENT` check). The decision "withdraw, expire, or skip" belongs in one place.
- **Fix.** Return the marks as typed values from the store (an extended `lease_deadlines_instance`). Decide with one pure application function, unit-tested over every combination. Keep the SQL condition in one macro, as `scoped_timer_metadata_valid!()` is today.

### Minor 11. The host's retry bound compares its own clock with the server's expiry
- **Location.** `stream-completion.md`, "The host at 1.8", item 4.
- **What is wrong.** "Until the latest expiry ... by the host's clock" compares the host's clock with the server's `expires_at_epoch_ms`. A host whose clock runs ahead by more than the time-to-live gives up after the first failed attempt.
- **Fix.** Bound the retries on the monotonic clock from receipt, with the wall-clock expiry as a cap only, as §6.4 already clamps the heartbeat interval.

---

## Nits

- **Nit 1.** `sdks/SPEC.md` §6.2 still says that leases beyond `max_tasks` "lapse" (the base text just before the new paragraph), which contradicts the hand-back paragraph and the 1.8 text. Align it in this change.
- **Nit 2.** `sdks/SPEC.md` §6.8: the new clause makes one very long sentence, and "hands back ... cancels nothing" reads as a contradiction. Split it in two.
- **Nit 3.** An acknowledgement of a `RUNNING` step whose mark is unacknowledged (a host that sent a heartbeat first) still moves the attempt deadline, though the attempt began at that heartbeat. Limit the move to `LEASED`.
- **Nit 4.** "`supports_delivery_acknowledgement`, true once the migration is applied": the existing proofs (`workers.rs:200-206`) are constants of the adapter. Say which this one is.
- **Nit 5.** Skipping the timeout check for provisional leases makes the lost-lease query step over overdue provisional rows on every pass (the index is ordered by `attempt_deadline_at`). The cost is bounded by the leases in flight; note it.

---

## What holds

- **Exactness.** For root leases written by binaries that know the columns, the run-lock argument holds. The acknowledgement checks the token of a `LEASED` or `RUNNING` step, the withdrawal checks a `LEASED`, unacknowledged step, both run under the same lock, and the host acts only on the verdict.
- **Arithmetic.** The arithmetic written in the documents is correct: 30 − 6.103 = 23.897; 23.897 / 6.103 ≈ 3.92; (30 − 6) / 2 = 12. The error in Major 2 is in what A measures, not in the sums.
- **Registration and gating.** Registering 1.8 as cumulative over 1.7 is consistent with ADR-0015 and design 041. Making the activation depend on `lease-withdrawal-v1` is the right direction.
- **Metric labels.** They come from closed lists: `verdict`, `cause`, `outcome`, and `method` with five values.
- **Layering.** Nothing is added to `crates/domain` beyond the new cause value, and no mapper or data-transfer type is introduced.
- **No duplicate work.** No other branch or worktree implements delivery acknowledgement. The branch under review is `docs/delivery-acknowledgement`. Related active work: `bugfix/withdraw-undelivered-leases`, `bugfix/withdraw-leases-sdks` and `feature/041-step-logs-server` (still at `PROTOCOL_MINOR = 6`).

## Items for the owner

1. What `timeoutMs` means for 1.8 leases (Major 4): counted from the acknowledgement, or kept from the claim with a known-outcome timeout.
2. Bounding the withdraw loop by consuming an attempt after repeated `NOT_ACKNOWLEDGED` withdrawals (Blocker 3), if that is judged a change to the retry guarantee.
3. Release coupling: delivery acknowledgement in 0.3.0 now requires T041-7 and all six SDKs at 1.7 (T041-15 to T041-20). If feature 041 slips, so does this feature.
4. The throughput bound and whether 0.3.0 records `delivery-acknowledgement-v1` by default (Major 6).
5. Acceptance of ADR-0098 first, then ADR-0099.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (lines 282-411, 1563-1577, 7036-7079)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (lines 69-90, 1819-1885, 2402, 2742-2769, 2813-2840)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker/scoped.rs (lines 93-145)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/scheduler/src/passes.rs (lines 436-507, 5601-5676)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/maintenance.rs (lines 361-424)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-grpc/src/limits.rs (lines 488-556)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/api-http/src/routes/identity.rs (line 68)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/tests/contract/sdk-conformance/scenarios/ (52 of 78 files pin 1.4 or 1.5; three set a 3-second lease)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md (section 5)
