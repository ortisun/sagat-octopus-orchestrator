# EGM design critique of design 039, round 1 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: d2ec55e796d5cc3f686795f04c721930d0598f57
Reviewer: egm-design
Note: the reviewer's output as returned, with this header added and machine paths made relative. Stages 5 to 8 of the
EGM review (the design critique with the zoom-out and comprehension lenses) folded into one run of the `egm-design`
agent of the accepted EGM plugin, with no conversation history. It read the revision that answers the round-1
security and correctness reviews.

Counts: Blocker 2, Major 8, Nit 7

1. Blocker — Phase 1 of removal stops every nested run, so phase 2's drain check can never pass
   Where: docs/designs/039-nested-collections.md:106-111, 154-164, 348-351, 551-563; docs/designs/053-rolling-upgrades.md:286-290, 331-337
   What: Technical Plan 9 says that in phase 1, `nested_collections_required()` answers no only to publication and activation, "while runs of already active nesting versions still create groups". Design 053 does not work that way. Form (a) answers one thing, whether the row is `active`. Design 053's table also makes `ServingGate::uses` answer "inactive: no new use" for a `draining` row. So in phase 1:
   - every creator call and every join gets `NESTED_COLLECTIONS_INACTIVE` and retries each tick, forever;
   - every engine's gate answers no, so Requirement 039-003 refuses every member lease.

   In-flight nested runs never become final, so the phase 2 check ("no group not `RETIRED`, no run of a nesting version not final") never passes. Two more gaps: nothing stops new runs of an already active nesting version from starting during phase 1, and a run started after phase 2 retries `INACTIVE` with no end. The only alert is for `busy`, not `inactive`.
   Why: removal, which Requirement 039-011 and Owner decision 2026-11-27-442 rely on, cannot finish. Starting it halts all nested work at once, and the only way out is `capability-undrain`.
   Correction:
   - Choose and state the drain rule. For example, phase 1 refuses new nesting runs and activations; creator calls on behalf of runs already started still pass; `ServingGate::uses` stays true for member leases while the row is `draining`. That needs either a fence function that is not plain form (a) or an explicit amendment of design 053.
   - Add new runs of nesting versions to the fenced writes.
   - Alert on `capacity_member_join_total{outcome="inactive"}`.
   - Add a test: drain with a nested run in flight, and that run finishes.

2. Blocker — A grouped root loop with a delay retires its group in the middle of the loop
   Where: docs/designs/039-nested-collections.md:96-97, 118-124, 139-142, 180-185, 357-362, 407-409; crates/infra-postgres/src/loop_progress.rs:2098-2110, 2349-2350, 2424-2431
   What: At a delay, iteration k is already `CHECKPOINTED`, and iteration k+1's member does not join until `fire_delay`. During the delay, k's member is the group's only active member. Its iteration is final, so selection 1 calls `FinishPath` on it, and Requirement 039-005 says the last finish retires the group. The `owns_unit` decrement then releases the loop's unit. When `fire_delay` runs, it "does not charge again", and the Requirement 039-008 trigger refuses a member of a `RETIRED` group. The loop then either stalls or runs without being charged. The sentence "the group retires when the loop has ended or drained and every member is final" (line 362) has no mechanism behind it: neither the built `Finish` nor the `FinishPath` proof looks at the loop scope's lifecycle.
   Why: the shape cannot work as written. It breaks the unit invariant of Requirement 039-016 for a common case (a `while` loop with a delay).
   Correction: make `FinishPath` refuse to finish the last member of a loop group while the loop scope is `RUNNING` or `DELAYING`. Better, adopt finding 8 so that a loop has one member. Add the loop scope lifecycle to the `FinishPath` proof and to selection 1. Add a limit-1 test of a nesting `while` with a delay that asserts `active_iterations` before and after `fire_delay`.

