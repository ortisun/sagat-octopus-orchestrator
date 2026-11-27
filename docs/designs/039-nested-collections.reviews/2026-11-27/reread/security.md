# Security re-read of design 039 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: bbfd5b782f15c13a8a2f95ddc4991f15c31eeed7
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 5, Nit 8

## Previous review

1. Fixed. An engine lists the capability only when its guard, creator and sweeper all run, and Task 039-18 adds the engine test. Lines 193-196, 846, 1060-1061, 1072-1073.
2. Fixed. The predicate is now written as the candidate query has it, and Task 039-6 adds the limit-1 test with two compensating members. Lines 163-167, 651-658, 992-998.
3. Partly fixed. A fan-out bound now exists (lines 52-53, 210-228, 844, 1095-1104), but child workflows and `parallel` get around it, and the tenant-wide total has no ceiling. Repeated as finding 1.
4. Partly fixed. The form (a) fence and the two-phase removal are in (lines 196-204, 422-432, 794-806, 1067-1076), but the database only refuses a group or member insert if the writer remembered to call the fence. Repeated as finding 5.
5. Partly fixed. `capacity_member` triggers are specified (lines 229-232, 515-521, 1077-1080), but the successor copy cannot be built where the design puts it. Repeated as finding 4.
6. Partly fixed. Selection 4 clears a stale yield (lines 151-153, 575-576, 584-586), but a stale join wait brings the stranding back. Repeated as finding 3.
7. Fixed. The yield is now a row of its own with no lock on the header, and there is a 64-poller test. Lines 43-44, 484-507, 571-574, 876, 1112-1114.
8. Fixed. Finishes are served first, groups that already have a yield are excluded from selection 3, pages are weighted by tenant, and there is a noisy-tenant test. Lines 560-576, 845, 1021-1025.
9. Fixed. `Acquire` skips a member that was acquired and vacated without a lease, with tests. Lines 158-161, 579-582, 630-634, 1011-1013.
10. Fixed in the design: no audit row per transfer, `transfer_count` instead, receipts kept 15 minutes plus the latest 16. Lines 178-186, 714-721. The ADR and the registers still say otherwise (finding 13).
11. Fixed. The purge function is named, the delete guards are replaced, and Gap 039-008 is recorded. Lines 180-183, 331-333, 699-712, 1028-1032. A residual Nit is finding 10.
12. Fixed. The exemption now tests `current_user` against the table's owner, not a setting `sagat_app` could set. Lines 698-705.
13. Partly fixed. The insert trigger forces state, counters and `shard` (lines 173-177, 939-941), but the caller still chooses `owns_unit` and `namespace_id` (lines 533-536). Repeated as finding 11.
14. Fixed. The join's header write and the `tenant_iteration_usage` lock are listed, and the lock contract covers them. Lines 143-147, 508-514, 609-611, 685-686.
15. Fixed. `nesting_of` is called on every insert path, with an architecture check. Lines 415-418, 1089-1093.
16. Fixed. Both lease producers are named and the instance path is passed to the guard. Lines 49-51, 135-136, 624-627, 1006-1007.
17. Fixed by D039-014 (`specs/039-nested-collections/research.md:24`). The registers have drifted again elsewhere (finding 13).

## Findings

1. Major — The fan-out bound can be bypassed through child workflows and `parallel`, and a tenant administrator can raise the tenant-wide total on their own
   Where: docs/designs/039-nested-collections.md:210-228, 415-419, 1095-1104; crates/domain/src/workflow.rs:687-702; crates/infra-ir/src/convert.rs:602-628; crates/domain/src/execution/continuation.rs:21; crates/domain/src/quota.rs:512-519; crates/domain/src/identity.rs:567-568; crates/application/src/quotas.rs:427-447
   What (verified in the text and the code):
   - (a) The rule says "a version without collections records 1", so a pinned child version C that has no collections hides the bound of its own pinned child D. Example: a root `forEach` with concurrency 256 calls pinned C, and C calls pinned D, which has a root `forEach` of 256. Publication computes 256 × 1, but 65,536 iterations can be live in one group.
   - (b) A workflow without collections may contain a `parallel` node whose branches each call a child. Each active-version grandchild is checked on its own against "the policy divided by the product on the path to the call", so k branches give k × 256 live iterations. The tree allows up to 1,024 runs.
   - (c) The design does not say where the product down the path is carried across run boundaries, which a grandchild's check needs.
   - (d) `QuotaManage` belongs to the tenant `Admin` role. The design gives a default (256) and a maximum (4,096), but no ceiling and no tenant-only scope. With today's default iteration ceiling of 1,024, the tenant's own administrator alone reaches 1,024 × 256 = 262,144 live waits. Owner decision 2026-11-04-6 caps pending waits at 20,000 per scheduler replica.
   - (e) A lowered policy is not re-checked when a new root run of an existing version is admitted, so an operator cannot contain a tenant without deactivating its versions.
   Why: Requirement 039-014's claim "at most L × `nested_group_fan_out`" is false, and scheduler replicas are shared across tenants.
   Correction:
   - Make `fan_out_of` include pinned children's bounds, and sum `parallel` branches (or refuse `parallel` in any run under a member).
   - Store the remaining share on each member child run (for example `runs.fan_out_share`) and check grandchildren against it.
   - Define `default_ceiling` 256 and `maximum` 4,096, refuse a namespace-level value, and check `fan_out_bound ≤ effective policy` at every root admission.
   - State L × F against Owner decision 2026-11-04-6.
   - Test a pinned grandchild and a `parallel` of children.

