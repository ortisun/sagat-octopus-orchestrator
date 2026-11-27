# Security review of design 039, round 1 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: 71629427d6efa1f9197683c2b4f9b3c6b8369ad0
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 2, Major 9, Nit 6

1. Blocker — The design lists `nested-collections-v1` on the wrong kind of process, so an engine can lease work without the guard
   Where: docs/designs/039-nested-collections.md:123-128, 412-416; docs/adr/ADR-0154-nested-capacity-membership-is-created-by-the-engine-and-the-sweeper-serves-every-member.md:53-55; apps/execution-engine/src/lib.rs:128-140; apps/execution-engine/src/capabilities.rs:18-35; scripts/check-capacity-activation.py:11-14
   What (verified in code): `WorkerService`, which owns both lease producers (`crates/application/src/worker.rs` and `worker/scoped_claims.rs`), is built only in the execution engine. No file under `apps/control-plane` builds one or depends on `sagat_api_grpc`. Requirement 039-011 has the control plane list the capability when "its worker guard runs" and the engine list it when "its creator and its `capacity_sweep` pass run". Following that text, an engine running the creator and the sweeper but with `child_root_capacity_guard` off lists the capability. The registry then accepts activation, and that engine grants leases to non-holders.
   Why: possession is the only thing that keeps the tenant limit holding inside a group. This breaks ADR-0075 conditions (f) and (g). The round-1 review's correction (item 5) pointed at the control plane, which does not run the guard, and the design adopted it.
   Correction: an engine lists the capability only when its guard, its creator and its sweeper all run. The control plane needs it only if it publishes, and then only for the `NESTED_COLLECTIONS_DISABLED` refusal. Add a test in `apps/execution-engine/tests/registry.rs` for an engine with the guard off and the passes on.

2. Blocker — A cancelled member can never reacquire possession, so its group never retires and its tenant unit is never released
   Where: docs/designs/039-nested-collections.md:291-294, 319-320, 356-357; crates/infra-postgres/src/workers.rs:97-98, 128-146; docs/adr/ADR-0075-worker-leases-retain-child-root-possession.md:22-24
   What (verified): the design says the shared predicate "excludes the steps of paused and stopping runs". The code it cites does not. A stopping run still leases its non-FORWARD steps (`r.stop_reason IS NULL OR st.kind <> 'FORWARD'`). The compensation branches have no stop filter. Compensation leases are gated by possession (ADR-0075).
   Scenario: cancel a nested root iteration whose two ChildRoot members both have compensations. The holder compensates and is finished, and the group becomes VACANT. The other member is stopping, so the predicate as written calls it not runnable and `Acquire` never selects it. Its compensations never lease, it never becomes final, and the group never retires. Technical Plan 2 releases the unit only at retirement. At a limit of 1, the tenant can never admit a root iteration again.
   Why: this breaks the goal "no deadlock at any tenant limit". The differential test of Requirement 039-007 compares the fragment with itself, so it cannot catch a wrong predicate.
   Correction: state the predicate as the code has it (compensations and other non-FORWARD steps of stopping runs count as runnable; paused runs do not). Add a limit-1 test that cancels a root iteration with compensations in two members.

3. Major — Nesting lets a workflow author get around the tenant iteration limit
   Where: docs/designs/039-nested-collections.md:53-55, 81-87, 278-284, 399-402; crates/infra-postgres/src/primitives.rs:1910-1920; docs/adr/ADR-0030-structured-instances-and-continuation-chains.md:101-108, 142-144
   What (verified): today every root iteration checks `active < limit` and is charged a unit, so a limit of L means at most L iterations run at once. The design makes inner collections and loops "work of that member": they are not charged and not serialized by possession, because the guard is keyed only by (run, root node, iteration index).
   Scenario: at a limit of 1, an author wraps a `forEach` over 10,000 items with concurrency 256 inside a one-item outer `forEach`, or inside a child workflow. 256 iterations then run at once. With depth 4, concurrency grows multiplicatively per level, up to `maxInstances` 100,000 per run and 1,024 runs per tree. Timers, event waits and control nodes are not gated by possession at all. One tenant at limit 1 can therefore go past the 20,000 pending waits per scheduler replica that Owner decision 2026-11-04-6 set (O463), and those replicas are shared with other tenants.
   Mitigation (verified, workers.rs:85-89, 170-175): the per-tenant `activity_concurrency` ceiling still bounds leased activity steps. Nothing bounds waits, timers or instances.
   Why: the claim "the tenant's limit still bounds what runs" is false. Success criterion 039-003 even assumes a holder runs with concurrency above 1.
   Correction: bound the worst-case concurrent fan-out of each group (the product of local concurrency down the nest, children included) at publication against a tenant policy value, or charge a per-tenant active-instance ceiling. Either keeps limit-1 progress, because neither blocks admitted work. State the semantics and test the wrapped-collection case.

