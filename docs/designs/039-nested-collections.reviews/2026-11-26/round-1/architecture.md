# Architecture review of design 039, round 1 (2026-11-26)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: dfc7611236ed0f675f78769250152d13d0cadad8
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 6, Major 10, Nit 8

1. Blocker: scoped-path members cannot be granted a lease.
   Where: design §4 (lines 268-271); `crates/infra-postgres/src/execution/worker_capacity.rs:48-63`; `crates/infra-postgres/src/workers.rs:37-55`; ADR-0075 Decision (lines 47-48).
   What: the guard refuses every claim on a run that has any `SCOPED_PATH` member (`UnsupportedScope`). The candidate window only knows ChildRoot members. ADR-0075 still requires scoped paths to fail closed. The design adds `VacatePath`/`AcquirePath` but no path-keyed guard.
   Why: the first nested admission halts the whole root run, including its top-level steps.
   Fix: add a guard keyed by (run, root node, iteration index) in both lease producers, add the scoped clause to the shared predicate, amend ADR-0075's bullet through ADR-0154, and add race tests.

2. Blocker: no Finish exists for a nesting iteration or for a non-success outcome.
   Where: design §1 and §4; `crates/application/src/leaf_certification.rs:1,43-45,96-98`; `crates/infra-postgres/src/scoped_leaf_finish.rs:1`.
   What: the scoped leaf finish only handles a depth-1 `OrdinaryLeafV1` path, it needs a certified success, and it is limited to an ordinary leaf body. A body that nests is not an ordinary leaf, and failed, cancelled or compensated iterations are never certified.
   Why: the member never finishes, so the group never retires and its unit counts forever (D703). This is the case ADR-0078 "Sweeper coverage" requires closed.
   Fix: specify a scoped finish for nesting bodies and every terminal iteration outcome, with its proof of the source facts and tests.

3. Blocker: descendant admission is undefined, which deadlocks at limit 1.
   Where: design §2 (lines 233-243); `crates/infra-postgres/src/primitives.rs:1910-1920`; `crates/infra-postgres/src/loop_progress.rs:292,1559,2425`.
   What: every collection and loop iteration admission checks and charges `tenant_iteration_usage` unconditionally. The design says inner work "joins nothing" but changes no admission path. It also keys group creation on a "root collection", so a `forEach` in a child run takes a fresh unit, and a top-level loop whose body nests is not covered.
   Why: ADR-0030 §14 and D705 are violated, and the policy-reduction test that §14 requires is missing.
   Fix: define the membership context (any admission inside an ACTIVE member skips the fresh-admission check), name the code paths, and add limit-1 tests (child containing a collection, loop containing a collection) plus a reduction test.

4. Blocker: memberships would be created before activation.
   Where: design §2, line 236 ("A root child workflow started outside a collection ... does the same").
   What: this applies to workflows already published with a top-level child, which today charge no unit. Once Task 039-5 ships, every such child start creates a group and a member before `nested-collections-v1` is recorded.
   Why: it breaks ADR-0075 (f), ADR-0078 §6, Non-functional requirement 039-002 and released behaviour at `iteration_concurrency` 0.
   Fix: a child outside any member creates no group. The creator acts only for versions published under the requirement and checks the requirement.

5. Blocker: the activation does not meet ADR-0075 (f), and removing the requirement is unsafe.
   Where: design §9 and Requirement 039-011; `crates/application/src/worker.rs:584,635`; ADR-0082 Decision 7.
   What: the registry checks a "guard-capable" capability, but (f) and ADR-0078 §6 require the guard to be enabled. The guard is a builder flag with no configuration path, and the engine's creator and sweeper are unnamed. Removing the requirement lets processes without the guard register while groups exist, and an older binary cannot parse stored nested versions.
   Fix: list the capability only when the guard runs (control plane) and when the creator and sweeper run (engine). Refuse removal while any group is not `RETIRED`. State that a binary rollback below the requirement is unsupported.

6. Blocker: the quantum cannot preempt a busy holder.
   Where: Requirement 039-006, §3 selection 3, CUJ 2, Success criterion 039-003.
   What: `Vacate` requires no `LEASED` or `RUNNING` step (D708), and the holder's own claims hold the group with `SKIP LOCKED` until the lease commits. A holder with continuous or concurrent work (for example an inner `forEach` with concurrency above 1) is never seen quiescent.
   Why: Gap 718 is not closed and Success criterion 039-003 cannot pass.
   Fix: once `held_since` plus the quantum has passed and another member waits, the guard stops granting new leases to the holder. This gates leases only, so it fits O706. Prove it with a test.

7. Major: when the tenant unit is released is not decided.
   Where: ADR-0075 (c); D703; `crates/infra-postgres/src/primitives.rs:2438-2441`; ADR-0078 §3.
   What: the commands write no quota row, and the iteration checkpoint decrements the unit. A cancelled iteration frees its unit while ChildRoot members still compensate (O703).
   Why: Success criterion 039-001 ("never two members of different groups") then fails.
   Fix: release the unit at group retirement, or reword Success criterion 039-001. Test either way.

8. Major: Requirement 039-008's claim about the existing constraint is false.
   Where: `crates/infra-postgres/migrations/20261212000000_scoped_finish_binding.up.sql:34-35`.
   What: `capacity_group_members_scoped_path_unique` includes `group_id` and is not limited to ACTIVE rows, so one iteration could be a member of two groups.
   Fix: in Task 039-2, add a partial unique index on (tenant, namespace, `original_run_id`, `root_owner_node`, `iteration_index`) for ACTIVE `SCOPED_PATH` rows.