2. Major — The lease fence refuses renewals, cancellations and every other rewrite of a step that is already `LEASED`
   Where: docs/designs/039-nested-collections.md:137-139, 779-786, 1105-1110; crates/infra-postgres/src/execution.rs:7102-7111, 7176, 7437-7449; crates/infra-postgres/src/execution/path.rs:1245
   What (verified in the code): the generic step writer always sets `status = $6`, and the same statement carries a lease renewal (`$23`) or `cancel_requested`. So `UPDATE OF status` fires for every write to a step that stays `LEASED`. As specified, the trigger refuses any row whose new status is `LEASED` unless `sagat.capacity_guard` is set, and only claims set it.
   Scenario: a flagged run's leased step gets a heartbeat renewal or a cancellation, and the transaction is refused. Requirement 039-003 says heartbeats must not be gated. Implementers would then be pushed to set the setting on every writer, which empties the fence.
   Correction: refuse only a new grant, on `INSERT`, or on `UPDATE OF status, lease_token_hash` when `NEW.status = 'LEASED' AND (OLD.status IS DISTINCT FROM 'LEASED' OR NEW.lease_token_hash IS DISTINCT FROM OLD.lease_token_hash)`. Add tests for renewing and cancelling a flagged run's leased step without the setting.

3. Major — A stale join wait strands the holder again (finding 6 of the previous review comes back)
   Where: docs/designs/039-nested-collections.md:151-153, 478-507, 571-576, 584-586, 1112-1116
   What (in the text): only the join that succeeds deletes a wait row (apart from the purge). Selection 3 inserts a yield for any wait a quantum old, and selection 4 clears a yield only when the group "has no join wait".
   Scenario: member M's child-start step finds the header busy and records a wait. M's run is then paused or cancelled. After a quantum the sweeper yields the holder H. Selection 4 never clears the yield because the wait exists. Selection 5 never vacates H because no other member is runnable. H stays stranded, and the group keeps its tenant unit, for as long as M is paused, which at limit 1 is the whole tenant. Anyone allowed to pause a run can cause this.
   Correction: record in the wait row the waiting parent (run, instance path, node) and a `last_busy_at` refreshed on each busy retry. Treat a wait as live only while that step is pending, its run is neither paused nor stopping, and `last_busy_at` is within two ticks. Selection 4 ignores and deletes stale waits. Add a pause-after-busy-join test.

