# Approval read of design 039: security (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: 985dd80b1f46c119dcc8f718c9657ae8a8426eaa
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative. It is the fifth
security read of the 2026-11-27 round; the four before it are under `../reread/` to `../reread-4/`. Its Nits are
answered by follow-up tasks.

Counts: Blocker 0, Major 0, Nit 3

## Previous re-read

1. Major, the version fence on member runs depended on design 044's unbuilt table and code: **fixed.** The order point is at lines 1069-1071. Lane A owns the move's skip check and the completion refusal (line 1060). Task 039-17's gate requires the move and continue-as-new tests whenever `code_run_pins` exists (lines 1215-1216). Design 044 carries the trigger, the skip reason and the completion refusal in Task 044-42 (docs/designs/044-workflow-as-code.md:3338-3353), under Owner decision 2026-11-27-479 (docs/owner-decisions.md:87). One gap in the 044-first order remains; it is new finding 3.
2. Nit, `capacity_role` and `fan_out_share` were not write-once and a null role did not fail closed: **partly fixed.** These are right now:
   - the guard tests `current_user` against the table owner (lines 250-252 and 258-260);
   - the three checks on `runs` (lines 260-264);
   - the refusal keyed on "flagged and not `ROOT`" (lines 255-258);
   - the definer copy with a pinned path (lines 294-296).

   But the text says `sagat_app` holds no `UPDATE` on these columns. That is false, because the runtime role has a table-wide `UPDATE` on `runs`. And the definer function as described can be called directly. Repeated as finding 1; the second part is finding 2.
3. Nit, stale "policy divided" text and no owner row for the exception: **fixed.** The decision table now says `reserved_bound` (line 448), and the cited range includes 2026-11-27-479 (line 423). Row OD-2026-11-27-479 (docs/owner-decisions.md:87) refines row 454 and records the 0.3.0 exception to Requirements 044-023 and 044-019. ADR-0156:182-189 agrees.

## Findings

1. Nit — `sagat_app` does have `UPDATE` on `capacity_role` and `fan_out_share`, through its table-wide grant on `runs` (previous finding 2, partly fixed)
   Where: docs/designs/039-nested-collections.md:250, 259, 658-660, 1244; docs/adr/ADR-0156-a-loop-owns-one-capacity-group-every-member-child-has-a-chain-and-the-yield-is-a-row-of-its-own.md:186-187; crates/infra-postgres/migrations/20261009000100_execution.up.sql:273
   What (verified in the migrations):
   - `GRANT SELECT, INSERT, UPDATE ON runs, steps TO sagat_app;` is a table-level grant, and no later up-migration revokes it. A table-level `UPDATE` covers every column, including columns added later.
   - So the claims "`sagat_app` holds no `UPDATE` on it" (lines 250 and 259) and "`runs`: `UPDATE (capacity_member)` only" (line 658) are false. A column-level `REVOKE` cannot remove part of a table-level grant.
   - The `current_user = owner` trigger is what actually refuses the write, and it does refuse it. The property holds.
   - But Task 039-19's test "refused (no grant)" (line 1244) expects a privilege error (42501). It will get the trigger's error instead, or the implementer will try a column revoke that does nothing.
   Why: a guarantee written as two barriers when only one exists will mislead whoever later relaxes the trigger, believing the grant still holds.
   Correction: say plainly that the trigger is the only barrier, with `sagat_app` keeping its table-wide `UPDATE`. Drop "UPDATE (capacity_member) only" and "no grant" from lines 250, 259 and 658-660, from the test at line 1244 and from ADR-0156:186-187. Have the test assert the trigger's SQLSTATE.
   The alternative is to revoke the table-level `UPDATE` and grant a column list. That changes every writer of `runs` and needs its own task.

