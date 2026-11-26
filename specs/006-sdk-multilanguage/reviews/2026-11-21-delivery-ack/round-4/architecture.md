# Architecture review, round 4: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**Not ready for acceptance yet. No finding is a Blocker.** Counts: 0 Blockers, 4 Majors, 13 Minors, 4 Nits.

The round-3 Blocker is answered with the mechanism I preferred (option (a)). Five of the six round-3 Majors are resolved and one is partly resolved. Two of the new Majors are each one sentence plus one test:
- the lost-lease pass has to read the requirement under its share lock;
- the claim's withdrawal count has to come from somewhere real.

The other two need a design choice:
- what the engine switch does to a mark;
- when the six SDK copies of the protocol file follow the canonical one.

**Which version of Architecture decision 0098 the design must follow: the server branch's.** The copy in this worktree is the version from before its build review:
- `docs/adr/ADR-0098-…md` lines 32-38: decision 4 names `lease-withdrawal-activate`, decision 5 has no reservation, and there are no decisions 7 or 8;
- `apps/control-plane/src/lease_withdrawal.rs` and `cli.rs:69` hold the old command;
- `stream-completion.md` line 235 still names `lease-withdrawal-activate`.

The design relies on things that exist only on `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases`:
- decision 5 with the reservation at 1.8;
- decision 7 (the cap);
- decision 8 (the replay floor and the switch);
- Decision 006-030 to Decision 006-039;
- `capability_activation.rs`.

Precondition (a) of the task phase makes that correct for the build. Acceptance has the same order: Architecture decision 0098 as amended must be accepted and merged first (Minor 11).

I had no shell, so I could not run `git branch -a` or `git worktree list`. The server branch has no `AcknowledgeTasks` or `acknowledge_before_start`, and the SDK worktree `sagat-wt-withdraw-sdks` no longer exists. I found no duplicate of this work.

---

## Status of the round-3 findings

### My Blocker and Majors

| Round 3 | Status | Reason |
|---|---|---|
| Blocker 1: the rollback floor existed only in prose | **Partly resolved: no longer a Blocker** | Decoding `NOT_ACKNOWLEDGED` is now a hard precondition of the Architecture decision 0098 build (precondition (a), Decision 006-059 (2), Decision 006-055). That build's forward-only `lease-withdrawal-v1` then makes the registry enforce the floor. The argument needs one more link: "`lease-withdrawal-v1`, which every writer of the cause needs" (ADR line 176). This holds only if the new writer, the lost-lease pass, reads the requirement under the share lock of Architecture decision 0098 decision 8. The design does not say that it does (Major 2). |
| Major 1: registry, command and rollback diverged from Architecture decision 0098 decisions 4 and 8, and from Architecture decision 0082 decision 7 | **Resolved; one new problem** | The design uses the generic `capability-activate` entry, the table `platform_requirement_dependencies` read by the guard in both directions, and no commands of its own. The switch is now named. But the switch rule makes a mark mean different things depending on which engine handles it (Major 1 below). |
| Major 2: the cap of four and the run of three were not reconciled | **Partly resolved** | There is one bound now: the claim marks only below the cap (Decision 006-052), so a marked lease's withdrawal is always inside the cap. The input it relies on, "read from the run state the claim already restored", does not exist (Major 3 below). |
| Major 3: decision 15 restated Architecture decision 0098 decision 5 | **Resolved** | Decision 9 now cites the rule and adds only what 1.8 changes. Decision 15 is now about the host only. "Refines" no longer claims the reservation. Two wording slips are left (Nit 4). |
| Major 4: 1.8 was offered before the gate | **Resolved** | Task 006-048 writes the negotiation with the constant still at 7. Task 006-057 raises it and lists the capability. Decision 13 and Decision 006-057 say that Gap 006-030 then stays open. The same ordering creates a new problem with the SDK copies of the protocol file (Major 4 below). |
| Major 5: the fallback dropped to 1.7 | **Resolved** | `WorkerCapabilities.declines_acknowledgement = 10` keeps the minor. Session replacement keeps the session id, so leases in flight survive the second registration (`crates/application/src/worker.rs:311-314`). A follow-on for the internal runners is Minor 8. |
| Major 6: nothing checked the invariant on the marks | **Resolved** | The static check and the barrier test are named in "Enforced by", in Task 006-047 and in the acceptance matrix. The allow-list leaves out the claim's own write (Minor 6). |

