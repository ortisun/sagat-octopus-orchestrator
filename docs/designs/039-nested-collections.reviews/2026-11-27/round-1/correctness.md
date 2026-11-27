# Correctness review of design 039, round 1 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: 71629427d6efa1f9197683c2b4f9b3c6b8369ad0
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 3, Major 9, Nit 5

1. Blocker — ChildRoot membership needs a continuation chain, which a child started inside a body never has
   Where: docs/designs/039-nested-collections.md:36-38, 110-113, 285-290, 401; crates/infra-postgres/src/execution.rs:1544, 5652-5676; crates/infra-postgres/src/chain_progress.rs:908-963, 1031-1047; crates/infra-postgres/src/execution/worker_capacity.rs:64-77; crates/infra-postgres/src/capacity_finish.rs:178-204; crates/infra-postgres/migrations/20261210000000_capacity_finish_storage.up.sql:32, 39-40; crates/infra-postgres/migrations/20261212000000_scoped_finish_binding.up.sql:20-24
   What (verified in code): `execution_chains`, `chain_segments` and `child_chain_links` rows are written only when the definition has a `continuation_policy` (chainPolicy). `initialize_chain_in_transaction` also needs a deadline. `link_child_chain` accepts only a parent step at the root path (`instance_path_nodes='{}'`). A CHILD_ROOT member row needs a non-null `chain_id`, with a foreign key to `execution_chains`. Take CUJ 1: a body calls a child workflow that has no chainPolicy, or one that has it but sits inside a `forEach`. The join cannot insert the member. If someone made the insert pass, the guard finds no chain and returns `Ok(true)`, so it grants the lease with no possession check. `Finish` and `Vacate` would fail with `Missing` or `BindingMismatch`. The design's "new chain and run rows" (line 286) describe a path that does not exist for these children.
   Why: Success criterion 039-001, CUJ 1 and User story 039-01 all rest on child members. As written, they either cannot be created or are not guarded.
   Correction: pick one. Either every member child gets a chain (lift the root-path restriction in `link_child_chain`, and decide the deadline and `max_segments` for a chain with no policy), or ChildRoot identity is keyed by run, with a matching unique index, guard lookup and command binding. Add a test with a chainless child inside a body that is refused a lease while it is not the holder.

2. Blocker — A loop carries its tenant unit across iterations, so "one group per root loop iteration, released at retirement" double-releases or deadlocks
   Where: docs/designs/039-nested-collections.md:273-277, 291-295; crates/infra-postgres/src/loop_progress.rs:1425-1426, 1558-1565, 2078-2110, 2349-2350, 2424-2431
   What (verified in code): loop entry charges one unit. A checkpoint that continues with `next_activity` neither decrements nor re-checks. The unit passes to iteration k+1 in the same transaction. Only the end, the delay start, the delay fire and the drain change the counter.
   Scenario, nesting `while` loop at limit 1, iteration k owns group Gk:
   - Reading A: the creator also makes G(k+1) on the carried unit. Gk's retiring `Finish` decrements and so does G(k+1)'s, so one charge is released twice. The counter either hits its `>= 0` check (the Finish fails and the sweeper retries forever) or under-counts and admits past the limit. Meanwhile Gk's still-final-pending members and G(k+1)'s members exist at the same time.
   - Reading B: there is no group, because the carry "takes" no unit. Iteration k+1's child is then outside every member, its collection asks for a fresh unit, and the tenant deadlocks (ADR-0030 §14).
   Why: Success criterion 039-001's "a loop that contains a collection" fails either way, and the tests the design plans do not separate the two readings.
   Correction: state loop semantics explicitly. Either the group belongs to the loop scope (one group for the loop, holding the carried unit, with a scoped member per iteration), or the carry is removed for nesting loops (each iteration checks and charges fresh). Add a limit-1 test of a multi-iteration nesting `while` that asserts `active_iterations` after every checkpoint.