2. Nit — The definer copy function is called from the trigger, so `sagat_app` can also call it directly with arguments of its choosing
   Where: docs/designs/039-nested-collections.md:251-252, 294-296, 615-619, 658-660, 1234-1237; ADR-0156:184-186
   What:
   - The text says the `chain_segments` trigger "calls a `SECURITY DEFINER` function owned by the table's owner". A function called from a trigger body is checked for `EXECUTE` against the inserting role, `sagat_app`, so `sagat_app` must be able to execute it. PostgreSQL checks `EXECUTE` on the trigger function itself only at `CREATE TRIGGER`, not when it fires; this is from the documentation, I did not test it.
   - The text does not give the function's signature, any `REVOKE ... FROM PUBLIC`, or a tenant filter.
   - Failing scenario, if it takes `(successor_run_id, predecessor_run_id)`: a defective writer under `sagat_app`, and no runtime code needs to do this, calls it on an unflagged run B of the same version. B gets `capacity_member` true, `MEMBER_CHILD` and a share, and every check is met.
   - The slot trigger of Requirement 039-016 then admits B's root iterations `ADMITTED AND NOT slot_held` (lines 318-321), so they charge no tenant unit and are outside the reservation.
   - Within the stated threat model (the platform's own role), this is integrity, not an outside attack.
   Why: the text claims "only the segment copy writes them" (lines 251-252 and 659-660). With a callable definer that takes run ids, that claim holds only by convention.
   Correction (either one):
   - Make the trigger function itself the `SECURITY DEFINER`, owned by the table owner, with `REVOKE ALL ON FUNCTION ... FROM PUBLIC`. It then needs no `EXECUTE` grant to fire.
   - Or give the definer only the segment key `(tenant_id, chain_id, segment_index)`. It then reads both runs from the `chain_segments` row (its foreign keys at 20261102000000_execution_chains.up.sql:60-61 and 86 bind the predecessor to the same tenant, namespace and chain) and refuses when the successor's values are not null. Execute is granted to `sagat_app` only.

   Add a test that a direct call by `sagat_app` with a foreign pair is refused.

3. Nit — When design 044 lands first, Task 044-42's trigger refers to a `runs.capacity_member` column that does not exist yet, and the two designs disagree on who creates it in that order
   Where: docs/designs/044-workflow-as-code.md:3338-3348; docs/designs/039-nested-collections.md:1069-1071
   What (verified: `code_run_pins`, `capacity_role` and `fan_out_share` appear nowhere in `crates/`):
   - Task 044-42 always creates the trigger with `code_run_pins`, in Task 044-6's migration. Its last sentence says that when Task 039-19 lands after, 039-19 "cites this one".
   - In that order, `runs.capacity_member` (Task 039-2) does not exist. A PL/pgSQL body is not resolved at creation, so every later `UPDATE` of `code_run_pins` fails with 42703 and every move breaks. A SQL-language body would fail the migration instead.
   - The red test `the_pin_trigger_refuses_a_version_change_of_a_member_run` cannot flag a run.
   - Design 039's line 1069 says that in that order Task 039-19's trigger lands after Task 044-6, which contradicts 044-42.
   - No security window opens: no run can be flagged before 039-2, and Task 039-17's gate (lines 1215-1216) catches a missing trigger.
   Why: as written, Task 044-42 cannot be built in one of the two orders it claims to support, so someone will deviate from it.
   Correction:
   - In Task 044-42: "the trigger is created by whichever of Task 044-6 and Task 039-2 lands second, in that change".
   - Or make Task 044-42 wait for Task 039-2 and state that in 044's build order (044:3357-3363), which does not list 044-42 today.

No finding, after looking:
- **Tenant boundary.** `NESTED_RUN_NOT_MOVABLE` and `NESTED_COLLECTIONS_INACTIVE` are reached only after authorization and resolution (lines 214-215 and 890-891). Side rows are inserted only for a live header of the same tenant and namespace. No metric label carries tenant data.
- **Secrets and personal data.** Nothing new reaches the journal, logs, audit or errors; the new reasons are closed codes.
- **Least privilege.** Apart from findings 1 and 2: no PostgreSQL row-level security exists (no `ROW LEVEL SECURITY` in the migrations), so the definer's owner rights bypass nothing beyond grants. No migration function updates `runs`, so the owner exemption is in practice the copy alone. The copy and purge pin `search_path` to `pg_catalog, pg_temp` with schema-qualified relations.
- **Injection and parsing.** No dynamic SQL. The bounds saturate at 4,097.
- **Races.**
  - The three checks make a flagged run without a role, a `ROOT` run with a share, and an unflagged member child unrepresentable. A store-bound `ROOT` on a successor of a member child fails closed: `ROOT` plus a share breaks check 2.
  - The `chain_segments` foreign keys bind the predecessor to the same tenant, namespace and chain.

Suspicion (not verified, carried from reread-4 and not addressed): under `PROPAGATE` the parent does not wait for a cancelled child to settle (crates/domain/src/workflow.rs:569-570). A retried member child can therefore overlap a cancelled one that still holds admitted iterations, briefly exceeding `reserved_bound`; possession still limits leasing to one member. Settling it needs a test in Task 039-22.
