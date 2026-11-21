# Architecture review, round 3: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**Not ready for acceptance.** Counts: 1 Blocker, 6 Majors, 14 Minors, 5 Nits.

The round-2 Blocker is resolved. The flag on each lease removes the coupling of later minors to the operator's switch and to the 12 s floor. Five of my six round-2 Majors are resolved and one is partly resolved.

Most of the new problems have one cause. The design was checked against a stale copy of Architecture decision 0098. The worktree `/Users/marcelo/Documents/Projects/sagat-wt-delivery-ack` holds Architecture decision 0098 as it stood before its build review:
- its decision 5 does not mention the minor;
- it has no decisions 7 or 8;
- its command is `lease-withdrawal-activate`.

The server branch (`/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases`) has moved on, with Decisions 006-030 to 006-039 and Gap 006-030 dated 2026-11-21:
- `TASK_NOT_STARTED` is honoured only at 1.8 (decision 5, Decision 006-030);
- a cap of four withdrawals per attempt (decision 7, Decision 006-031);
- the requirement is a replay floor, with an engine switch `SAGAT_LEASE_WITHDRAWAL=off` (decision 8, Decision 006-032);
- capability lists in SQL must equal `Capability::ALL` (Decision 006-033);
- one generic command, `control-plane capability-activate <capability>` (decision 4, Decision 006-034).

The design conflicts with three of these, and the Blocker follows the same pattern as the build review's architecture Blocker 2.

I had no shell, so I could not run `git branch -a` or `git worktree list`. A glob across `/Users/marcelo/Documents/Projects` timed out. I read the two sibling worktrees directly (`sagat-wt-withdraw-leases`, `sagat-wt-withdraw-sdks`) and found no copy of this work. Decision numbers do not clash: the server holds 011 to 018 and 030 to 039, the SDK branch 020 to 028, this design 040 to 061.

---

## Status of the round-2 findings

### My report

| Round 2 | Status | Reason |
|---|---|---|
| Blocker 1: the host rule tied to the minor gated every later minor | **Resolved** | `TaskLease.acknowledge_before_start = 19` (ADR decision 1). The server offers 1.8 on build proofs (`stream-completion.md` lines 327-329). The requirement and the 18 s threshold gate only the marking, at the claim. One coupling to later minors is left, in the host's fallback (Major 5), plus an order problem (Major 4). |
| Major 1: acknowledgement after the attempt deadline | **Resolved** | Step 4 (`stream-completion.md` line 388) requires at least 6 s before both `lease_expires_at` and `attempt_deadline_at`, measured by `clock_timestamp()` under the row lock. The interleaving is in Task 006-051 and in the acceptance matrix. |
| Major 2: the hand-back counted as a violation | **Resolved** | Decision 1 exempts the hand-back. Decision 12, Decision 006-053 and Task 006-053 exempt it, count it separately and test it. |
| Major 3: the clarification of feature 037 | **Resolved** | The first clarification is narrowed (1.8, the cap, not acknowledged). The second covers `NOT_ACKNOWLEDGED`, the hand-back and the hand-back after `CANCELLED`. `ATTEMPT_TIMEOUT_BEFORE_START` is gone. Only a wording artifact is left (Nit 1). |
| Major 4: the window after `lease-withdrawal-v1` is removed | **Resolved in substance, text stale** | Architecture decision 0098 decision 8 now makes `lease-withdrawal-v1` forward-only once any journal holds the event. That is stronger than this design's ordering. But the design's rollback step 3 and Task 006-046 describe a removal command that does not exist, and miss the replay floor of the design's own requirement (Blocker 1, Major 1). |
| Major 5: the gate after the SDKs, no dormant state | **Partly resolved** | Task 006-056 now runs before any SDK task, and the capability stays unlisted until Task 006-057. But Task 006-048 (the server offering 8) runs before the gate, so "released if its build stops at its gate" cannot be honoured (Major 4). |
| Major 6: conformance evidence | **Resolved** | Decision 006-058 says the 26 no-minor scenarios prove nothing. The override covers the 1.4 and 1.5 host scenarios with step logs off. The new scenario carries the 1.4 cases. The count is 51 (52 files with workers, less `protocol-negotiation`). A check that the override pass is not vacuous is still missing (Minor 5). |