4. Major — Removing the requirement races the creator; the design does not follow ADR-0131
   Where: docs/designs/039-nested-collections.md:128-130, 268-272, 416-420; docs/adr/ADR-0131-migrations-expand-before-and-contract-after-a-release-and-a-binary-refuses-a-schema-it-cannot-use.md:44-56, 76-77 (accepted 2026-11-26)
   What (verified in the text): the creator checks the requirement with "a read of the registry's requirement, no lock". Removal is one `verify` step: no group is non-RETIRED and no nesting version is "active".
   Race: transaction T1 admits an iteration of a run of nesting version V (V deactivated but the run still in flight) and reads the requirement as recorded. Transaction T2, the removal, cannot see T1's uncommitted group, passes `verify` and commits. T1 then commits a HELD group. Engines without the guard can now register, and they lease non-holders. The design also does not say what the creator does for a nesting version once the requirement is gone.
   Why: ADR-0131 decision 5 requires a database-side fence, conditional on the capability's row, on every write of new state. Decision 6 requires a two-phase drain. Its option 6 rejects one-phase removal for exactly this race. Design 039 does not cite ADR-0131.
   Correction: add an INSERT trigger on `capacity_groups` and `capacity_group_members` that reads the capability row `FOR SHARE` and refuses when the capability is absent or `draining`. Use the two-phase removal with a drain check of "no non-RETIRED group and no non-final run of a nesting version". Specify the creator's refusal.

5. Major — Skipping the guard on the run-row flag `capacity_member` fails open
   Where: docs/designs/039-nested-collections.md:142-146; crates/infra-postgres/src/execution/worker_capacity.rs:64-77
   What (verified): today the guard finds ChildRoot membership by chain, through `chain_segments`, so a successor run of a member chain is guarded. The design skips the guard's reads when the run row's flag is false, and never says who sets the flag on a continue-as-new successor or on any other run-insert path.
   Scenario: an author's child workflow, started inside a `forEach`, uses continue-as-new. The successor run is inserted with `capacity_member=false`. Its claims skip the guard, so it leases while not the holder and runs beside the holder of its own group or of another.
   Why: this turns the guard from fail-closed to dependent on every run writer remembering the flag.
   Correction: enforce the flag in the database. Make it NOT NULL with no default on chain successors (copied from the chain under the chain lock). Add a trigger on member insert that refuses unless the run or chain head is flagged. Never clear the flag. Add a successor test.

6. Major — A yield request can strand the holder when no other member is runnable any more
   Where: docs/designs/039-nested-collections.md:101-106, 314-320, 322
   What (verified in the text): `yield_requested_at` is cleared only by `Vacate` and `Acquire`. `Vacate` runs only "while another member has runnable work".
   Scenario: holder H passes the quantum while M has runnable work, so `RequestYield` sets the flag. M is then paused, or its remaining work becomes a timer wait. The guard now gives H no new lease, H becomes quiescent, and no selection applies. H's runnable work waits for as long as M is paused, and the group keeps its tenant unit. An author or operator can reach this by pausing a single child run.
   Correction: add a selection, or a rule inside the guard, that clears a yield whose waiting member is no longer runnable. A sweeper write needs no lease gating. Add a pause-after-yield test.