4. Major — The successor-copy trigger for `capacity_member` cannot be built on the run insert (finding 5 of the previous review, still partly open)
   Where: docs/designs/039-nested-collections.md:229-232, 515-517, 1077-1080; crates/infra-postgres/src/chain_progress.rs:1266-1288, 1398-1410, 1762-1790; crates/infra-postgres/src/execution/worker_capacity.rs:75-77
   What (verified in the code): `runs` has no chain column, and the successor's `chain_segments` row is inserted after its run row. A trigger on the run insert therefore cannot tell that the run is a successor, or which chain head to copy from.
   Consequence: the successor's flag stays false. The guard's reads are skipped on that flag (Non-functional requirement 039-002), and the lease fence reads the same flag, so the successor leases while not the holder.
   Correction: copy the flag in an `AFTER INSERT` trigger on `chain_segments` (from `predecessor_run_id`'s run onto `run_id`) in the same transaction. Alternatively, refuse a segment insert whose predecessor is flagged and whose run is not. Name it so in Requirement 039-015 and Task 039-19.

5. Major — The fence only protects against the removal race if every writer remembers to call it (finding 4 of the previous review, still partly open)
   Where: docs/designs/039-nested-collections.md:173-177, 196-201, 422-427, 1067-1076
   What (in the text): groups and members are inserted from three places: the root collection admission, the root loop entry, and the join in `PgRunTx::commit` (Task 039-25). Each must call `nested_collections_required('continue')`, and no check enforces it.
   Scenario: one forgotten call, and after phase 2 a group or member commits with the requirement gone. That is exactly the race the fence exists to close, and it fails open (engines that no longer list the capability can register).
   Correction: call the fence in mode `continue` from the insert trigger that Requirement 039-008 already adds on both tables, and raise `nested_collections_fence` when it answers no. That adds no statement, and the rate is "a few hundred per second" by the design's own figure. Keep the `new_use` calls in the application.

6. Nit — Phase 2's table locks are not in design 053's model, and their lock order is the reverse of the writers'
   Where: docs/designs/039-nested-collections.md:798-804; docs/designs/053-rolling-upgrades.md:348-355, 648-654
   What: Task 053-9 says "phase 2 is unchanged", and in design 053 form (a) takes no table lock. Design 039 has phase 2 take both tables `IN SHARE MODE`, and 053 orders that table lock before the row `FOR UPDATE`. The creator takes the row (`FOR KEY SHARE`) first and the table second, so the two deadlock (40P01). It is safe, but it is unstated.
   Correction: list the two tables in the Task 053-9 descriptor and state the 40P01 retry on both sides. Alternatively, drop the table locks: the row `FOR UPDATE` already waits for every caller of the fence.

7. Nit — The refusal of a new run's start while draining has no surface, and the fence's mode is a free value
   Where: docs/designs/039-nested-collections.md:196-204, 753-762
   What: the codes table has no row for an API, schedule, batch or top-level child start of an active nesting version refused by `new_use`. Nothing says the function raises on an unknown mode.
   Correction: add a typed code for run starts and refuse any mode other than `new_use` and `continue`.

8. Nit — The lease-fence setting is bound to neither the transaction nor the run
   Where: docs/designs/039-nested-collections.md:782-790
   What: a session-level `set_config` left behind by mistake, or `ALTER ROLE ... SET`, would pass every later transaction on that pooled connection, and inside one transaction the value covers every run.
   Correction: store `v1:<txid_current()>`, and optionally the run id, and compare it in the trigger.

9. Nit — The relaxed slot check applies to every iteration, not only to descendants
   Where: docs/designs/039-nested-collections.md:237-240, 947-948; crates/infra-postgres/migrations/20261029000000_collection_progress.up.sql:263-265; crates/infra-postgres/migrations/20261107000000_loop_drain.up.sql:59-63
   What (verified): the database can then no longer refuse a root iteration admitted without charging the tenant, in any run.
   Correction: allow `ADMITTED AND NOT slot_held` only when the run's `capacity_member` is set, checked by an insert trigger.

10. Nit — Execution of the purge function, the owner exemption and the pinned `search_path`
    Where: docs/designs/039-nested-collections.md:699-707; crates/infra-postgres/migrations/20270119000000_batch_purge.up.sql:51
    What:
    - "Executable by the role feature 020's purge runs as" names no role and says no `REVOKE ALL ... FROM PUBLIC` (PostgreSQL gives functions to PUBLIC by default).
    - The function does not check that the root run is being purged.
    - Suspicion: where services connect as the owner (the migrations treat `sagat_app` as optional), the exemption lifts the delete guards entirely, which today refuse even the owner.
    - `pg_catalog, pg_temp` without `public` means the trigger bodies must schema-qualify every relation, or a caller's temporary table could shadow the receipt table that the retention trigger counts.
    Correction:
    - Revoke from PUBLIC and grant execution to a named role.
    - Require the run's purge or stub marker in the same transaction.
    - State that every relation is qualified, or use the repository's `pg_catalog, public, pg_temp`.

11. Nit — The caller still chooses `owns_unit` and `namespace_id` on a new header (finding 13 of the previous review, still partly open)
    Where: docs/designs/039-nested-collections.md:533-536
    What: a wrong `owns_unit` releases another group's unit at retirement. A wrong `namespace_id` hides the group from `capacity_purge_retired(tenant, namespace, run)`.
    Correction: the insert trigger derives `namespace_id` from `root_run_id`, and allows `owns_unit` only when the first member's iteration or loop holds a slot.

12. Nit — Orphaned side rows are never met by the sweeper
    Where: docs/designs/039-nested-collections.md:503-504, 547
    What: the sweeper pages over `capacity_groups WHERE possession <> 'RETIRED'`, so a yield or wait row whose group retired or was purged is never "met", contrary to line 504. The insert also never checks that the group exists or that its tenant matches.
    Correction: insert with `INSERT ... SELECT ... WHERE EXISTS` (a non-retired header of the same tenant and namespace, read without a lock), and correct the sentence.

13. Nit — ADR-0156 and the registers contradict the revised design
    Where:
    - docs/adr/ADR-0156-a-loop-owns-one-capacity-group-every-member-child-has-a-chain-and-the-yield-is-a-row-of-its-own.md:36-38 (`ON CONFLICT DO NOTHING`), :52 (`nested_collections_required()` with no mode), :59-60 (one audit record per group and hour)
    - specs/039-nested-collections/research.md:28 (D039-018: hourly summary, the trigger tests the root run's absence), :32 (`iteration_policy_change`)
    - docs/owner-decisions.md:46, :55, :62
    - docs/designs/039-nested-collections.md:339-342 (cites up to 455, while 456-458 are used)
    Correction: mark the superseded clauses in place, append superseding rows, and widen the cited range.

No finding, after looking:
- Tenant boundary in API bodies, errors and timing: the only new API surface is the existing quota endpoint. Every new row and query carries the tenant, and children stay in the parent's namespace.
- Metric labels: all are closed sets with no tenant label.
- Secrets and personal data in the journal, logs, audit and errors: nothing new is written there.
- Dynamic SQL, parsing and egress: none is added.
- Replay of a purged receipt: it is refused by the revision and epoch compare-and-set.
