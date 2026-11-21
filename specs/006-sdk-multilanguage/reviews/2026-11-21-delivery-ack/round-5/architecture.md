# Architecture review, round 5: delivery acknowledgement (Architecture decision 0099, worker protocol 1.8)

## Verdict

**There are no Blockers. One Major must be fixed before the owner accepts the second assumption of decision 12.** Counts: 0 Blockers, 1 Major, 9 Minors, 8 Nits.

- **Round 4.** All four of my Majors are resolved. Twelve of my thirteen Minors and all four Nits are resolved. Minor 3 is resolved in substance, but the fix creates a new ordering problem in the plan (Minor 1 below).
- **The timeline fix.** The answer to the design reviewer's durability Major (marks bound to the database timeline) is the right mechanism. It has one hole that the timeline cannot see: a committed transaction lost by a plain crash and restart when `synchronous_commit` is off (Major 1).
- **Most of what is new is at the edges:**
  - the per-kind meaning of the capability;
  - one source for the rules shared with Architecture decision 0098;
  - the probe's fail mode against Architecture decision 0082;
  - an audit for `end_lease_marks()`;
  - which restores are covered;
  - drafts 032 and 035.
- **Size.** The design is getting too big for one decision in one place. The dependency between requirements should become its own short Architecture decision (Minor 9). The rest is cohesive.
- **What must happen before what:**
  - Major 1 before acceptance;
  - Minor 1 before Task 006-043 starts;
  - the others before or during the build.

**Limits of this review.** I had no shell, so I could not run `git branch -a` or `git worktree list`. A glob over the sibling directories timed out. The server branch `/Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases` has no `AcknowledgeTasks`, `acknowledge_before_start` or `sagat_database_timeline` anywhere under `crates/`. I found no duplicate of this work.

---

## Status of the round-4 findings

### My Majors

| Round 4 | Status | Reason |
|---|---|---|
| Major 1: the engine switch made a mark's meaning depend on which engine withdrew it | **Resolved** | Option (a) is applied everywhere it was needed: decision 10, Decision 006-047, "The hand-back at 1.8", the withdrawal table ("not consulted for a lease whose mark counts"), Rollback step 1, the failure table and request (6). There is a test: "an engine with the switch off withdraws a marked lease" (Task 006-050). The capability's meaning is now stated, but only for an engine (Minor 2). |
| Major 2: the lost-lease pass wrote the event without the share-locked read | **Resolved** | Decision 4, "The withdrawals" and Decision 006-055 all name the read through `lease_withdrawal_required()`. That function exists on the server branch: `20270208000000_lease_withdrawal.up.sql:71-77`, `FOR SHARE`. The race has a barrier test with an empty journal in both orders (Task 006-051), the failure table has a row for removing both requirements in a row, and "Enforced by" names it. |
| Major 3: the claim's withdrawal count did not exist | **Resolved** | The cap moved to the withdrawal command, which already gets its count from `lease_withdrawals` (server branch `crates/infra-postgres/src/execution.rs:7715-7733`). The `LEASE` line stays unchanged and the skip reason `attempt_capped` is gone. Considered option 7 records both rejected forms. One cost is unstated (Nit 7). |
| Major 4: the six SDK copies drifted from Task 006-043 on | **Resolved** | Task 006-043 synchronises the copies and their stubs, with the byte-equality tests as evidence. Decision 006-063 records the choice. Decision 006-057 keeps the messages "reserved, not offered" if the gate fails. |

### My Minors and Nits

- **Minors resolved:**
  - Minor 1 and Minor 2: there is no table any more, and the trigger is its own.
  - Minor 4: the variants `DependencyNotRecorded` and `RequiredBy`, with a distinct error text.
  - Minor 5: the entry is added in Task 006-057 with `empty_fleet: false`, and there is a test that the command refuses before then.
  - Minor 6: the allowed forms are written out, with negative tests.
  - Minor 7: one function over a narrow trait.
  - Minor 8: decision 11, Decision 006-063, and notes in drafts 023 and 024.
  - Minor 9: Decision 006-057, Owner question 006-002 and decision 13.
  - Minor 10: the clamp, request (3).
  - Minor 11: the rebase sentence. The edit to step 5 of "Leases that never reach a host" is still in the diff, and the design says it will be made again after the rebase.
  - Minor 12: Task 041-3 in precondition (b).
  - Minor 13: requests (3) and (4) reframed.