### My round-3 Minors and Nits

- Resolved:
  - Minor 1: the port `AcknowledgementTx`; Minor 7 below adds a refinement.
  - Minor 2: about eight statements, about 10 percent. Minor 2 is now undercut by Major 3.
  - Minors 3 to 7, 10 and 12 to 14.
  - Nits 1 to 4.
- Minor 8: resolved for drafts 019 and 031. Drafts 023 and 024 are missing (Minor 8 below).
- Minor 9: resolved in substance. Owner question 006-002 is in the register. A row in `docs/owner-decisions.md` follows when the owner answers.
- Minor 11: resolved.
- Nit 5: covered by the note to draft 031.

### Other reviewers' round-3 findings in my area

| Finding | Status |
|---|---|
| Design Major 1 and security Major 1 (the hand-back decided from a session row that changes) | Resolved by `lease_hand_back_generation`. A window during a rolling upgrade is left (Minor 10). |
| Design Major 2 (the cap) | As my Major 2: partly resolved. |
| Design Major 3 (the reservation makes the hand-back worth nothing until 1.8; Gap 006-030) | Stated; it is an owner item. |
| Design Major 4, correctness Minors 1 and 9, security Minor 3 (lock order, waiting on rows) | Resolved: binding locks come before row locks, and `FOR UPDATE OF steps SKIP LOCKED` never waits for a row. |
| Correctness Minor 7 (precondition (a) not met by the merged base) | Still true, and correctly a precondition. See Minor 11. |
| Correctness Minor 8 (the deferred flush per row) | Named ("plus the triggers"), itemised in Task 006-055 and measured by the gate. |
| Security Major 2 (a hand-back after a capability was served) | Resolved by `lease_capability_generation`. |
| Security Minor 5 (the dependency per kind; check-then-act) | Resolved: per kind, both orders under the epoch lock. |

---

## Majors

### Major 1. The engine switch makes a mark's meaning depend on which engine's pass reaches the run, which contradicts decision 10, the Rollback and Architecture decision 0082 decision 7

- **Location.**
  - ADR-0099 decision 10 (lines 102-108, "a mark already written keeps its meaning until its lease ends").
  - Consequences, line 159 ("a lease's meaning never changes while it is held").
  - Rollback, line 174.
  - `stream-completion.md` lines 354-357, Rollback steps 1-2 (lines 538-542) and the failure table, line 577.
  - Decision 006-047 (`research.md` around line 477).
- **What is wrong.**
  - A lease is claimed by the engine that serves the worker's stream. It is withdrawn by the lost-lease pass of whichever engine holds the run's shard. These are often different engines.
  - Suppose the claiming engine has the switch on and marks the lease, and the shard holder has `SAGAT_LEASE_WITHDRAWAL=off`. Then the provisional lease "lapses as before, counted `disabled`". For an `Activity`, that is a review of work the host promised not to start.
  - In a fleet where some engines have the switch off, the outcome of a lost frame is therefore decided by shard placement.
  - Rollback step 1 offers the switch as one way to "stop marking". Step 2 then says every marked lease ends "with its meaning unchanged". With the switch off on every engine, every provisional mark in flight lapses instead.
  - The design says such an engine still lists `delivery-acknowledgement-v1` because "it serves the call and replays the cause". That defines the capability more narrowly than what the activation promises: that marks are honoured. Architecture decision 0082 decision 7 says a capability is never listed for what the process will not do.