3. Blocker — A yield request can strand a lone holder for good
   Where: docs/designs/039-nested-collections.md:101-106, 314-320, 322
   What (verified in the text): `yield_requested_at` is cleared only by `Vacate` and `Acquire`. `Vacate` fires only "while another member has runnable work", and a holder alone is never vacated.
   Scenario: holder H passes the quantum while member B has a READY step, so `RequestYield` commits. B's deadline then ends its run TIMED_OUT. That is a scheduler transition, which possession does not gate. Selection 2 finishes B. H is now alone with the flag set, so no selection applies. The guard refuses every new lease H asks for until H's own deadline fails it.
   Why: Goal "no member starved" fails, and so does progress in a terminal outcome of a sibling.
   Correction: a yielded holder that is quiescent is vacated whether or not another member waits, and `Acquire` then picks by age (possibly H again). Or the sweeper clears the flag when no other member is runnable. Add a test where the yield is requested and then the waiting member times out. The security review's item 6 covers the case where B is paused. This variant is permanent.

4. Major — The unit release names one decrement site out of four, and moves a quota write into `Finish` without amending ADR-0078
   Where: docs/designs/039-nested-collections.md:291-295, 331-332; crates/infra-postgres/src/primitives.rs:2438-2445, 3263-3270; crates/infra-postgres/src/loop_progress.rs:2098-2110, 3065-3072; docs/adr/ADR-0078-capacity-group-lock-order-and-possession-protocol.md:81-83; crates/infra-postgres/migrations/20261029000000_collection_progress.up.sql:1-2
   What (verified): the unit is decremented at:
   - the collection checkpoint;
   - the collection drain (cancellation or failure);
   - the loop checkpoint end;
   - the loop drain.

   The design changes only "the decrement at the iteration's end" in `primitives.rs`. If a drain still decrements and the retiring `Finish` also does, a cancelled nesting iteration releases twice. That is over-admission, which is exactly the cancellation case this rule exists for (line 293).
   Other gaps:
   - ADR-0078 §3 says the commands write no quota row. Neither ADR-0154 nor the lock contract (lines 329-332) adds `tenant_iteration_usage FOR UPDATE` after G, or the `> 0` guard and its failure path.
   - The counter stops matching `count(slot_held)` and the journal projection that existing tests use, for example `tests/integration/workflow-primitives/collection_events.rs:79`.
   Correction:
   - List all four sites.
   - Record on the header that the group owns a unit, so exactly one retiring write decrements it.
   - Amend ADR-0078 §3 in ADR-0154 and add the tenant-usage lock to the command sequences of the contract file.
   - Restate the consistency check as `active_iterations = slot_held iterations without a group + groups not RETIRED that own a unit`, and test it for checkpoint, drain and cancellation.

5. Major — Scope compensation runs after the iteration's group has retired, and the scoped guard has no rule for it
   Where: docs/designs/039-nested-collections.md:336-341, 343-349; crates/scheduler/src/passes.rs:2541-2577; crates/domain/src/execution/scoped_sagas.rs:51-81; crates/infra-postgres/src/execution/worker_capacity.rs:94-107
   What: it is verified that scope compensation (D380) is planned only after every iteration is drained or checkpointed (`active_count == 0`). That is after `FinishPath` has retired the groups of earlier successful iterations. It is a suspicion, not checked, that the compensation steps sit under that iteration's path.
   Scenario: iteration 0 succeeds, its group retires and the unit is released. Iteration 1 fails, the run fails, and compensation of iteration 0 is emitted. A guard modelled on the built ChildRoot rule ("FINISHED-only history" refuses) never leases those steps, so the run never ends. A guard that treats them as "outside every member" runs them without a unit or possession. That contradicts line 293's rationale.
   Correction: state the rule for steps under a finished member's path, together with the unit and limit semantics that go with it. Add a limit-1 test where a successful iteration with a nested body is compensated after a later iteration fails.