- **Minor 3: resolved in substance.** There is now one source of truth (a constant in the trigger, held equal by a check). But the check cannot pass between Task 006-046 and Task 006-057 (Minor 1).
- **Nits 1 to 4: resolved.**

### Other reviewers' round-4 findings in my area

| Finding | Status |
|---|---|
| Design Major 1 (a lost acknowledgement becomes a false "nothing ran") | **Partly resolved.** The timeline covers a promotion and a point-in-time recovery. It does not cover a crash with asynchronous commit (Major 1). The restores it does not cover are named too narrowly (Minor 6). |
| Design Major 2, correctness Major 3, security Minor 4 (the fallback) | **Resolved.** The fallback is bounded (10 minutes, doubling to an hour), counts calls rather than leases, is not triggered by the server's own statuses, and is alerted. |
| Design Major 3, correctness Major 1 (the cap) | Resolved, as my Major 3. |
| Correctness Major 2 (a hand-back after the expiry) | **Resolved** by lifting the expiry condition for a marked provisional lease. One sentence now contradicts it (Nit 1). |
| Security Major 1 (the floor build decides from the session row) | **Resolved** by the clamp, request (3). The server branch does not have it yet: `lease_holder_minor` at `execution.rs:7737` returns the stored minor unclamped. That is correctly a precondition. |
| Security Minor 1 (root `WriteBlob` commits nothing) | Resolved: its check transaction now commits (Decision 006-050). |

---

## Major

### Major 1. A committed acknowledgement can be lost with the timeline unchanged: a crash and restart of the primary under `synchronous_commit = off`, or with `fsync = off`

- **Where:**
  - Architecture decision 0099, decision 2 (the timeline), decision 4 ("where the database can lose one") and decision 12 ("the exactness holds while committed transactions survive. A promotion or a point-in-time recovery starts a new timeline");
  - Decision 006-062 and Gap 006-048;
  - `stream-completion.md`, "The marks", the bullet "The timeline", and step 1 of "The server's acknowledgement";
  - `docs/runbooks/disaster-recovery.md`.
- **What is wrong.** With `synchronous_commit = off`, a `COMMIT` returns before its write-ahead log record is flushed. This can be set for the server, the database or the runtime role, and it is a common tuning that neither the runbook nor the platform forbids. A crash of the primary followed by crash recovery on the same server then loses up to three times `wal_writer_delay` of commits, and the server stays on the same timeline. With `fsync = off` the loss is larger. Step by step:
  1. The host is answered `ACKNOWLEDGED` and starts the handler.
  2. PostgreSQL crashes and restarts.
  3. The mark is provisional again, on the current timeline.
  4. The pass withdraws the lease as `NOT_ACKNOWLEDGED`.

  That is exactly the false "nothing ran" of round-4 design Major 1, reached by a path the timeline does not see. Decision 12 tells the owner the exactness holds "while committed transactions survive", and that a timeline change is what ends a mark. Nothing in the design makes a committed acknowledgement survive a crash.
- **Why it matters.** This is the one guarantee of this decision. The owner is asked to accept an assumption about the database, and this is a gap in that assumption that one line closes.
- **Smallest correction.**
  - In the acknowledgement's single settings statement (step 1), also raise `synchronous_commit` for the transaction when the effective value is `off`. It is a user-settable parameter, so the runtime role can do it per transaction, and no statement is added:

    ```sql
    set_config('synchronous_commit',
      CASE current_setting('synchronous_commit') WHEN 'off' THEN 'local' ELSE current_setting('synchronous_commit') END,
      true)
    ```

    Draft 032 already requires this of its replay paths (Requirement 032-009, item (e)). Only the acknowledgement needs it: a lost withdrawal, claim or capability record is safe or covered by the acknowledgement.
  - The start probe also reads `current_setting('fsync')` and marks nothing while it is `off`. Give it its own skip reason `no_durability`, or count it under `no_timeline`.
  - Name both settings in decision 12 and in `disaster-recovery.md`.
  - Add a test in `integration_durable_execution`. With `ALTER ROLE sagat_app SET synchronous_commit = off`, assert from a test hook in the adapter that the acknowledgement's transaction runs with `local` or stronger. Assert that the probe refuses `fsync = off`. Add both to "Enforced by".