### Round-2 Minors of mine that matter here

- Minor 1, the store port: **partly resolved**. The run-lock half is gone, but the binding checks outside a run transaction are still unnamed (Minor 1 below).
- Minor 2, parity: **resolved in intent**. Where the test lives is unclear (Minor 4).
- Minor 3, the trigger: **partly resolved**. It is now an invoker function with schema-qualified names, takes the epoch first and gives a typed refusal. The capability names are still hard-coded and there is no dependency table, and the design is now overtaken by `capability-activate` (Major 1).
- Minor 10, the reservation of 1.8: **partly resolved**. The text says "put to the owner", but `docs/owner-decisions.md` has no row for it (Minor 9).
- Minors 4 to 9 and 11 to 13: **resolved**.

### Other reviewers' round-2 findings in my area

| Finding | Status |
|---|---|
| Design Blocker 1 (hand-backs forbidden and punished) | Resolved (decisions 1 and 12). |
| Design Major 1, security Major 2 (the run lock paid for a renewal; a renewal channel) | Resolved: no renewal, no run lock, unmarked leases are never touched. The safety of dropping the run lock now rests on an invariant that no test enforces (Major 6). |
| Design Major 6 (the time-to-live floor in the registry) | Resolved: applied at the claim (Decision 006-046). |
| Design Major 7 (the gate after six languages) | Resolved, apart from Major 4. |
| Correctness Major 4 (an attempt consumed at the attempt deadline) | Resolved: the lease is withdrawn at the earlier deadline and the next claim has its own deadlines. |
| Correctness Major 5 (1.4 goldens under the override) | Resolved: the instance identity is compared separately. |
| Security Minor 5 (the guard) | Resolved as described. |

---

## Blocker

### Blocker 1. Removing `delivery-acknowledgement-v1` lets in a binary that cannot replay the cause `NOT_ACKNOWLEDGED`; the rollback floor exists only in prose

- **Location.**
  - ADR-0099 decision 10 (line 102: "a mark keeps its meaning until its lease ends").
  - ADR-0099 Rollback (line 184: "roll the binary back no further than a build that decodes … otherwise this build").
  - `stream-completion.md` "Rollback", steps 1 and 4 (lines 493-503).
  - Task 006-045 (`tasks.md` line 168: "or its confirmation if the … build added it").
  - Decision 006-059 (2).
  - The cause type on the server branch: `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/event.rs:438-461`. It is a closed type with `NotSent` and `NotStarted` only, and `parse` returns `None` for any other text.
- **What is wrong.**
  - The registry admits a process when it lists every recorded requirement of its kind.
  - Once the operator runs `delivery-acknowledgement-remove`, only `lease-withdrawal-v1` stays recorded. That one stays for good, because it is forward-only under Architecture decision 0098 decision 8.
  - A binary of the Architecture decision 0098 build therefore registers and serves.
  - That binary fails to decode any journal that holds `step.lease_withdrawn` with `NOT_ACKNOWLEDGED`, so every such run fails to load. Its `down` migration is never run on a binary rollback.
  - The floor "this build" is a sentence, and nothing enforces it.
  - Architecture decision 0098's own build review rated exactly this a Blocker (architecture Blocker 2). Decision 006-032 answered it: "a new durable event is forward-only", the removal is refused while a journal holds the event, and an engine switch stops new writes.
  - This design adds a new value to that event's closed field and leaves the hole open.
- **Smallest correction.** Pick one, and write it into decision 10, the Rollback sections and Task 006-045.
  - (a) Preferred: make Decision 006-059 (2) a hard part of precondition (a). The Architecture decision 0098 build must decode `NOT_ACKNOWLEDGED` before it merges, and Task 006-045 becomes a confirmation only. Every binary the registry can then admit decodes the cause, because `lease-withdrawal-v1` is already a forward-only floor, and the rollback floor becomes "the Architecture decision 0098 build".
  - (b) Otherwise, give `delivery-acknowledgement-v1` the same guard Decision 006-032 gives `lease-withdrawal-v1`: refuse its removal while any journal holds the cause. Add an engine-local switch that stops marking new leases and keeps the capability listed. The rollback then becomes "switch off, drain the marks", and the runbook says the activation is one-way once a lease has been withdrawn as `NOT_ACKNOWLEDGED`.