9. Major: the lock order of the scoped-path commands omits the chain.
   Where: design §4, line 270.
   What: the design orders "run, then group `NOWAIT`, then shard". The existing scoped finish locks the member's chain first (`crates/infra-postgres/src/scoped_leaf_finish.rs:301-309`), as ADR-0030 §18 and ADR-0078 rule 1 require.
   Fix: state "chain (if any), then run, then group `NOWAIT`, then shard `FOR SHARE`", and put it in the checked contract.

10. Major: grants and triggers are incomplete for Tasks 039-2 and 039-9.
    What:
    - The `capacity_groups` UPDATE grant does not include `held_since` (`20261210000000_capacity_finish_storage.up.sql:176`).
    - The scoped columns of the finish receipts are not insertable by `sagat_app` (`20261212000000_scoped_finish_binding.up.sql:103-115`), so the scoped finish cannot run as the runtime role.
    - The receipt tables have `ENABLE ALWAYS` append-only triggers that refuse DELETE (`20261226000000_capacity_transfer_receipts.up.sql:44-51`; `20261210000000_capacity_finish_storage.up.sql:158-167`), so §7's "grant DELETE" purges nothing.
    Fix: list every grant and trigger change in Task 039-2, with negative privilege tests.

11. Major: retention is not bounded in total.
    Where: Requirement 039-009 and §7.
    What: 16 receipts per group are kept forever, and groups and members are undeletable tombstones with foreign keys to runs and chains. Storage grows with every nesting admission, including the 100,000-instance workload.
    Fix: state a total bound or a compaction after retirement, reconciled with the down migration's refusal and with run retention.

12. Major: which replica serves a group, and the sweeper's throughput, are not derived.
    Where: design §3 and Non-functional requirements 039-001 and 039-003.
    What: a group spans runs on several shards, and each command is fenced on the shard of the run it names. Nothing says which replica selects a VACANT group, or how "oldest-ready first" works across shards.
    Throughput: 64 commands per 500 ms tick is 128 commands/s, so at most 64 transfers/s per replica. With a 1 s quantum, more than about 64 contended groups per replica miss the "two ticks at p99" target.
    Fix: define how groups are assigned to replicas and derive the bounds.

13. Major: Vacate churns on single-member groups.
    Where: §3 selection 3 ("has no runnable work").
    What: a holder is vacated with no other member waiting, so every gap between steps costs a Vacate and an Acquire (two receipts, two audit rows, two outbox rows) and up to two ticks of latency.
    Fix: vacate only when another member has runnable work.

14. Major: Non-functional requirement 039-002 cannot hold once nesting is active.
    What: with the guard on, every poll runs a second candidate query (`crates/application/src/worker.rs:984-999`). Every claim runs the guard's reads (`worker_capacity.rs:50-90`) and the extra candidate-window filter, whether or not the run is a member.
    Fix: restate the requirement as "nothing added with the requirement off", and give budget lines for the "on" state in Task 039-13.

15. Major: several rules have no check that fails when they break.
    Where: Task 039-3.
    What:
    - Requirement 039-004 says "only the sweeper calls the commands", but the planned check has no allowlist; ADR-0078's "Enforced by" asks for one (`scripts/check-capacity-activation.py:15-28` bans every production call today).
    - ADR-0075 (g) is not planned.
    - The lock sequences of the creator and of the commands are not in the checked file.
    - Gap 726 (a holder change between the two reads, the cost of the generic plan) is not addressed.
    Fix: extend Task 039-3 to cover all four.

16. Major: ADR-0154 was accepted before review and leaves open what it claims to decide.
    Where: ADR-0154, line 3.
    What: it was accepted the same day this design is still a Draft, with no review evidence. Its text does not decide the scoped guard, the unit release, how the guard is enabled, or the descendant rule, yet it claims ADR-0075 (c), (d) and (f).
    Fix: return ADR-0154 and Owner decision 2026-11-26-150 to Proposed until the design is approved, and add those decisions.

17. Nit: the migration list in "The Problem" (lines 53-55) omits `20261226000000_capacity_transfer_receipts`.

18. Nit: the quantum's lower bound of 100 ms is below the 500 ms tick (O463). Start the range at the tick.

19. Nit: selection 5 is covered by selection 2, since every member of a VACANT group is a non-holder. Also, ADR-0154 should state that the creator's initial `HELD` state is an exception to ADR-0078 §3 ("possession moves only by the three commands").

20. Nit: codec table row "collection beside loops, approvals and other nodes" is not a list, which contradicts Requirement 039-001. Name the kinds and keep continue-as-new and return refused.

21. Nit: a join that gets `Busy` is retried with no bound and no metric. Add a `busy` outcome and an alert.

22. Nit: there is no metric for sweeper backlog or deferral beyond the 256-candidate and 64-command limits.

23. Nit: state that the runnable predicate excludes paused and stopping runs (`workers.rs:97-98`). Otherwise `Acquire` can hand possession to a paused member and waste a quantum.

24. Nit: `capacity.member.created` depends on design 040's mapping. Name the change to the closed audit vocabulary and its test.

Scope: I found no `039` or capacity branch in `.git/packed-refs` or `refs`. I could not run `git branch -a` or `git worktree list` here, so this check is incomplete.