---

## Minors

### Minor 1. The check "the trigger's pair equals the entries' `requires`" cannot pass before Task 006-057, and the dependency is anchored in the apps layer

- **Where:**
  - Task 006-044 (the trigger with its constant pair);
  - Task 006-046 (the check, "with its row in the fitness-function table");
  - Task 006-057 (the entry with `requires`);
  - Decision 006-047; `stream-completion.md`, "Negotiation and activation".
- **What is wrong.**
  - From Task 006-044 the trigger holds the pair (`delivery-acknowledgement-v1` requires `lease-withdrawal-v1`). No activation entry has `requires` until Task 006-057. So the check added in Task 006-046 fails in its own pull request, or it is written to tolerate the gap, and then it checks nothing.
  - Separately, the relation is held equal to `Activatable` in `apps/control-plane/src/capability_activation.rs`. That is a command-line table in the apps layer, and it stands for a rule the database and the application both rely on.
- **Fix.** Put the relation on the capability in the application, for example `Capability::requires()` next to `Capability::ALL`. It is known from Task 006-046, where the capability is already dormant. The activation entry reads it, and the check compares the SQL constant with it, as the check of Decision 006-033 does for the lists. The fallback is to add the trigger's pair in a migration of Task 006-057.

### Minor 2. The capability's meaning is stated for an engine only; a control plane lists it and does one of the three things

- **Where:**
  - decision 10 ("The capability means: the process serves `AcknowledgeTasks`, replays the cause, and withdraws a marked provisional lease");
  - Decision 006-047; `stream-completion.md`, "Negotiation and activation" ("Architecture decision 0082 decision 7: nothing is listed that the process will not do").
- **What is wrong.**
  - `WorkerGrpc` is served only by the engine (`apps/execution-engine/src/lib.rs:799`). The control plane does not run the lost-lease pass.
  - The control plane lists `lease-withdrawal-v1` "since every configuration of this build replays `step.lease_withdrawn`" (server branch `apps/control-plane/src/registry.rs:33`). It will list `delivery-acknowledgement-v1` too, under a meaning two thirds false for it. That is the very sentence of Architecture decision 0082 decision 7 the design cites.