6. Major — `Detach` children keep the group and its tenant unit for as long as they run
   Where: docs/designs/039-nested-collections.md:291-294, 338; crates/domain/src/workflow.rs:568-575
   What (verified enum): with `CancelPolicy::Detach` the child keeps running on its own. `FinishPath` waits until "every ChildRoot member admitted under that iteration is final", and the group retires only with its last member. A cancelled parent iteration with one detached long-running child (a monitor, say) holds the unit indefinitely, and at limit 1 the tenant admits nothing more.
   Correction: decide what detach means for membership. Either a detached child leaves the group (and needs a rule for its own unit that does not block admitted work), or detach is refused inside nested bodies at publication. Add a test.

7. Major — A join writes the header, which the design and ADR-0154 leave out
   Where: docs/designs/039-nested-collections.md:285-290; docs/adr/ADR-0154-nested-capacity-membership-is-created-by-the-engine-and-the-sweeper-serves-every-member.md:35-36; crates/domain/src/execution/capacity.rs:217-245, 298-334
   What (verified): `CapacityGroup::join` bumps `revision` and `active_members`. `Finish` retires the group when `active_members` reaches 0. The design's join "locks the group and inserts the member" and says nothing about updating the header. ADR-0154 counts two header writes outside the commands, not three.
   Scenario: if the join only inserts the row, `active_members` stays at 1. The first `FinishPath` or `Finish` then retires the group while child members are still ACTIVE. The retired header is immutable, so those children are refused (`NotHolder`) forever and the unit is released early.
   The security review's item 14 lists this as a Nit; this scenario makes it Major.
   Correction: specify that the join compares and sets `revision` and increments `active_members` under the `NOWAIT` lock. List it in ADR-0154 and in the creator's lock sequence. Test that a join invalidates an in-flight command's expected revision.

8. Major — The lock order in the checked contract contradicts the code
   Where: docs/designs/039-nested-collections.md:367-371; crates/infra-postgres/src/execution.rs:868-878; crates/infra-postgres/src/execution/path.rs:80-89, 119-126; crates/application/src/worker/scoped_claims.rs:79-127, 239
   What (verified): a scoped claim takes the chain (`lock_chain_for_run`), then the run (`lock_path_revision`), then the drain advisory lock, the binding advisory lock, the session and the secret checks, then G. The design lists the drain, binding, session and secret locks before the chain and the run. A checker built from that file either fails against the code, or pushes a change that moves blocking advisory waits ahead of the chain and run locks, which ADR-0030 §18 forbids.
   Correction: write the order the code takes (chain, run, drain, binding, session, secret, G per instance, quota, breaker, database and script). Pass the instance to the guard (a scoped claim leases many instances per transaction).

9. Major — Selection 4 vacates a busy holder at any gap between its steps, so the quantum does nothing
   Where: docs/designs/039-nested-collections.md:101-106, 245-247, 317-318
   What (verified in the text): selection 4 asks only for "quiescent while another member has runnable work". A holder running many short steps is quiescent in every gap between a report and its next lease.
   Scenario: B waits. A tick that lands in such a gap vacates H after one step. B is then vacated the same way. Each step costs a `Vacate` and an `Acquire` plus at least two ticks. H's 1,000 steps of 10 ms take about 1,000 × 1 s instead of about 10 s, and the commands eat the per-tick budget.
   Correction: vacate only a holder whose yield was requested, or a holder that has no runnable work. Add a test that counts transfers per quantum.

10. Major — The fairness and throughput figures leave out `RequestYield`
    Where: docs/designs/039-nested-collections.md:147-152, 164-166, 245-247
    What (arithmetic): one transfer costs up to 1 tick to see the quantum, then the longest step S, up to 1 tick to see quiescence, and 1 tick to `Acquire`. That is Q + S + 3T.
    - Fairness: with three members, the last one served waits up to 2Q + S_H + S_B + 6T. At Q = 1 s and T = 0.5 s that is 5 s + 2S, against the claimed 3Q + 2T + S = 4 s + S. The criterion also has no start instant.
    - Throughput: each contended group needs three commands per quantum (`RequestYield`, `Vacate`, `Acquire`). 128 commands/s ÷ 3 ≈ 42 groups, not 64, before counting Finishes.
    Correction: restate the bound from the mechanism, with a defined start, and say "42 contended groups per replica" (or give `RequestYield` a budget of its own).