---

## Majors

### Major 1. The registry, command and rollback text diverge from Architecture decision 0098 decisions 4 and 8, and from Architecture decision 0082 decision 7

- **Location.**
  - ADR-0099 decision 10.
  - `stream-completion.md` lines 333-340 and 499.
  - Decision 006-047.
  - Task 006-046 (`tasks.md` line 172).
  - On the server branch: `docs/adr/ADR-0098-…md` lines 34-39 and 50-53, and `docs/runbooks/lease-withdrawal.md` lines 76-93.
- **What is wrong.**
  - (a) Decision 4 of Architecture decision 0098 says the generic `control-plane capability-activate <capability>`, with a closed list of entries, "activates every fleet requirement that has a way to be activated". This design adds two commands of its own, `delivery-acknowledgement-activate` and `-remove`, with no new architecture decision and no reason given.
  - (b) No command removes `lease-withdrawal-v1`: removal is the owner's `platform_requirement_remove`, and decision 8 refuses it once any journal holds the event. So "the command that removes `lease-withdrawal-v1` also refuses while any mark counts" (line 338, Task 006-046) names something that does not exist. Rollback step 3 applies only to a fleet that has never withdrawn anything.
  - (c) `SAGAT_LEASE_WITHDRAWAL=off` (decision 8) is not addressed:
    - an engine with the switch off writes no withdrawal;
    - so a marked lease it should withdraw lapses into a review, which breaks "a mark keeps its meaning";
    - meanwhile it lists `delivery-acknowledgement-v1` "whatever it knows".
    - Architecture decision 0082 decision 7 says "a capability is never listed for what the binary could do and the process will not."
- **Smallest correction.**
  - Add `delivery-acknowledgement-v1` as an entry of `capability-activate`, with a field "requires `lease-withdrawal-v1`". That is the generic home for a dependency, and it gives the typed refusal.
  - Back it up in the guard by reading a small dependency table seeded by the migration, not two literal names.
  - Drop both new commands. Removal follows the runbook's owner function, or under Blocker 1 (b), a switch.
  - State the rule for the withdrawal switch. Either an engine with `SAGAT_LEASE_WITHDRAWAL=off` does not mark leases and does not list the capability (decision 7), or the design names what happens to marks it cannot withdraw.
  - Amend Architecture decision 0082 by an amendment line, as Architecture decision 0083 did. That is the right vehicle, because this extends decision 1's guard and does not contradict decision 2: the trigger reads, and the epoch is already held by the activation function. The amendment should point at Architecture decision 0098's generic command, not at new commands.

### Major 2. The cap of four withdrawals per attempt (Architecture decision 0098 decision 7) and the run of three withdrawals (decision 9) are not reconciled

- **Location.**
  - ADR-0099 decisions 9 and 15 ("this design names it and does not choose it").
  - `stream-completion.md` line 321.
  - Decisions 006-052 and 006-061.
  - On the server branch: decision 7 and Decision 006-031.
- **What is wrong.**
  - The cap has a value (four), and its derivation counts four causes that can end one delivery of an honest attempt: the consumer's call ending, a host stopping, a frame over the credit, a malformed frame.
  - 1.8 adds causes: `NOT_ACKNOWLEDGED`, a lease given up, the verdict `CANCELLED`, a refused acknowledgement, and the fallback.
  - A `NOT_ACKNOWLEDGED` withdrawal is a `step.lease_withdrawn` that carries the attempt, so it counts toward the cap, and "the domain refuses the fifth in the command".
  - The claim's marking condition, the pure function and the SQL macro read only row columns. They never see the cap, which is derived from the journal.
  - So a marked lease can pass its deadline when the domain refuses to withdraw it. Example: four unmarked `NOT_SENT` withdrawals, then a 1.8 claim that marks the lease. The design does not say what happens then; by Architecture decision 0098 it lapses into a review, for work the host promised not to start.