- **Why it matters.** The one guarantee of this design ("a lease the server withdraws was not run; a marked lease that is lost is withdrawn, not reviewed") becomes conditional on an operator setting that is local to each engine. The design does not state that condition.
- **Smallest correction.** Choose one and write it into decision 10, Decision 006-047, the Rollback and the failure table.
  - (a) Preferred: the switch stops new marks and the causes of Architecture decision 0098 (`NOT_SENT`, `NOT_STARTED`). It does not stop the `NOT_ACKNOWLEDGED` withdrawal of a lease that is already marked. That withdrawal needs only `lease-withdrawal-v1`, which is recorded. This narrows Architecture decision 0098 decision 8 for marked leases, so add it to Decision 006-059 as request (6), to be made on that branch. Define the capability in the ADR as "serves `AcknowledgeTasks`, replays the cause, and withdraws a marked provisional lease at its deadline". An engine with the switch off then honours all three.
  - (b) Otherwise, keep the switch as it is. Then:
    - decision 10 says "except where the withdrawal falls to an engine whose switch is off: the lease lapses";
    - Rollback step 1 offers only the removal of `delivery-acknowledgement-v1` as the way to stop that keeps meaning, and the switch becomes the emergency stop that accepts lapses;
    - the ADR says why listing the capability under the switch is consistent with Architecture decision 0082 decision 7;
    - it goes to the owner as an item.

### Major 2. The lost-lease pass writes `step.lease_withdrawn` without the share-locked requirement read of Architecture decision 0098 decision 8, so the floor argument has a race

- **Location.**
  - ADR-0099 decisions 4 and 5 (lines 71-80), and Rollback lines 175-178 ("which every writer of the cause needs").
  - `stream-completion.md` "The withdrawals" (lines 443-452).
  - Task 006-050 (`tasks.md` line 205) and Task 006-051 (line 215).
- **What is wrong.**
  - Architecture decision 0098 decision 8 (server branch, lines 50-53) says a withdrawal MUST read the requirement under a lock that the removal waits for. It is built as `lease_withdrawal_required()`: `FOR SHARE` on the row (server branch migration `20270208000000_lease_withdrawal.up.sql:67-77`).
  - Every withdrawal path of that build calls it (`crates/application/src/worker/withdrawal.rs:228, 261, 340, 413`).
  - The `NOT_ACKNOWLEDGED` withdrawal is written by a different path: the lost-lease pass (`crates/scheduler/src/passes.rs:5612`, `crates/application/src/maintenance.rs:167`). So it does not inherit the read.
  - The design describes the pass's withdrawal as "as decision 3 of Architecture decision 0098 withdraws". Decision 3 says nothing about the lock.
  - **The race.** Shortly after an activation, no journal holds the event yet. The operator removes `delivery-acknowledgement-v1`, which is allowed, and then `lease-withdrawal-v1`, which is allowed because no event exists and no dependent is recorded.
  - Meanwhile a pass whose engine still believes both are recorded (one registry heartbeat) withdraws a marked lease. If it does not take the share lock, it commits after the removal.
  - Then a journal holds the event while no requirement is recorded. The registry admits a binary below Architecture decision 0098, which cannot replay it. That is the Blocker class of Architecture decision 0098's own review.
- **Smallest correction.**
  - Add to "The withdrawals" and to decision 4: every withdrawal of a marked lease, the pass's included, calls `lease_withdrawal_required()` in its transaction before it writes. When the requirement is not recorded, it writes nothing, and the lease lapses, counted.
  - Add to Task 006-051 a barrier test: the removal of `lease-withdrawal-v1` held between the pass's read and its commit, in both orders, with an empty journal.
  - Add the race to "Enforced by".

### Major 3. The attempt's withdrawal count is not in the run state; the claim needs a read of the journal, and the `LEASE` line changes

- **Location.**
  - ADR-0099 decision 2, line 58 ("read from the run state the claim already restored").
  - `stream-completion.md` lines 386-391.
  - Decision 006-052 (`research.md` line 505).
  - Task 006-047, and Task 006-055 with the acceptance row "Cost" ("`LEASE` … unchanged").