7. Major — `RequestYield` under `NOWAIT` competes with the holder's own claims for the group row
   Where: docs/designs/039-nested-collections.md:315; crates/infra-postgres/src/execution/worker_capacity.rs:130-133, 182; docs/adr/ADR-0075-worker-leases-retain-child-root-possession.md:65-68
   What (verified): every claim by the holder keeps the group row `FOR UPDATE` until its lease commits, including through blocking advisory-lock waits. `RequestYield` must update that row and gives up on `NOWAIT`.
   Suspicion (not measured): for exactly the holder the yield exists for (many workers, inner concurrency), the row is held almost all the time. `RequestYield` then nearly always answers Busy, the quantum never takes effect, and Gap 718 stays open. Success criterion 039-003 may pass only under light polling.
   Correction: keep the yield request out of the header row (for example a `capacity_group_yields` row inserted with `ON CONFLICT DO NOTHING`), which the guard reads under its `SKIP LOCKED` lock. Alternatively, measure the busy ratio in Task 039-12 with 64 pollers on one holder.

8. Major — The sweeper is shared across tenants with no tenant fairness, so one tenant can delay another tenant's unit releases
   Where: docs/designs/039-nested-collections.md:147-152, 304-325; crates/scheduler/src/shards.rs:57-62, 232-236
   What (verified in the text): each replica has 64 commands per tick (128 per second) for every tenant on its shards. The design gives no priority among the five selections, no per-tenant share, and does not exclude groups whose yield is already set from selection 3. Because unit release moved into the retiring `Finish`, a tenant's admissions now depend on this budget.
   Scenario: tenant A has 64 or more contended groups, or 64 yielded holders running long activities (re-selected every tick), on a shard. Tenant B's retiring `Finish` is deferred tick after tick, so B's units stay counted and B's fresh admissions get `AtCapacity`.
   Why: the scheduler already weighs tenants (`tenant_weights`); the sweeper ignores those weights.
   Correction: serve finishes before transfers. Exclude groups whose yield is set from selection 3. Page by tenant round-robin using the scheduler's weights. Add a test with a noisy tenant beside a limit-1 tenant.

9. Major — Oldest-ready-first `Acquire` can hand possession to a member no worker can serve
   Where: docs/designs/039-nested-collections.md:319-320, 355-357; crates/infra-postgres/src/workers.rs:79-176
   What (verified): candidate selection depends on the session: the activity list (`$3`), the protocol minor (`$5`), the subject's secret pools (`$6`), activity quotas and compensation breakers. The sweeper has no session, so its "runnable" cannot include those conditions.
   Scenario: member A's oldest ready step is an activity with no connected worker, a quota of 0, an open breaker, or a pool secret no worker holds. `Acquire` picks A, A cannot lease and is quiescent, B has work, A is vacated, and `Acquire` picks A again, every tick. B starves indefinitely. Each cycle writes two audit rows, two outbox rows and two receipts, and spends sweeper budget shared with other tenants (finding 8).
   Correction: base acquire order on recent lease failure or on rotation (for example skip a member that was acquired and vacated without a lease within the last quantum). Add a test with an unserved activity.

10. Major — Transfers write audit rows that are never deleted, at a rate the workflow author controls
    Where: docs/designs/039-nested-collections.md:114-118, 222, 380-386; crates/infra-postgres/src/capacity_transfer.rs:45-46, 364
    What (verified): every `Vacate` and `Acquire` writes an audit record and an outbox row. The design makes each transfer's audit record "the permanent evidence", and keeps every receipt younger than 24 hours.
    Arithmetic: one contended group with a 1-second quantum makes about one transfer pair every 2 seconds, which is 86,400 audit rows per day. Under finding 9 it is one pair per tick. A tenant with limit 1,000 can write tens of millions of permanent audit rows per day and keep up to about 172,800 receipts per group. The design's own justification says receipts are "needed for minutes".
    Correction: audit membership creation and finish, and count transfers as metrics plus a counter on the header (or one audit row per period), and/or bound audit by a per-tenant quota. Shorten the receipt window to the commit-uncertainty horizon (minutes), plus the latest 16.