- **Smallest correction.**
  - Redo the derivation of Decision 006-031 with the 1.8 causes. Then choose one:
    - exempt `NOT_ACKNOWLEDGED` from the cap, because the run of three already bounds it, and amend decision 7 while it is still proposed;
    - or have the claim mark only while the attempt's withdrawals are under the cap, and add that input to the pure function, the macro and the parity test.
  - Cite Decision 006-031 by number in decision 15 and in the hand-back table.

### Major 3. Decision 15 restates Architecture decision 0098 decision 5 with fewer conditions, so two normative texts define one server rule

- **Location.**
  - ADR-0099 decision 15 (line 126) and the "Refines" line (line 5).
  - `stream-completion.md` hand-back table (lines 315-319).
  - `sdks/SPEC.md` §6.2 (lines 373-380).
  - Decision 006-061.
- **What is wrong.**
  - Architecture decision 0098 decision 5, as amended in place on its branch, already reserves the code at 1.8. It withdraws only when four things hold: the session's minor reserves the code, the requirement is recorded, the step is `LEASED`, and the cap is not reached.
  - Decision 15 says the server "MUST withdraw" from a 1.8 session when the step is `LEASED`, the lease is not acknowledged and the cap is not reached. It leaves out "the requirement `lease-withdrawal-v1` is recorded", and it leaves out the engine switch.
  - Read literally, a 1.8 hand-back of an unflagged lease on a fleet without `lease-withdrawal-v1` writes `step.lease_withdrawn`. That breaks Architecture decision 0098 decision 4 and its replay floor.
  - The "Refines" line also claims that this decision "makes `TASK_NOT_STARTED` a code that 1.8 reserves", which Architecture decision 0098 already decides.
- **Smallest correction.**
  - Decision 15 should cite Architecture decision 0098 decision 5 for the rule. It should add only what 1.8 changes: "and, when its mark counts, the lease is not acknowledged".
  - Make the table and §6.2 say the same.
  - Remove the reservation from the "Refines" line. Architecture decision 0098 owns the rule; this decision supplies the minor.

### Major 4. The server offers 1.8 (Task 006-048) before the gate (Task 006-056), so "released if its build stops at its gate" cannot be honoured

- **Location.**
  - `tasks.md` Task 006-048 (line 186) before Task 006-056 (line 229).
  - `sdks/SPEC.md` §9, row 1.8.
  - Decision 006-041; ADR-0099 decision 1.
- **What is wrong.**
  - The capability's dormancy is handled. The minor's is not.
  - From Task 006-048 on, `develop` answers 8, and `AcknowledgeTasks` answers `UNIMPLEMENTED` until Task 006-049.
  - If the gate fails, the reservation is "released", but the server already offers the minor. A release cut in that window publishes it, and Architecture decision 0015 forbids taking a published minor back.
  - Gate failure also has a second effect nobody names. Under Architecture decision 0098 decision 5, 1.8 is the only minor at which a hand-back is ever honoured. Gap 006-030 of the server branch is "closed when the delivery acknowledgement build accepts 1.8". If the gate stops the build, that hand-back is never honoured.
- **Smallest correction.**
  - Move the change of `protocol_ceiling` to Task 006-057, together with listing the capability.
  - Until then, test the 1.8 paths with a session whose stored minor the test raises, as Decision 006-030 (c) does.
  - In the gate-failure branch of decision 13 and Decision 006-057, state that Gap 006-030 then stays open. The owner then chooses between a minor that only reserves the code (the SDK rewrite already exists, Decision 006-028) and leaving the gap open.

### Major 5. The host's fallback "register again offering 1.7" ties every later minor to `AcknowledgeTasks` working

- **Location.**
  - ADR-0099 decision 9 (line 97).
  - `sdks/SPEC.md` §6.2 (line 368).
  - `stream-completion.md` "The host at 1.8", item 5 (line 475).
  - Decision 006-048.