- **What is wrong.**
  - On the server branch, `withdraw_lease` takes `withdrawn_before` from its caller (`crates/domain/src/execution/command.rs:1838-1843`). The reason given there: "The count is the caller's because the state is restored from the projection, which does not keep it."
  - The caller reads it with `lease_withdrawals` (`crates/infra-postgres/src/execution.rs:7713-7733`): one `count(*)` over `run_events` for each step.
  - The claim never calls it today; only the withdrawal paths do.
  - So the marking condition needs a statement the design does not count. With one such statement per claimed step, the `LEASE` line (38 statements) is not "unchanged" at 1.8 with the requirement recorded.
  - The prediction of about 10 percent is also low. One more statement on 104 round trips moves it to about 11 percent, and to more for multi-step claims.
- **Smallest correction.**
  - Name the read in decision 2, in the contract and in the port: one statement per run transaction of the claim, over the steps it would mark (`… WHERE instance_id = ANY($n) AND event_type = 'step.lease_withdrawn' … GROUP BY instance_id, attempt`). Take it only when every other marking condition holds.
  - Compare against `MAX_WITHDRAWALS_PER_ATTEMPT` from the domain, never a literal 4.
  - Pin the extra statement in the `LEASE` line for the 1.8 run of Task 006-055. Keep "unchanged" for sessions below 1.8 and for fleets that are not activated.
  - Redo the prediction in decision 3, in Consequences and in Decision 006-057.
  - Two cheaper alternatives, to be checked against the code before they are chosen:
    - skip the read for a step whose loaded columns show it was never withdrawn;
    - a counter that is lease data, which brings back the column round 3 dropped.

### Major 4. The six SDK copies of `worker.proto` drift from Task 006-043 until Tasks 006-059 to 006-064, and for good if the gate fails

- **Location.**
  - `stream-completion.md` line 266 ("additively (the six SDK copies stay byte-identical)").
  - Task 006-043 (`tasks.md` line 155), which changes only the canonical file.
  - Task 006-059 (line 272) and the tasks "as Task 006-059": "the proto copy and stubs".
- **What is wrong.**
  - SDK tests assert that each copy equals the canonical file byte for byte:
    - `sdks/rust/tests/protocol.rs:1-12`;
    - `sdks/java/sdk/src/test/java/dev/sagatoctopusorchestrator/sdk/SharedContractsTest.java:53`.
  - Between Task 006-043 and each SDK task, which comes after the gate and after Task 041-15 to Task 041-20, those tests fail on `develop`, and in the full run of a release.
  - If the gate fails, the SDK tasks never run. The tree then stays broken, or the canonical file has to be reverted.
  - Design 041 sets the precedent: Task 041-3 copies the contract "into `worker.proto` and its six SDK copies" in the same task (`docs/designs/041-step-logs.md:1418`).
- **Smallest correction.**
  - Task 006-043 runs each SDK's sync script (`sdks/*/scripts/sync-shared.*`, `generate.sh`). It copies the file and regenerates checked-in stubs, with no change to host behaviour, as Task 041-3 does.
  - Tasks 006-059 to 006-064 keep only the behaviour.
  - Decision 006-057 says what happens to the messages if the gate fails: kept, with a comment "reserved, not offered", or reverted before any release is cut.

---

## Minors

1. **The dependency table is not protected as Architecture decision 0082 decision 1 protects the registry.**
   - Location: `stream-completion.md` lines 358-365 ("So the hand SQL … cannot break it either"); Task 006-044.
   - Problem: an owner-only table that has no `ENABLE ALWAYS` trigger can be edited or truncated by hand. Deleting the dependency row then lets `lease-withdrawal-v1` be removed while `delivery-acknowledgement-v1` is recorded. The runbook's promise, "a hand edit … is refused by triggers that hold even for the owner" (`docs/runbooks/instance-registry.md:164-165`), would not cover the new table.
   - Fix: give the table the triggers of `platform_requirement_history` (`execution_reject_mutation` on `UPDATE`, `DELETE` and `TRUNCATE`, `ENABLE ALWAYS`), so that it is changed only by migrations. Test it in Task 006-046.