11. Major — Deleting a retired group "with its root run" cannot work as specified, which also blocks purging runs
    Where: docs/designs/039-nested-collections.md:114-118, 385-387, 501-507 (Task 039-2); crates/infra-postgres/migrations/20261210000000_capacity_finish_storage.up.sql:39-40, 119-157; crates/infra-postgres/migrations/20261212000000_scoped_finish_binding.up.sql:38-40, 43-47; crates/infra-postgres/AGENTS.md:8
    What (verified): `capacity_groups_guard` and `capacity_group_members_guard` refuse every DELETE, and are `ENABLE ALWAYS`. Members and receipts have foreign keys to `execution_chains` and `runs`. Task 039-2 replaces only the receipts' append-only triggers. No run purge exists in `crates/` (no `DELETE FROM runs`); feature 020 is not built.
    Consequence: when it is built, feature 020's purge of a run that was part of a nested group is refused by these foreign keys and triggers. The personal data in that run cannot be erased unless someone widens grants for the purpose. The design's total bound ("bounded by the retained runs") depends on a component that does not exist.
    Correction: Task 039-2 must replace both guards so they allow DELETE only of a RETIRED group and its members, and only in the purge path. Name that path (an owner-role or definer function, never a DELETE grant to `sagat_app` on these tables). Record the dependency on feature 020 as a gap.

12. Nit — The receipt-trigger condition "in the transaction that purges its root run" is unspecified
    Where: docs/designs/039-nested-collections.md:383-385
    What: if it is implemented with a session or transaction setting, `sagat_app` can set it and delete any receipt.
    Correction: test "the root run row no longer exists", which `sagat_app` cannot cause because it cannot delete runs. Write the trigger function as SECURITY INVOKER with a pinned `search_path`.

13. Nit — Column-scoped INSERT comes with no insert-time invariants
    Where: crates/infra-postgres/migrations/20261210000000_capacity_finish_storage.up.sql:150-157 (triggers fire on UPDATE or DELETE only); docs/designs/039-nested-collections.md:296-297
    What (verified): with the grant, `sagat_app` can insert a FINISHED member, a member into a RETIRED group, or a header with any counters or any `shard`.
    Correction: add an INSERT trigger forcing revision=epoch=active_members=1 and HELD on a new header, ACTIVE with null `finished_at` on a new member, and refusing a member for a RETIRED group. Derive `shard` from the root run rather than taking it from the caller.

14. Nit — The list of header writers and the lock contract are incomplete
    Where: ADR-0154:35-36; crates/domain/src/execution/capacity.rs:221-245; docs/designs/039-nested-collections.md:291-295, 365-374; crates/infra-postgres/src/primitives.rs:1912
    What (verified): a join updates the header (revision, active_members), so there are three writes outside the commands, not two. The retiring `Finish` now takes `tenant_iteration_usage FOR UPDATE` while holding the group row, and the checked contract does not list it.
    Correction: list the join and the tenant-usage lock in ADR-0154 and in `worker-lock-sequence.json`.

15. Nit — `nesting` is a flag stored by one writer of version rows
    Where: docs/designs/039-nested-collections.md:268-272; crates/infra-postgres/src/execution/ad_hoc.rs:34; crates/application/src/runs/ad_hoc.rs:612-639
    What (verified): a second path inserts `workflow_versions` rows. It is safe today only because `validate_definition` refuses collections.
    Correction: derive `nesting` from the parsed document, or add a fitness check that every version-insert path sets it.

16. Nit — The design names the wrong lease producers, and the guard has no path argument
    Where: docs/designs/039-nested-collections.md:345-348; crates/application/src/worker.rs:1413; crates/application/src/worker/scoped_claims.rs:239
    What (verified): the candidate window in `workers.rs` only suggests candidates; it grants nothing. The producers are `worker.rs` and `scoped_claims.rs`. `worker_capacity_available()` takes no instance path, which the scoped guard keyed by iteration needs.
    Correction: name both producers and pass the instance path to the guard.

17. Nit — The decision register disagrees with the design
    Where: specs/039-nested-collections/research.md:13-14 against docs/designs/039-nested-collections.md:102-103, 311-320
    What: D039-004 gives the quantum range as 100 milliseconds to 60 seconds; the design says 500 milliseconds to 60 seconds. D039-003 lists five selections that differ from the design's five (it has no yield).
    Correction: append a superseding decision row.

No finding after looking:
- **Tenant boundary for error responses and API bodies:** there is no new API; every new row and query carries the tenant, and children inherit the parent's namespace (`chain_progress.rs:1031-1034`).
- **Metric labels:** all are closed sets, with no tenant label.
- **Secrets in the journal, logs or errors:** nothing new.
- **Injection and parsing:** no dynamic SQL; codec bounds unchanged.
- **New definer functions:** the design adds none.