- **What is wrong.**
  - Minors are cumulative. A host at 1.9 or later that hits `UNIMPLEMENTED` from a proxy, three refusals or three give-ups drops to 1.7 and loses every feature of drafts 020, 021, 025 and 026 until it restarts.
  - Six SDKs will build this rule, and every later minor inherits it. It is the one coupling to later minors that the flag on each lease did not remove.
- **Smallest correction.**
  - Make the fallback keep the session's minor. Add one additive registration field (for example `RegisterWorkerRequest.declines_acknowledgement`) or a capability string, and the server never marks leases for that session.
  - The claim already reads the session's minor, so this is one more bit read in the same place.
  - If the downgrade is kept instead, say in §9 and in the notes of the four drafts that a fallback drops every minor from 1.8 up.

### Major 6. Nothing checks the invariant that lets the run lock go

- **Location.**
  - ADR-0099 decision 2 (line 53: "No statement writes `lease_acknowledged_at` from a value it read earlier").
  - `stream-completion.md` line 364.
  - Task 006-047.
  - ADR-0099 "Enforced by".
- **What is wrong.**
  - Every run transaction rewrites the step row from the `RunState` it loaded under the run lock, not under the row lock. `upsert_step` (`crates/infra-postgres/src/execution.rs:7362-7405`) binds `status`, `generation`, `cancel_requested` and others as parameters.
  - The acknowledgement now commits without the run lock. If any statement on that path ever writes `lease_acknowledged_at` or `lease_unacknowledged_withdrawals` from a bound value, a run transaction that loaded before the acknowledgement silently clears it.
  - The paths at risk are the three lease statements, the generic update, the scoped and path statements, reset, and the revocation in `bindings.rs`.
  - The re-read of the withdrawal would then see "provisional" and withdraw a lease whose handler is running. The work runs twice, and the journal says it never ran.
  - The matrix of Task 006-047 checks the values each statement writes, not an interleaving. The races of Task 006-051 are the acknowledgement against withdrawals only. "Enforced by" names no check for this invariant.
- **Smallest correction.** Two checks, with a row in the table of `docs/engineering/architecture-principles.md`:
  - A static check in `scripts/check-architecture.py`. Every SQL text in `crates/infra-postgres` and in the migrations that assigns one of the two columns must match an allow-list:
    - `= clock_timestamp()` and `= 0`, only in the acknowledgement module;
    - `= CASE WHEN … THEN NULL ELSE lease_acknowledged_at END` in the claims;
    - `= lease_unacknowledged_withdrawals + 1` after a `FOR UPDATE` re-read;
    - `= 0` in reset;
    - and no `$n` on the right-hand side.
  - A barrier test in `integration_durable_execution`:
    - a run transaction loads the run and is held;
    - an acknowledgement commits;
    - the run transaction then writes that step row through each statement that can touch a `LEASED` row (a cancel request, a heartbeat renewal, the scoped and path statements) and commits;
    - the test asserts `lease_acknowledged_at` is still set, and that the next withdrawal pass decides "acknowledged".

---

## Minors

1. **The store port for the binding checks is not named, and the coverage decision could move into SQL.**
   - Location: Task 006-049; `stream-completion.md` step 3 (line 383).
   - Problem: `ensure_bound` (`crates/application/src/worker.rs:2761-2815`) is generic over a run transaction and takes the activity from `tx.parts()`. The acknowledgement has no run transaction. The coverage decision, `sagat_domain::binding::first_uncovered`, must stay in the application.
   - Fix: name a port, for example `AcknowledgementTx`, with `matched_leases`, `bound_patterns` (taking the binding lock), `current_worker_authorized` and `acknowledge`. The application calls `first_uncovered` on the activity from the pre-read. Do not write a second binding rule in SQL.