2. **The design rewrites the body of the guard of Architecture decision 0082 decision 1 instead of adding a trigger.**
   - Location: Task 006-044 ("the new body of `platform_requirements_guard` and its restoration in `down`").
   - Problem: the server build added its replay floor as a separate trigger, `platform_requirements_replay_floor_guard`. Rewriting the security guard of the registry, and restoring its body in a `down` migration, puts a guard that has already been reviewed at risk for no gain.
   - Fix: add `platform_requirements_dependency_guard` as its own `BEFORE INSERT OR DELETE` trigger, `ENABLE ALWAYS`, which the `down` migration drops.

3. **The dependency has two sources.**
   - Problem: the command entry's "requires `lease-withdrawal-v1`" and the seed row of the migration can drift apart.
   - Fix: keep one source. Either a `requires` field on `Activatable` (or on `Capability`), with the seed checked against it by extending the check of Decision 006-033 in `scripts/check-architecture.py`, or the command reads only the table and the entry carries no field. Add the row to the table of `docs/engineering/architecture-principles.md`.

4. **The typed refusal has no type.**
   - Problem: the activation returns `ChangeRefused` (`sagat_application::instances`, used at `capability_activation.rs:299-319`). A dependency refusal needs a variant of its own, for example "dependency not recorded for these kinds" and "a recorded requirement depends on it".
   - Fix: name the variant, the sentence the command shows the operator, and a message or error code for the guard that is distinct from the replay floor's `restrict_violation` text, so that a hand removal reads clearly.

5. **The activation entry exists before the capability is listed.**
   - Problem: Task 006-046 adds the entry and Task 006-057 lists the capability. If the entry has `empty_fleet: true`, as `lease-withdrawal-v1` does, then `--expect-live 0` records the requirement on an empty fleet. Every process of that build is then refused when it starts.
   - Fix: add the entry in Task 006-057, or set `empty_fleet: false`. Test that the command refuses before Task 006-057.

6. **The static check's allow-list leaves out the claim's write.**
   - Location: `stream-completion.md` lines 393-396.
   - Problem: the claim assigns `lease_offered_generation`, `lease_offered_at`, `lease_hand_back_generation` and clears `lease_acknowledged_at`. `upsert_step` also rewrites `LEASED` rows for unrelated reasons, such as a cancel request.
   - Fix: write the allowed form. The claim's assignment is conditioned in SQL on a new lease (the stored `generation` differs from the new one), never on a bound flag alone. Make the check refuse any other form, and give it a negative test for a bound-flag form.

7. **`AcknowledgementTx` copies the steps of `ensure_bound`.**
   - Problem: two copies of the binding rule's lock-and-check sequence can diverge, and this is a trust boundary.
   - Fix: extract the sequence into one function over a narrow trait, implemented by the run transactions and by `AcknowledgementTx`. Add a test that both refuse the same narrowed binding.

8. **The internal runners will rise above 1.8.**
   - Location: decision 11 and Gap 006-043 ("keep their minors").
   - Problem: two drafts will raise internal runners above 1.8:
     - draft 023's script runner needs draft 019's checkpoint minor (`specs/023-script-languages/contracts/shim-protocol.md:38-39`);
     - draft 024's database runner takes "the next minor after 1.6" (`specs/024-data-engines-pipelines/contracts/worker-data.md:4`).

     Both will therefore run at 1.9 or later, where minors are cumulative.
   - Fix: decision 11 says an internal runner at 1.8 or later registers with `declines_acknowledgement = true` unless it implements the procedure. Add the note to drafts 023 and 024 (024 still writes "1.7" for itself).

9. **The gate-failure option "1.8 as the code reserved, the flag never set" fixes the meaning of 1.8 for good.**
   - Problem: under Architecture decision 0015 and constitution principle IV (one meaning per published version), hosts of such a 1.8 never implement the acknowledgement. No later server could flag a 1.8 session, so delivery acknowledgement would need a later minor.
   - Fix: say so in Decision 006-057, Owner question 006-002 and decision 13.

10. **There is a window during a rolling upgrade.**
    - Problem: engines of the Architecture decision 0098 build that are still live decide a 1.8 hand-back from the session row (`lease_holder_minor`, server branch `execution.rs:7735`), without the check for a served capability.
    - Why it is bounded: no lease is marked until every live process lists the capability, so only unmarked 1.8 leases are exposed.
    - Fix: state it as a gap in the compatibility paragraph.