3. Major — A child's join can be starved by its holder's own claims, the same problem that moved the yield off the header
   Where: docs/designs/039-nested-collections.md:115-116, 369-378, 573-574, 625; docs/adr/ADR-0075-worker-leases-retain-child-root-possession.md:65-68
   What: A join takes the group header `FOR UPDATE NOWAIT` and gives up on `Busy`. Every holder claim keeps the header locked through its lease commit, including blocking advisory-lock waits. The security review's item 7 showed that under many pollers the header is held almost continuously, and the design itself rejects a header yield because it would be "almost always `Busy`" (line 625). The same holds for a join. In the worst case:
   - the holder has 256 inner iterations, each starting a child;
   - every join gets `Busy`;
   - no child becomes a member, so no other member is runnable;
   - so no yield is ever requested.

   The only response in the design is an alert after 5 minutes.
   Why: the child workflows of a busy holder (CUJ 1) may never start. That is a starvation path the goal "no member starved" does not cover, because the waiting child is not yet a member.
   Correction: give pending joins a guaranteed way through. One option: after N `Busy` answers, the sweeper inserts a yield row for the group (selection 3 extended to pending joins), so the holder stops taking new leases and the header frees up. Add a held-load test with 64 holder pollers and a joining child.

4. Major — `last_vacated_unleased_at` depends on a fact nothing records
   Where: docs/designs/039-nested-collections.md:128-130, 416-419, 667-671
   What: `Vacate` sets the column "of a member that was granted no lease since its `Acquire`". Nothing records a lease grant per acquisition:
   - the guard writes nothing, and Requirement 039-004 forbids it from writing the header;
   - `steps` has only `lease_expires_at` and a general-purpose `updated_at`;
   - scoped instances are not discussed.
   Why: the skip rule closes the security review's item 9 (a member that no worker can serve keeps taking possession back). As written, it cannot be built.
   Correction: name the source. Either the guard sets a column under the `SKIP LOCKED` lock (then add it to Requirement 039-004's list of writers, to the grants and to the lock file), or define an indexed query over facts that exist. Test both an "acquired, leased, vacated" case and an "acquired, never leased, vacated" case.

5. Major — The header has no root run or namespace column, which the shard, the purge and the receipt trigger all need
   Where: docs/designs/039-nested-collections.md:139-142, 145-148, 399-402, 506-512, 667-678; crates/infra-postgres/migrations/20261210000000_capacity_finish_storage.up.sql:4-24
   What: `capacity_groups` has only `tenant_id`, `group_id` and the possession columns. The insert trigger must derive `shard` "from the root run". `capacity_purge_retired(tenant, namespace, root_run)` must find "the retired groups of that root run". The receipt trigger must test that "the group's root run row no longer exists". The first member is inserted after the header, so the header trigger cannot read the root run from it. Task 039-2 adds `held_since`, `owns_unit` and `transfer_count`, but no `root_run_id`, `namespace_id` or `shard`.
   Why: as specified, the migration cannot carry three of the design's own rules.
   Correction: add `namespace_id` and `root_run_id` to the header, with a foreign key to `runs` and the index the purge and the receipt trigger need. Derive `shard` from `root_run_id` (the shard is `Shard::of(run)`) or store it. List all of this in Task 039-2 and in the insert trigger of Requirement 039-008.

6. Major — The built commands' write set changes, but neither the tasks nor the ADRs say so
   Where: docs/designs/039-nested-collections.md:143-149, 416-419, 515-518, 710-713, 726-727; docs/adr/ADR-0078-capacity-group-lock-order-and-possession-protocol.md:81-83; crates/infra-postgres/src/capacity_transfer.rs:364-394; crates/infra-postgres/migrations/20261226000000_capacity_transfer_receipts.up.sql:22
   What: The built `Vacate` and `Acquire` insert one audit row per transfer, and `capacity_group_transfer_receipts.audit_id` is `NOT NULL`. The design changes several things:
   - transfers are "not audited one record each";
   - the commands now increase `transfer_count`, upsert `capacity_transfer_audit`, delete the yield row, set `held_since` and write `last_vacated_unleased_at` on a member row.

   ADR-0078 section 3 limits `Vacate` and `Acquire` to "only the header, one audit row, one outbox row and one transfer receipt". ADR-0156 amends section 3 only for the tenant-usage write. No task changes the built commands or the `audit_id` column. Separately, the hourly table is over-built: it adds an upsert on a hot row inside every transfer, while the header already holds `transfer_count`.
   Why: this departs from an accepted ADR without saying so (Constitution, Governance), and Tasks 039-7 and 039-9 do not cover the change.
   Correction:
   - Record the new write set in ADR-0156 as an amendment of ADR-0078 section 3 and Decision 710.
   - Add the change to the built commands, and make `audit_id` nullable, in Task 039-2 or 039-7.
   - Simpler option: drop `capacity_transfer_audit` and its pass, and carry `transfer_count` in the `capacity.group.retired` record.

7. Major — The fan-out bound has no storage, no policy column, and nothing defined for children whose version records no bound
   Where: docs/designs/039-nested-collections.md:52-53, 170-175, 342-347, 667-678, 730-734, 778-782
   What: A pinned child counts "its version's recorded bound", but:
   - Task 039-2 adds only the `nesting` flag to versions, with no bound column;
   - a child version published without nesting (a flat root collection, or one published before this design) has no recorded bound, yet Requirement 039-002 makes its collections descendants that join the group;
   - "the bound left to it" for a child that follows the active version is not defined;
   - `nested_group_fan_out` is a new tenant policy with no column, no write path and no audit, although Principle VI says policy changes are audited.

   The claim of "at most L × `nested_group_fan_out`" also stops holding after the policy is lowered, and compensation runs outside the unit. Technical Plan 2 says "the creator acts only for such versions", which reads as if a non-nesting child's collections would charge a fresh unit, which is the limit-1 deadlock.
   Why: the security fix (security review item 3) cannot be built as written, and the child path is ambiguous at exactly the case it exists for.
   Correction: compute and store the bound for every version at insert, through `nesting_of`'s companion. Define "left" as the policy value divided by the product down the path to the child call. Add the policy column with its audited write. State that descendant resolution depends only on the run's `capacity_member` flag, whatever the child's version. Qualify the L × bound claim.

8. Major — One member per loop iteration costs one to two sweeper ticks per iteration, and a simpler shape exists
   Where: docs/designs/039-nested-collections.md:96-97, 294, 357-362, 630
   What: Each loop iteration adds:
   - a member insert and its audit record;
   - a `FinishPath` and a finish receipt;
   - one more audit record;
   - an `Acquire` on the next tick, because the new member joins as a non-holder behind a final holder.

   That is about 0.5 to 1 second per iteration, plus three capped commands, on a primitive that published `maxIterations` 1,000 because it was already slow (8,806 iterations in 1,800 seconds). Loop iterations are sequential, so possession never needs to move from one iteration to the next.
   Why: this is over-built, adds latency and sweeper load the design does not state or measure, and it causes finding 2.
   Correction: use one scoped-path member per root loop, keyed by the loop's owner node with a fixed index, and gate the guard on (run, root node) for loops. This amends ADR-0156 decision 1. If per-iteration members are kept, state the per-iteration cost and measure it in Task 039-15.

9. Major — Complexity Tracking justifies possession against the wrong alternative and does not state what users will see
   Where: docs/designs/039-nested-collections.md:39-40, 61-64, 229-230, 304-311, 624
   What: The row "Possession and a sweeper" rejects "a unit per descendant". That alternative is rejected by joining (Decision 705), not by possession. The real simpler alternative is joining without possession, with concurrency bounded by the new fan-out bound (which already counts children). That alternative is never weighed. The design also never tells authors the visible consequence: at any tenant limit, the members of one group (the root iteration and every child workflow under it) never run at the same time. In CUJ 1, the five children declared with concurrency 5 run one after another, each transfer costing Q + S + 3T.
   Why: the largest piece of the design (the sweeper, the quantum, yields, transfers, receipts, fairness) rests on a justification that does not hold. User story 039-01 hides serial children.
   Correction: in Complexity Tracking and Alternatives, state what possession bounds that the fan-out bound does not, or record why possession stays given Owner question 706 and ADR-0075. In Requirement 039-001 and the release notes, state that children of one group run one at a time.

10. Major — Activation has no fence or harmlessness argument for an older binary
    Where: docs/designs/039-nested-collections.md:551-556, 154-164; docs/designs/053-rolling-upgrades.md:272-275
    What: The answer to the correctness review's item 17 relies on `ServingGate::uses` (or `requires`), which only a guard-capable binary has. Requirement 053-006 covers "an older binary not counted" that reconnects and serves for about 12 seconds. Such an engine calls no gate and no guard, and it leases member steps without possession.
    Why: Requirement 053-006 obliges the design to state either a fence or why this is harmless. The current text covers only new binaries.
    Correction: state the harm bound (possession may be broken for up to one heartbeat, nothing written that N-1 cannot process) and accept it explicitly, or add a fence in the database on the lease write for flagged runs. Test it with an N-1 engine in the mixed-writer suite.

11. Nit — Terms are used before they are defined, and identifiers mix forms
    Where: docs/designs/039-nested-collections.md:11-19, 31-53, 118-130
    What: The Terms list does not define quantum, quiescent, fence and form (a), registry requirement, serving gate, continuation chain, shard, `owns_unit`, slot-held or drain, yet the Overview and Requirements 039-005 and 039-006 use them. "Decision 705" (feature 014) and "Decision 039-011" sit side by side without a feature prefix. "ADR-0030 §14" and "Architecture decision 0078" are mixed. A reader with no history cannot follow Requirements 039-005 to 039-007.
    Correction: add those terms. Write feature 014's register items as "Decision 014-705" and "Gap 014-724", or say once that bare numbers mean feature 014.

12. Nit — Register rows D039-012 to D039-018 have no date
    Where: specs/039-nested-collections/research.md:22-28
    What: The table has three columns, and these rows fill two.
    Correction: add `2026-11-27`.

13. Nit — The sweeper budget is ambiguous
    Where: docs/designs/039-nested-collections.md:200-206, 423-424
    What: "(configurable to 512)" does not say which of the two limits it applies to. Non-functional requirement 039-003 budgets yield inserts only, while Technical Plan 3 puts inserts and deletes on that budget.
    Correction: name the setting, and say "yield inserts and deletes" in both places.

14. Nit — The two lanes share crates
    Where: docs/designs/039-nested-collections.md:654-662, 710-713, 774-782
    What: Task 039-7 (lane A) edits the lease producers in `crates/application`. Task 039-21 (lane B) edits `crates/infra-postgres/src/execution/ad_hoc.rs`. Task 039-22 is creator code but sits in slice 4.
    Correction: move the tasks or drop the "no crate in common" claim.

15. Nit — The guard does not check the yield row's epoch
    Where: docs/designs/039-nested-collections.md:410-412, 455
    What: The yield row stores `epoch`, but the guard refuses on "no yield row exists" alone. A row left over from an earlier epoch would yield a new holder at once and mark it unleased.
    Correction: the guard and selection 5 match the yield row's epoch with the header's.

16. Nit — A gate row does not name its task
    Where: docs/designs/039-nested-collections.md:703; docs/owner-decisions.md:111
    What: Task 039-5 cites OD-2026-11-27-392 as a gate, but that row's `Covers` does not name T039-5, which docs/designs/README.md requires.
    Correction: add T039-5 to the row's `Covers`.

17. Nit — Form (a) is chosen without stating the write rate
    Where: docs/designs/039-nested-collections.md:159-162
    What: Design 053 limits form (a) to "a write of moderate rate". Every nested admission and join takes `FOR KEY SHARE` on one global row, and the 100,000-instance workload will run at that rate.
    Correction: state the expected rate and why form (a) fits it, or choose form (b).