2. **The cost is under-counted.**
   - Location: decision 3, Decision 006-043, Consequences.
   - Problem: "About 6 statements" is too low. On the existing paths, a call with one namespace makes:
     - `SET LOCAL` (1);
     - the pre-read (1);
     - `hold_worker_binding` and the pattern read (2; `execution.rs:4805-4825`);
     - the secrets lock and the membership check (about 2);
     - the update (1);
     - a `SELECT … ORDER BY … FOR UPDATE`, if "locked in primary-key order" is to be true, since a single `UPDATE` does not order its locks (1).

     That is 7 to 8 statements. It gives 103 to 104 round trips against 94: 1 − 94/103 = 8.7%, and 1 − 94/104 = 9.6%. That is still inside the 15% gate, but it is not "about 8%".
   - Fix: list the statements in decision 3 and quote 9 to 10%.

3. **`CANCELLED` is decided from the unlocked pre-read.**
   - Location: step 5 (line 393).
   - Problem: a cancellation that commits between the pre-read and the update makes the conditional write miss, and the host gets `NOT_HELD`. The host drops the lease, and the step is cancelled only after the withdrawal at the deadline, up to one time-to-live later.
   - Fix: with the ordered `FOR UPDATE` of Minor 2, decide the verdicts from the locked row.

4. **The parity test needs PostgreSQL.**
   - Problem: `crates/domain/tests/outcome_unknown_parity.rs` compares texts inside the domain crate. A test that compares a SQL predicate with a Rust function over combinations of a row needs PostgreSQL.
   - Fix: name its target (`integration_durable_execution`), and say whether the pure function extends `LeaseDeadlines` (`crates/application/src/execution.rs:993`), as round 2 asked.