11. **Base and version.**
    - Problem:
      - The design should say in one sentence (in the research introduction) that it follows Architecture decision 0098 as amended on its branch.
      - It should be rebased onto `develop` after that branch merges, and before acceptance. Until then, the in-tree references to decision 5's reservation, decision 7 and decision 8 point at text that does not exist in this tree.
      - The edits to steps 5 and 6 of "Leases that never reach a host" (`stream-completion.md` lines 237-245) overwrite lines that the server branch rewrites (its lines 242-249). They will conflict, and they should be made after the rebase.

12. **Precondition (b) is missing the contract task of design 041.**
    - Problem: Task 041-3 (the 1.7 contract in `worker.proto` and `worker.baseline.json`) has to merge before Task 006-043. The two edit the same file and baseline, and field numbering depends on it. Only Task 041-7 is ordered, and only before Task 006-057.

13. **Decision 006-059 (3) and (4) are framed as requests of the Architecture decision 0098 build "before it merges".**
    - Problem: the columns they need (`lease_hand_back_generation`, `lease_capability_generation`) exist only in this build. The code is Task 006-050 and Task 006-052.
    - Fix: what that branch must do is amend the text of its decision 5 (the minor is read from the lease, not from the session). Say that, and leave the code in this build.

## Nits

1. `docs/adr/README.md` new line: "refines Architecture decision 0029, 0098, 0082 and 0015". Write each in full form, so that `grep "Architecture decision 0098"` finds it.
2. The note in draft 019 (`specs/019-worker-deployments-dispatch/research.md` line 346) says the draft names "`TaskLease` field 19". It names 19 to 22, as the Wire section says. Drafts 019 and 031 both claim `TaskLease` 19 and `WorkerCapabilities` 6. That clash is between them, not with this design, but it is worth a sentence to the coordinator.
3. The "amendment line" for Architecture decision 0082 is to be added at acceptance. Rule 2 of the ADR README says an accepted decision is not edited in substance, so call it a "Refined by" pointer. Architecture decision 0098 (branch) says it amends Architecture decision 0082, but its branch has no such line either; add both.
4. Decision 9 attributes "the engine's switch" and "the cap of four" to Architecture decision 0098 decision 5; they are decisions 8 and 7. `sdks/SPEC.md` §6.2 (line 379) leaves out the switch from its summary of the rule. Either cite decisions 5, 7 and 8, or say "the conditions of Architecture decision 0098".

---

## Layering, compatibility, identifiers

- **Layering holds.**
  - The domain gains only the cause, through the server build. The marks are lease data, never journaled.
  - The decision function is in the application, extends `LeaseDeadlines`, and has a parity test against the SQL macro in `integration_durable_execution`.
  - `AcknowledgementTx` is an application port with a PostgreSQL adapter. The lease identity type of `Heartbeat` is reused, so no mapper is added.
  - The dependency table is registry infrastructure, read by the command through the adapter, as the census is.
  - The gaps: the refusal type (Minor 4) and the shared binding checks (Minor 7).
- **Architecture decision 0082.**
  - Decision 2 is kept: the dependency read in the activation reads and does not lock, and both orders serialise on the epoch.
  - Decision 1 needs Minors 1 and 2.
  - Decision 7 needs Major 1.
- **Compatibility.**
  - The proto change is additive, the field numbers are clear of the drafts, and 1.8 includes 1.7.
  - Below 1.8, a session that declines, and an unflagged lease are all served as today.
  - Released v0.2 workers are unaffected. The rollback floor is enforced once Major 2 is fixed.
  - Migrations: the columns are additive, there is no `CHECK` on the marks, the capability checks are added `NOT VALID` and then validated, and the `down` migration refuses while anything depends on it.
- **Identifiers (constitution 4.7.0).** The new prose uses the new forms and full ranges ("Decision 006-040 to Decision 006-061"). The places tools read keep the old forms: task lines, register ids, the `# ADR-0099` heading. I found no broken artifacts left by the rename. Only Nit 1 remains.