- **Fix.** State the meaning per kind:
  - a control plane replays and shows the cause (event reads and the Studio's closed list);
  - an engine does all three, within what its configuration runs.

### Minor 3. Two sources for the 1.8 rule across Architecture decisions 0098 and 0099

- **Where:**
  - Decision 006-059, requests (4) and (6), which write the 1.8 rule into the text of Architecture decision 0098's decisions 5 and 8;
  - Architecture decision 0099 decisions 9 and 10 and Decision 006-047, which state the same rule and add "Refined by" pointers to Architecture decision 0098.
- **What is wrong.** The same rule would live in two decision records. One of them, Architecture decision 0098, is accepted first and describes a build that has no marks. When they differ, nobody knows which one governs.
- **Fix.** Keep Architecture decision 0098's decisions 5 and 8 about its own build. Add only a "Refined by Architecture decision 0099" pointer to them. Keep the rules of what 1.8 changes in Architecture decision 0099 alone. Request (3), the clamp, stays: it is code that the floor build needs.

### Minor 4. The timeline probe "on each registry heartbeat" can close the gate under Architecture decision 0082 decision 5

- **Where:** `stream-completion.md`, the bullet "The timeline" ("an engine probes it at start and on each registry heartbeat and marks nothing while the probe fails").
- **What is wrong.**
  - Architecture decision 0082 decision 5 says that "a heartbeat that fails … for any other reason (… a permission or function error) MUST close the gate".
  - Decision 8 gives the registry a connection of its own.
  - If the probe runs on the registry connection, or shares the heartbeat's error handling, a database that hides the write-ahead log functions closes the whole process. The design promises only that marks stop.
- **Fix.** Say that the probe runs on the request pool, on the heartbeat's tick but outside its call. Its failure sets `no_timeline` and never affects the gate or readiness. Add a unit test of that fail mode.

### Minor 5. `end_lease_marks()` changes every tenant's lease rows and leaves no record

- **Where:**
  - Task 006-044 (the owner function), Task 006-054 (runbook section 7);
  - the allowed form in `stream-completion.md`, "The marks".
- **What is wrong.** An operator act that changes what leases mean across all tenants has no operator name, no history row and no audit record. Constitution rule 10 says always include auditing. The instance registry sets the precedent: its owner functions write `platform_requirement_history` with who did it.
- **Fix.**
  - Wrap the function in a control-plane command such as `control-plane end-lease-marks --operator NAME`, run as the owner role in a bounded transaction.
  - The command writes one history or audit row with the number of marks it ended, and is idempotent.
  - The runbook calls the command, and the privilege audit lists the function as owner-only.

### Minor 6. The restores that keep the timeline are named too narrowly

- **Where:**
  - decision 12 ("a restore by any other means, such as a logical dump");
  - Gap 006-048; the failure-table row "A restore by a logical dump";
  - Decision 006-062.
- **What is wrong.** A logical dump is not the only restore that keeps the timeline and loses commits:
  - `disaster-recovery.md` section 3 allows "your platform's snapshot tool";
  - a crash-consistent volume snapshot keeps the timeline;
  - a base backup started without `recovery.signal` runs crash recovery, which also keeps the timeline;
  - a managed service's failover may not start a new timeline, or may hide the functions.
- **Fix.**
  - Write "any restore that does not end in archive recovery" in decision 12, Gap 006-048 and the failure table.
  - Make the section 7 step unconditional after every restore. For a failover, the timeline does the work.
  - List in the runbook the topologies the timeline rule is proven on: self-managed PostgreSQL 16 through `infra/dr/drill.sh`.
  - Optionally, stamp the cluster's system identifier as well, if the runtime role may read `pg_control_system()` (verify it). That would end marks mechanically after a logical restore into a new cluster.

### Minor 7. Nothing alerts when the protection is recorded but cannot work

- **Where:** `stream-completion.md`, "Observability" (five alerts) and "The timeline" ("reported at start").
- **What is wrong.** On a database whose runtime role cannot read the timeline, the activation succeeds and no lease is ever marked. The operator believes the protection is on. The only signal is a skip counter.
- **Fix.** Add an alert on any increase of `worker_lease_marks_skipped_total{reason="no_timeline"}` (and `no_durability` from Major 1) while the requirement is recorded. Add a `promtool` case and the matching symptom in `docs/runbooks/sdks.md`.

### Minor 8. Drafts 032 and 035 are missing from the later minors, and draft 032 bears on this design

- **Where:**
  - the context of Architecture decision 0099 ("drafts 019, 020, 021, 023, 024, 025, 026 and 031 will take 1.9 onward");
  - `sdks/SPEC.md` §9, the "next" row.
- **What is wrong:**
  - **Draft 032 takes a minor.** Its contract adds fields in "1.<next>" (`specs/032-multi-region-failover/contracts/worker-protocol.md:32-41`).
  - **Its Requirement 032-009, item (b),** says an answer that acknowledges earlier work without a new write must wait for a quorum commit. That covers the `duplicate` and `recovered` verdicts.
  - **Its items (a) and (e)** forbid acknowledging on a commit whose replication wait was cancelled. The acknowledgement's 1 s `statement_timeout` also applies to its `COMMIT`. So with a slow synchronous standby, an acknowledgement can end up committed on the primary only and still be answered. Today that is safe only because of the timeline; the design should say so.
  - **Draft 032's epoch** is the kind of fence an operator advances that considered option 9 rejected.
  - **Draft 035** expects to take the next minor when it first changes the wire. Its contract still says drafts 020, 021, 025 and 026 claim 1.7 (`specs/035-iceberg-write-steps/contracts/worker-protocol.md:4`).
- **Fix.**
  - Add 032 and 035 to the "next" row and to "Later minors".
  - Add a note in each draft's research, as was done for the others.
  - In 032's note, say three things:
    - its activation runs on a promoted database, so the timeline already ends the marks and no second mechanism is needed;
    - in a quorum cell, the `duplicate` verdict follows its item (b);
    - the acknowledgement's cancelled replication wait is covered by the timeline.

### Minor 9. Size: the dependency between requirements is a decision of its own

- **Where:** decision 10; Decision 006-047; "Refines" (Architecture decision 0082).
- **What is wrong.**
  - Architecture decision 0099 now holds 15 decisions across protocol, storage, registry and durability.
  - The `requires` relation, its trigger and its typed refusals form a general mechanism of the registry. Feature 022 will add entries to it. It refines an accepted decision record and has its own considered options (a table, rejected for a constant).
  - It also has a consequence nobody has written down: every future dependency is a migration that replaces the trigger function, with a `down` that restores the previous body. That is the "rewrite a reviewed guard" risk that round-4 Minor 2 moved away from, now applied to the new guard.
  - The project rule is one decision per record (`docs/engineering/architecture-principles.md`, line 47).
- **Fix.**
  - Move it into a short decision record of its own ("A fleet requirement may require another; the registry refuses either order by a trigger"), with that consequence stated.
  - Architecture decision 0099 then cites it.
  - The timeline should stay in Architecture decision 0099: it is the condition of decision 4.

---

## Nits

1. **Decision 006-059, request (1), says "only those two conditions … are relaxed".** But this build also lifts the expiry check of `attempt_authority_fence` (`lease_unexpired_instance`, server branch `crates/application/src/worker.rs:2926-2928`) for a provisional hand-back. Add "and this build lifts the expiry for a lease whose mark counts (Decision 006-052)".
2. **The replay-floor guard's error text on the server branch is wrong after 1.8.** It says "set SAGAT_LEASE_WITHDRAWAL=off on the engines to stop new withdrawals" (`20270208000000_lease_withdrawal.up.sql:57`), which stops being true for marked leases. Amend it with request (6) before that branch merges, since a migration that has been applied cannot be edited.
3. **Trigger firing order.** Triggers fire in name order, so `platform_requirements_dependency_guard` fires before `platform_requirements_guard`. A hand change then gets the dependency error instead of "change only through …", after the trigger has taken a lock on the epoch row. Name it so it fires after the guard (for example `platform_requirements_guard_dependency`). It then always runs inside an activation function that already holds the epoch `FOR UPDATE`, and it need not lock anything. That is literally "it reads, it does not lock" (Architecture decision 0082 decision 2; `20270113000000_instance_registry.up.sql:15-19`).
4. **"An earlier timeline" should read "another timeline, or none".** The rule is equality. A stored timeline above the current one (a new cluster after an upgrade or a logical restore) and a `NULL` one (after `end_lease_marks()`) must both not count. Put both in the parity test.
5. **Layering of the decision function:**
   - The pure function in `crates/application` receives a PostgreSQL timeline. Pass it as an opaque durability epoch, or as a boolean "the offer is current", and let the adapter map it.
   - State that the gauges and the `down` migration's "a mark counts" use the same macro, timeline included. Otherwise `worker_leases_marked` (Rollback step 2) counts marks that no longer count.
6. **Consequences says "one trigger and one function".** The migration adds three functions (the timeline, `end_lease_marks()`, the trigger's function) and one trigger. Also, `integer` for an unsigned 32-bit timeline needs either `bigint` or a sentence.
7. **The pass's `NOT_ACKNOWLEDGED` withdrawal costs one statement.** It takes one count statement per withdrawal (`lease_withdrawals`). State that cost, which falls on the pass and not on the claim, so that "the `LEASE` line unchanged" is not read as "free".
8. **The drill checks only that the timeline changes.** Add one end-to-end case to the failover drill: a provisional mark at the kill is not withdrawn after the promotion.

---

## Layering

- **The timeline function** (`sagat_database_timeline()`) is infrastructure: an invoker function of the migration, called only inside the adapter's statements, with no extra statement. That is correct. The two gaps are Nit 5 (an opaque value in the application) and Minor 4 (where the probe runs).
- **`end_lease_marks()`** is an owner function with no application surface. It is correct for a disaster-recovery step but needs an audited command (Minor 5).
- **The domain gains only the cause.** The marks stay lease data. `AcknowledgementTx` is an application port with one PostgreSQL adapter. The binding checks are shared through one function. There is no empty mapper and no data-transfer type without semantics.

## The dependency trigger and Architecture decision 0082

- **Decision 1 is kept.** The trigger is set `ENABLE ALWAYS`, it is changed only by migrations, and recording still goes only through the activation functions.
- **Decision 2 is kept in effect.** The trigger runs inside the activation, which already holds the epoch. Nit 3 makes it literal.
- **Decision 7 is kept for an engine but not yet for a control plane** (Minor 2).
- **The single source of truth now has an ordering problem** (Minor 1), and the mechanism deserves its own record (Minor 9).

## Agreement with Architecture decision 0098 on its branch

- **What agrees.** Decisions 5, 7 and 8 are cited correctly. The marked withdrawal reads `lease_withdrawal_required()` as decision 8 requires. The cap stays in the withdrawal command, as in decision 7.
- **What the branch still lacks:**
  - `NOT_ACKNOWLEDGED` (no match under `crates/`);
  - the clamp (`lease_holder_minor` is unclamped);
  - `PROTOCOL_MINOR` is 6.

  Precondition (a) and the rebase sentence handle this correctly.
- **Still to settle:** one source for the rule (Minor 3), and the guard's error text (Nit 2).

## Compatibility and rollback

- **The changes are additive:**
  - nullable columns with no `CHECK`;
  - additive proto fields, clear of the drafts' field numbers, with the copies synchronised;
  - one new cause, decoded by the floor build.
- **Older binaries.** A lease written by an older binary never counts, because of the generation. Older workers and sessions that decline are served as today.
- **Rollback is safe.** The marks are drained first. The floor build clamps the holder's minor, so a 1.8 hand-back there is an ordinary failure. The released v0.2 scenario covers it with and without a served capability. The `down` migration refuses while a mark counts, and drops the trigger and the functions.
- **The emergency stop.** No switch stops the `NOT_ACKNOWLEDGED` withdrawal of an existing mark. Removing the requirement stops new marks, and the existing ones drain within one lease time-to-live plus the pass period. That is acceptable, but it is the owner's to accept (owner item 5).

## Build plan order

- The preconditions are in the right order: (a) the server build with requests (1) to (6), then (b) Task 041-3 before Task 006-043, Task 041-7 before Task 006-057, and Tasks 041-15 to 041-20 before each SDK task.
- The gate (Task 006-056) comes before the minor is offered (Task 006-057), and the activation entry arrives in Task 006-057.
- The runbook step (Task 006-054) comes before activation is possible.
- The only contradiction is Minor 1.

## Fitness functions for the rules this change relies on

| Rule | Enforced by | Gap and smallest addition |
|---|---|---|
| Exactly one of the acknowledgement and a withdrawal commits | Barrier tests, Task 006-051 | none |
| The pass reads the requirement under its lock | Barrier test with an empty journal, Task 006-051 | none |
| A committed acknowledgement survives | The timeline: parity test, pass test, drill reads | A test that the acknowledgement runs with `synchronous_commit` at least `local`, and that the probe refuses `fsync = off` (Major 1); the end-to-end drill case (Nit 8) |
| No stale write of a mark | The static check of allowed forms, with negative tests; the barrier test | Add the allowed form of `end_lease_marks()` to the check's tests |
| A mark keeps its meaning whatever the switch | Task 006-050, "an engine with the switch off withdraws a marked lease" | none |
| The dependency between requirements | Trigger tests in both orders; the equality check | Anchor it on `Capability` so the check can pass from Task 006-046 (Minor 1) |
| The capability is listed only for what the process does | The registry tests of Task 006-057 | A control-plane case once the meaning is stated per kind (Minor 2) |
| The probe never closes the gate | none | A unit test of the fail mode (Minor 4) |
| The protection is not silently absent | none | The `no_timeline` alert and its `promtool` case (Minor 7) |
| The SDK copies equal the canonical file | The byte-equality tests, kept green by Task 006-043 | none |
| The rollback floor | Precondition (a), the replay floor, the clamp, `provisional-lease-cutover` | none once precondition (a) holds |
| The budget | `LEASE` unchanged; `ACKNOWLEDGEMENT` and `FIRST_HEARTBEAT` pinned | none (Nit 7 is wording) |

## Identifiers (constitution 4.7.0)

- Every old form among the added lines is in a place a tool reads: the register ids, the task lines, the heading, a link target.
- New prose uses the new forms and full ranges ("Decision 006-040 to Decision 006-063", "Gap 006-048").
- I found no abbreviation from the refused table among the added lines.

## Items for the owner

1. **Accept Architecture decision 0098 first, as amended on its branch, and merge it.** Requests (2) and (3), the cause and the clamp, are in the branch's code before it merges. Neither is there today.
2. **Accept Architecture decision 0099 with both assumptions of decision 12:**
   - the host's promise;
   - the database's durability, with the `synchronous_commit` and `fsync` conditions of Major 1 and the restores of Minor 6.
3. **Owner question 006-002:** the reservation of 1.8, including what shipping it "with the code reserved" would fix for good.
4. **Supported topologies.** On a managed database that hides the write-ahead log functions, or whose failover keeps the timeline, either nothing is protected (`no_timeline`) or Gap 006-048 applies. Decide whether that is acceptable for 0.3.0.
5. **No emergency stop for existing marks.** Removing `delivery-acknowledgement-v1` stops new marks within one heartbeat, and the existing ones drain within a lease time-to-live.
6. **Whether the dependency between requirements gets its own decision record** (Minor 9). This is my recommendation, and the owner accepts decision records.
7. **The 15% bound and its statistic** were chosen as a delegate. The gate's result is to be reported.
8. **Release coupling for 0.3.0:**
   - Task 041-3 and Task 041-7, and Task 041-15 to Task 041-20;
   - both builds of Architecture decision 0098, with Decision 006-059 answered;
   - the gate.

## Files relevant to this review

- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0099-a-lease-of-protocol-1-8-is-provisional-until-its-host-acknowledges-it-and-a-provisional-lease-that-expires-is-withdrawn.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/stream-completion.md (the timeline, the acknowledgement's step 1, "Negotiation and activation", Observability)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/research.md (Decision 006-047, Decision 006-059, Decision 006-062, Gap 006-048)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/006-sdk-multilanguage/tasks.md (Task 006-044, Task 006-046, Task 006-054, Task 006-057)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/sdks/SPEC.md (§9)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/runbooks/disaster-recovery.md (sections 3, 4, 5, 7)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql (lines 11-19, 98-150)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/032-multi-region-failover/spec.md (Requirement 032-006 to Requirement 032-009)
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/032-multi-region-failover/contracts/worker-protocol.md
- /Users/marcelo/Documents/Projects/sagat-wt-delivery-ack/specs/035-iceberg-write-steps/contracts/worker-protocol.md
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/docs/adr/ADR-0098-a-lease-the-server-never-handed-to-the-transport-is-withdrawn-behind-a-fleet-requirement.md
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/migrations/20270208000000_lease_withdrawal.up.sql
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/infra-postgres/src/execution.rs (lines 7713-7770)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/crates/application/src/worker.rs (lines 75, 2906-2933)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/registry.rs (line 33)
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/control-plane/src/capability_activation.rs
- /Users/marcelo/Documents/Projects/sagat-wt-withdraw-leases/apps/execution-engine/src/lib.rs (line 799)