11. Major — A stuck VACANT group or a retirement that keeps being refused raises no alert
    Where: docs/designs/039-nested-collections.md:424-431, 447
    What: the only alerts are HELD for more than 5 minutes and joins answered `busy`. A VACANT group whose member never becomes runnable, or whose `Finish` is refused every tick (for example `BindingMismatch`, as in finding 1), holds the unit silently. At limit 1 the tenant halts and nothing fires.
    Correction: add a gauge and an alert for groups that are not RETIRED and have made no progress for N minutes, broken down by state, and for a member whose head is final but which is still ACTIVE after N ticks.

12. Major — Success criteria 039-001 and 039-002 assert "never" and "ever" with no observer
    Where: docs/designs/039-nested-collections.md:156-163
    What: checking states by sampling cannot prove "no group ever has two members with a live lease". A brief overlap between samples passes. That is an assertion on a transient state.
    Correction: name the observer. For example, a test-only constraint trigger on `steps` moving to LEASED that raises when another member of the same group has an unexpired LEASED or RUNNING step. Check it at the end of every race test.

13. Nit — The `FinishPath` proof reads a row that drained iterations do not have
    Where: docs/designs/039-nested-collections.md:337-338; crates/infra-postgres/src/primitives.rs:2408-2425, 3236-3251
    What (verified): `iteration_checkpoints` rows are written only on success. A drained iteration is `collection_iterations.lifecycle='DRAINED'` (and likewise in `loop_iterations`).
    Correction: base the proof on the lifecycle (`CHECKPOINTED` or `DRAINED`).

14. Nit — The `FinishPath` tests omit terminal outcomes that `Finish` accepts
    Where: docs/designs/039-nested-collections.md:340-341; crates/infra-postgres/src/capacity_finish.rs:349-358
    What: `finality` also accepts TIMED_OUT, PARTIAL and COMPENSATION_FAILED. Add them to the test list.

15. Nit — The down-migration refusals stop protecting once receipts are purged
    Where: docs/designs/039-nested-collections.md:386-387; crates/infra-postgres/migrations/20261226000000_capacity_transfer_receipts.down.sql:8
    What: a single-member HELD group has no transfer receipt. That down migration then passes and drops the table and `steps_run_capacity_lease_idx` while live groups exist.
    Correction: refuse while any group or member exists.

16. Nit — Non-functional requirement 039-002 covers claims only
    Where: docs/designs/039-nested-collections.md:142-146, 278-282
    What: resolving the `MembershipContext` is a statement in every collection and loop admission.
    Correction: state that it is skipped by the run row's flag, which `lock_run` already reads.

17. Nit — The fence for a process not live at activation is not stated
    Where: docs/designs/039-nested-collections.md:127-128, 415-416; docs/adr/ADR-0082-the-instance-registry-records-requirements-only-for-the-operator-and-a-refused-process-stays-up-with-a-closed-gate.md:24, 54-58; docs/adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md:44-48
    What (suspicion): ADR-0131 decision 5 needs "a stated harmlessness or a fence for a process not live at the activation". The amendment ADR-0099 makes a silent engine refuse calls that carry a lease token. Whether `Poll` is one of them is not stated.
    Correction: say which rule covers it, and test that a silent engine's `Poll` grants nothing.

Separate from the counts, I confirm independently the following items of `docs/designs/039-nested-collections.reviews/2026-11-27/round-1/security.md`, which the doc at 71629427 does not answer yet:
- item 2: the predicate drops the non-FORWARD steps that stopping runs still lease (`workers.rs:97-98, 114-115`);
- item 4: removal races the lock-free read of the requirement;
- item 5: the `capacity_member` flag fails open on continue-as-new successors;
- item 11: the delete guards on groups and members, and the run purge that does not exist.