## Fitness functions for the rules this change relies on

| Rule | Enforced by (named) | Gap and smallest addition |
|---|---|---|
| Exactly one of the acknowledgement and a withdrawal commits | Barrier tests, Task 006-051 | none |
| No stale write of a mark | Static check plus barrier test, Task 006-047 | Allow-list form for the claim (Minor 6) |
| The rollback floor | Precondition (a), the replay-floor trigger of the server build | The pass's share-locked requirement read, with a race test (Major 2) |
| The cap of four agrees with the marks | Matrix "three and four withdrawals" | Name the count's source and pin its statement (Major 3) |
| A mark keeps its meaning | Failure table | Decide the switch rule (Major 1); a test with the claiming engine switch-on and the withdrawing engine switch-off |
| The dependency between requirements | Trigger tests in both orders, Task 006-046 | Immutability of the table (Minor 1); seed equals the Rust entry (Minor 3) |
| Capability lists in SQL equal `Capability::ALL` | Check of Decision 006-033, cited in Task 006-044 | none |
| SDK protocol copies equal the canonical file | Existing SDK tests | Keep them green: sync in Task 006-043 (Major 4) |
| The minor and the capability stay dormant until the gate | Acceptance row "Gate" | A test that `capability-activate` refuses before Task 006-057 (Minor 5) |
| The override pass exercises 1.8 | Assertion that a flagged lease was acknowledged | none |
| The budget of the acknowledgement | `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` lines, 64-lease itemisation | The `LEASE` line at 1.8 (Major 3) |
| Bounded labels, token canaries, `skip_debug` | Task 006-052 and Task 006-054 | none |

## Items for the owner

1. **Accept Architecture decision 0098 first, as amended on its branch**: decisions 5, 7 and 8, and Decision 006-030 to Decision 006-039. Merge it into `develop` before Architecture decision 0099 is accepted. It carries Gap 006-031: below 1.8 a hand-back spends the attempt, and a step with one attempt fails for good.
2. **Accept Architecture decision 0099**, with the trust assumption of decision 12 and the two clarifications of Requirement 037-006.
3. **Owner question 006-002, the reservation of 1.8.** Include what the gate-failure option means: a 1.8 shipped "with the code reserved" can never carry the acknowledgement (Minor 9).
4. **The engine switch.** If the design keeps option (b) of Major 1, the owner accepts that `SAGAT_LEASE_WITHDRAWAL=off` on one engine turns lost marked leases into reviews.
5. **Activation is one-way in practice.** `lease-withdrawal-v1` is forward-only once any lease has been withdrawn, so the binary floor becomes the Architecture decision 0098 build for good. Release 0.3.0 ships both requirements unrecorded.
6. **The 15 percent bound and its statistic** were chosen as a delegate. The gate's result is to be reported.
7. **Release coupling for 0.3.0.** The feature needs:
   - Task 041-3 and Task 041-7, and Task 041-15 to Task 041-20;
   - both Architecture decision 0098 builds, with Decision 006-059 answered;
   - the gate.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (lines 235-245, 251-612)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (lines 420-590)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (lines 130-300)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§5, §6.1, §6.2, §9)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (the copy from before the review)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md (the version to follow)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/specs/006-sdk-multilanguage/research.md (lines 428-536)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/domain/src/execution/command.rs (lines 1831-1870)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs (lines 7713-7745)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker/withdrawal.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/capability_activation.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (lines 98-168)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/instance-registry.md (section 5)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/rust/tests/protocol.rs
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/java/sdk/src/test/java/dev/sagatoctopusorchestrator/sdk/SharedContractsTest.java
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/designs/041-step-logs.md (line 1418)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/023-script-languages/contracts/shim-protocol.md (lines 38-39)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/024-data-engines-pipelines/contracts/worker-data.md (line 4)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/019-worker-deployments-dispatch/contracts/worker-protocol.md (lines 34, 57-60)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/governance/identifier-scheme.md