5. **The override pass can pass without testing 1.8.**
   - Problem: if a scenario's leases are never flagged (a short time-to-live, or a requirement not recorded), the pass proves only that an unflagged lease behaves as at 1.7.
   - Fix: the harness asserts, for each scenario raised by the override, that at least one lease was flagged and acknowledged (the verdict counter, or the SDK's `acknowledgements` hook), except the three scenarios with a 3 s time-to-live.

6. **Task 006-066 tests a hand-back that release 0.2 never sends.**
   - Problem: released v0.2 SDKs leave dropped leases to lapse (the old §6.2 text; Decision 006-021 on the SDK branch: "all six cut them and left them to lapse"). The task's "their hand-backs answered as ordinary retryable failures" describes nothing they do.
   - Fix: test the case that exists, a v0.2 handler that raises `TASK_NOT_STARTED` (an ordinary failure). Add the Architecture decision 0098 SDK build's hand-back at 1.5 against this server.

7. **The SDK rewrite is specified twice, and the two texts disagree.**
   - Location: `sdks/SPEC.md` §6.5 (line 472: "from `1.8` on"); Tasks 006-059 to 006-064.
   - Problem: Decision 006-028 of the SDK branch already rewrites a handler-raised `TASK_NOT_STARTED` at every minor (that branch's `sdks/SPEC.md` line 481). "From 1.8 on" contradicts it, and the six SDK tasks would build it again.
   - Fix: drop "from 1.8 on", cite Decision 006-028, and remove the rewrite from the SDK tasks (keep the conformance case). Also cite Decision 006-026 (at most eight hand-backs on their way at once) for the 1.8 hand-backs.

8. **Field 19 and the minors of drafts 019 and 031.**
   - Problem: two approved drafts already name `TaskLease` field 19:
     - `specs/019-worker-deployments-dispatch/contracts/worker-protocol.md:57` (`DispatchFacts dispatch = 19`);
     - `specs/031-client-side-encryption/contracts/worker-protocol.md:42` (`PayloadPlan payload_plan = 19`).

     Both also take minors. The notes went only to drafts 020, 021, 025 and 026, and the §9 "next" row lists only those four.
   - Fix: add the same note to drafts 019 and 031 ("field 19 is taken; next free is 20; next minor 1.9"), and list 019 and 031 in the §9 row.

9. **The exception for the reservation is not recorded.**
   - Problem: "put to the owner" (decision 1, §9, Decision 006-041) has no row in `docs/owner-decisions.md`.
   - Fix: add one with the status "Asked".

10. **Decision 006-059 says it changes none of Architecture decision 0098's decisions.**
    - Problem: request (2) changes the cause set of decision 3, and request (4) adds a condition to decision 5.
    - Fix: say they are amendments of a proposed decision, to be made on its branch.

11. **The notes on later minors leave out the SDK prerequisite.**
    - Problem: a 1.9 host must also carry out the whole 1.8 host procedure, because any server may flag a lease. Six SDK builds of delivery acknowledgement therefore come before any later minor in the SDKs.
    - Fix: say so in the notes of drafts 020, 021, 025 and 026 (and 019 and 031).

12. **Stale facts about the server branch.**
    - Problem: the research introduction says the server build records "from Decision 006-011 and Gap 006-004". It also holds Decisions 006-030 to 006-039 and Gap 006-030. Decision 006-030 (c) tests the 1.8 path with a raised stored minor and expects "the delivery acknowledgement build replaces that with a real 1.8 session". Task 006-050 does not list that replacement.
    - Fix: correct the introduction, and add the replacement to Task 006-050.

13. **The "a mark counts" queries have no named access path.**
    - Problem: the removal check, the refusal of the `down` migration and the gauges `worker_leases_marked` and `worker_leases_provisional` all search `steps`, the hottest table. Marks are not cleared when a lease ends.
    - Fix: either clear the marks when the lease ends and add a partial index on `lease_offered_generation IS NOT NULL` (and weigh what it costs heap-only updates), or name the existing index the query uses. Pin the gauge query in the budget.

14. **The gate measures "ready to handler entry" only with a test host.**
    - Problem: the workers of `tests/performance/scalability-ha` are a seventh implementation of the host procedure.
    - Fix: add to Task 006-065 a re-run of `sdk-streaming` at 1.8 that records the result and does not gate.

---

## Nits

1. **Artifacts of the identifier rename.**
   - The prose has broken phrases left by the rename:
     - "gap Gap 006-001" (ADR-0099 Context, lines 13-14);
     - "decisions Decision 006-044, Decision 006-045 …" (`specs/037-compensation-safety/spec.md` line 156);
     - "tasks Task 006-065 to Task 006-067" (`tasks.md` lines 146-147);
     - "the matching SDK task Tasks 006-059 to 006-064" (line 141).
   - Ranges such as "Decisions 006-040 to 006-061" leave the far end where `grep "Decision 006-061"` cannot find it. Write "Decision 006-040 to Decision 006-061".
   - The old forms in tool-read places (task lines, the identifiers of the register, the `# ADR-0099` heading) are correct under `docs/governance/identifier-scheme.md`.
2. **The symbol "R".** "R", "3R" and "R / 2" are used throughout as a symbol for the receive bound. Owner decision 2026-11-20-9 names "R" as an example of what not to write. Write "the receive bound (6 s)" and "three receive bounds (18 s)".
3. **When the reservation is released.** ADR-0099 decision 1 releases 1.8 only "if this architecture decision is not accepted". §9 and Decision 006-041 add "or its build stops at its gate". Align them, after Major 4.
4. **The status line.** "Round 3 is recorded beside them" is written before round 3 has finished. Write "will be".
5. **Draft 031.** Its contract still says "1.7 at the base commit". Minor 8's note covers it.

---

## Fitness functions for the rules this change relies on

| Rule | Enforced by (named in the design) | Gap and smallest addition |
|---|---|---|
| Exactly one of the acknowledgement and a withdrawal commits | Barrier tests, Task 006-051 | none |
| No stale write of `lease_acknowledged_at` or `lease_unacknowledged_withdrawals` | **none** | Major 6: a static check plus a barrier test with an in-flight run transaction |
| A stale mark never counts | Matrix with the old column list, Task 006-047 | none |
| The SQL condition equals the Rust function | Parity test | Name its target (Minor 4) |
| No capability on a provisional lease | Contract test driven by the descriptor | none |
| Capability lists in SQL equal `Capability::ALL` | `scripts/check-architecture.py` (Decision 006-033, server branch) | Cite it in Task 006-044 |
| The feature stays dormant until the gate passes | Acceptance row "unlisted before Task 006-057" | Also gate the minor (Major 4) |
| Replay floor of the cause | **prose** | Blocker 1 |
| Cap and run of withdrawals agree | **none** | Major 2: edge tests across both bounds |
| The override pass exercises 1.8 | **none** | Minor 5: flagged-lease assertion |
| Budget of the acknowledgement | `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` lines | Itemise statements (Minor 2) |
| Bounded telemetry labels, token canaries | Task 006-054, `promtool` cases | none |

---

## What holds

- **Layering.**
  - The domain gains one cause, with no input or output.
  - The application gains one pure decision function, which adapters may call (`sagat-infra-postgres` depends on `sagat-application`).
  - The adapter gains the SQL and the store method.
  - No mapper is added, and no `serde_json` in business rules.
  - The marks are lease data. They are not journaled, which is correct, because the replay never needs them.
- **The variant.** Variant 2 is now compared on what it really adds (a run restore, a transition and a journal event per short task). The chosen variant adds no journal event.
- **Compatibility.**
  - The proto change is additive.
  - Below 1.8, or with no flag, a lease is served exactly as today.
  - `timeoutMs` has one meaning.
  - Released v0.2 workers are unaffected: they never send the code, and a code their handler raises is an ordinary failure below 1.8.
  - The Architecture decision 0098 SDK build at 1.5 gets an ordinary failure for its hand-back, a known outcome.
- **Operability.**
  - Every alert reads a named metric.
  - Admission has its own class, which keeps half of the reserve for heartbeats and reports.
  - The gate has a stated statistic (the median of five interleaved runs).
  - Transient errors are never answered `NOT_HELD`.

---

## Items for the owner

1. **Narrowing of Architecture decision 0098 decision 5** (made in place on its branch). It changes the basis on which the owner answered Owner decision 2026-11-20-12, whose context says the hosts' report of dropped leases "closes the cases the platform can see".
   - Below 1.8 a hand-back is an ordinary failure. It spends the attempt and fails a single-attempt step for good, which is the outcome option 2 of that decision rejected.
   - Gap 006-030 closes only if this build passes its gate and six SDKs ship 1.8.
   - This belongs to the owner's acceptance of Architecture decision 0098. It should be put with that acceptance, and Major 4 says what happens if the gate fails.
2. **Accept Architecture decision 0098 first, then Architecture decision 0099**, with the trust assumption of decision 12 and the two clarifications of feature 037.
3. **The reservation of 1.8**, an exception to Owner decision 2026-11-15-8. It needs a row (Minor 9), and it carries Architecture decision 0098's hand-back as well.
4. **Activation is one-way.** Both requirements become forward-only once a lease is withdrawn. Under Blocker 1 (a) that comes through `lease-withdrawal-v1`; under (b) through this design's own requirement. Rollback becomes an engine switch, not a removal. Release 0.3.0 ships both unrecorded, so an operator must activate both before anything changes.
5. **The 15% bound and its statistic** were chosen as a delegate. They are to be reported.
6. **Release coupling for 0.3.0.** The feature needs Task 041-7 and Tasks 041-15 to 041-20 (all still open in `docs/designs/041-step-logs.md`), both Architecture decision 0098 builds with Decision 006-059 answered, and the gate.

---

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 249-568)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (lines 331-499)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (lines 130-286)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§6.1, §6.2, §6.5, §9)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/037-compensation-safety/spec.md (lines 150-163)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (the stale copy)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (decisions 4, 5, 7, 8)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/specs/006-sdk-multilanguage/research.md (Decisions 006-030 to 006-039, Gap 006-030)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/event.rs (lines 436-462)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/runbooks/lease-withdrawal.md
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-sdks/specs/006-sdk-multilanguage/research.md (Decisions 006-020 to 006-028)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/src/execution.rs (lines 4805-4825, 7206-7445)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/worker.rs (lines 2761-2815)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/application/src/execution.rs (line 993)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/domain/tests/outcome_unknown_parity.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/019-worker-deployments-dispatch/contracts/worker-protocol.md (line 57)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/031-client-side-encryption/contracts/worker-protocol.md (lines 3-7, 42)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/governance/identifier-scheme.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/owner-decisions.md (rows for Owner decisions 2026-11-20-9, 2026-11-20-10 and 2026-11-20-12)
