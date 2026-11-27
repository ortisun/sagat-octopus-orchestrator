# Fourth security re-read of design 039 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: 9065e03513bd5257857330c940caf61a1ff548ef
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 2

## Previous re-read

1. A code member child can move, or continue as new, into a version that lists children: **partly fixed.** The rule is now in the text at lines 291-303, 609-613, 652-655 and 885, with its tests in Task 039-19 (lines 1229-1235). But the move half of it depends on design 044's table and code. That table does not exist yet, and 044 does not mention the rule. The plan has no order point and no lane for it. Repeated as finding 1.
2. A child's share was read from the policy in force at the child's admission: **fixed.** The share is now the group's fixed `reserved_bound` divided by the path product (lines 243-247), with its test (lines 1257-1258 and 1274-1276) and ADR-0156:177-178. Two older texts still say "policy divided"; that is finding 3.
3. `fan_out_share` could not both mark a member child and fail closed on a null share: **partly fixed.** `capacity_role` is added (lines 247-252, 312-315, 615-616 and 630), every member child gets a share (lines 252-253), and the null-share refusal is keyed on a `MEMBER_CHILD` parent (lines 255-257). But the exemption predicate `pg_trigger_depth() > 0` (lines 252 and 258) is always true inside the guard trigger itself. That predicate was my own correction in reread-3 (security.md:66), and it was wrong. Nothing ties the flag, the role and the share together. Repeated as finding 2.
4. The architecture check did not require version inserts to bind `fan_out_open`: **fixed.** The rule and a negative fixture for each of the three columns are at lines 493-494, 1049-1050 and 1251-1252. Some wording is stale but has no security effect: "binds both" at line 633, the lane table at line 1054 naming only `nesting` and `fan_out_bound`, and the defaults list at lines 1058-1059.

## Findings

1. Major — The version fence on member runs rests on design 044's table and code, which are not built yet; the plan has no order point or owner for it, and 044 does not mention it
   Where: docs/designs/039-nested-collections.md:291-303, 654-655, 885, 1048, 1052-1055, 1057-1063, 1229-1235; docs/designs/044-workflow-as-code.md:377-389, 1765-1777, 1792, 2193, 2232, 2666 (Task 044-6), 2916 (Task 044-13), 3061 (Task 044-22)
   What (verified):
   - `code_run_pins` and `sagat_move_code_run` exist nowhere in `crates/`, `migrations/` or `infra/` (grep returns nothing). Task 044-6 creates them from 044's table row at line 2232, which lists no trigger.
   - Design 044 (Approved) never mentions `capacity_member`, `NESTED_RUN_NOT_MOVABLE` or design 039.
   - Its move skip reasons are a closed set: `REPLAY_MISMATCH`, `ENVELOPE_MISMATCH`, `RUN_NOT_MUTABLE` (044:1792, 2193).
   - Requirement 044-023 (044:377) says a person with the permissions MUST be able to move an open code run.
   - Design 039's order points (lines 1057-1063) name Tasks 053-5, 053-6 and 053-9 but no task of 044.
   - Line 1048 says every file has one owner, yet neither lane (lines 1054-1055) owns the move's skip check (`crates/domain/src/batch.rs`, `crates/application/src/runs/batch_items.rs`, per 044:2944) or the code completion's refusal.
   - Scenario: 039 is built and activated first. Task 039-19 cannot create a trigger on a table that does not exist, and the plan does not say what happens then. Later, Tasks 044-6 and 044-13 are built from 044's text, with no trigger on `code_run_pins` and no skip reason.
   - The reread-3 scenario is then open again. A code member child K1 (empty `children` list, bound 1) is moved to K2, which lists a graph child with a root `forEach` of concurrency 256. K2 starts children up to the tree budget of 1,024 (044:1969), in a group that reserves 1.
   - The `chain_segments` backstop does catch the continue-as-new path whatever the order, because that table exists. But without Task 044-22's completion check, the handoff raises in the continuation pass, and 044:333 gives no outcome for that error.
   Why: the correction of the previous Major exists only in 039's text. Whether it is ever built depends on which of the two designs lands first, and on whoever builds 044 reading design 039. Without it, Requirement 039-014, the `nested_live_iterations` ceiling and Owner decision 2026-11-04-6 can be passed by one tenant's author on replicas shared with other tenants.
   Correction:
   - Add an order point: the `code_run_pins` trigger of Task 039-19 lands in the change that runs Task 044-6, or after it.
   - Amend design 044 with a dated revision and add a line to Tasks 044-6, 044-13 and 044-22:
     - the trigger in 044's table row (line 2232);
     - `NESTED_RUN_NOT_MOVABLE` in the closed skip reasons (lines 1792 and 2193);
     - the completion refusal in Requirement 044-019.
   - Give those files an owner in the lane table.
   - Make the activation of Task 039-17 require the move and continue-as-new tests of Task 039-19 to pass whenever `code_run_pins` exists.

2. Nit — `capacity_role` and `fan_out_share` are not write-once as written, and a member child with a null role does not fail closed (finding 3 of the previous re-read, partly fixed)
   Where: docs/designs/039-nested-collections.md:247-259, 312-315, 615-616, 630, 652-654, 1228, 1272-1275
   What (verified in the text; the PostgreSQL behaviour is from its documentation):
   - `pg_trigger_depth()` returns 1 inside a trigger fired by a plain statement. So the guard on `runs`, testing `pg_trigger_depth() > 0`, passes every direct `UPDATE` that sets either column from null to a value.
   - `sagat_app` holds `UPDATE (capacity_member, fan_out_share, capacity_role)` (line 652). The claim at lines 652-653 that only the segment copy writes them after insert is therefore false.
   - The test at lines 1274-1275 ("a null-to-value update outside the segment copy refused") would fail against the trigger as written. That is good evidence, but the text prescribes the wrong predicate.
   - `pg_trigger_depth() > 1` would not be a boundary against `sagat_app` either. A temporary table's trigger can raise the depth (docs/designs/041-step-logs.reviews/2026-11-20-hot-row/security-reviewer.md:109; I did not check `sagat_app`'s `TEMP` privilege).
   - Separately, the fail-closed rule refuses a child only under a parent whose role is `MEMBER_CHILD` (lines 255-257). A flagged member child inserted with a null role by a defective writer is not `MEMBER_CHILD`, so it falls into the group-share branch meant for `ROOT`. Its grandchildren's shares would then be computed from the group's `reserved_bound` over a path product in the wrong run.
   - Nothing in the schema ties the flag, the role and the share together.
   Correction:
   - Make the `chain_segments` copy function `SECURITY DEFINER`, owned by the table owner, with `search_path` pinned and schema-qualified relations, as Technical Plan 7 does for the purge.
   - Revoke `sagat_app`'s `UPDATE` on `fan_out_share` and `capacity_role`; both are bound at `INSERT`. Have the guard allow the null-to-value write only when `current_user` is the table owner.
   - Add these checks on `runs`:
     - `NOT capacity_member OR capacity_role IS NOT NULL`;
     - `(capacity_role = 'MEMBER_CHILD') = (fan_out_share IS NOT NULL)`;
     - `capacity_role IS DISTINCT FROM 'MEMBER_CHILD' OR capacity_member`.
   - These hold for each writer: the successor copy sets all three in one `UPDATE`, a member child binds all three at insert, and a `ROOT` run is flagged later.
   - Key the runtime refusal on "the parent is flagged and not `ROOT`".

3. Nit — Two texts still say a child's share is the policy divided by the product, and no owner row records Decision 039-033's restriction of an Approved design
   Where: docs/designs/039-nested-collections.md:442 (the row of Owner decision 2026-11-27-454), 414-417; docs/owner-decisions.md:62; specs/039-nested-collections/research.md:43
   What (verified):
   - The design's own decision table (line 442) and Owner decision 2026-11-27-454 (docs/owner-decisions.md:62) still say "the child's share is the policy divided by the product". That contradicts Requirement 039-014 (lines 243-247). An implementer reading the table would rebuild the policy-based share that finding 2 of reread-3 removed.
   - Decision 039-033 refuses moves and continue-as-new for member runs, a behaviour that Requirement 044-023 of the Approved design 044 says MUST be available. No owner row records it: the 039 rows stop at 2026-11-27-463, and rows 464 onwards belong to design 042.
   - ADR-0156:172-179 does record both changes.
   Correction:
   - Add a delegate row refining Owner decision 2026-11-27-454 (share from `reserved_bound`; `capacity_role`) and recording the 0.3.0 exception to Requirement 044-023 and Requirement 044-019.
   - Update the row at line 442 and the range at lines 414-417.

No finding, after looking:
- **Tenant boundary.** `NESTED_RUN_NOT_MOVABLE` is reached only by a caller who already holds `run:operate` and `version:activate` on the run's namespace, after the run is resolved, so it is no oracle. The new refusals carry no tenant data, and no metric label was added.
- **Secrets and personal data.** The move item's reason is a closed code, and no new payload reaches the journal, logs, audit or errors.
- **Least privilege.** The `code_run_pins` trigger needs no grant, and it fires inside the definer function `sagat_move_code_run` as well. `reserved_bound` is immutable and set by the insert trigger.
- **Injection and parsing.** No dynamic SQL is added. Code versions cannot issue `forEach`, `while`, `repeatUntil` or `parallel` in 0.3.0 (044:583), and a child command must name a `children` entry (044:1441). So the bound of 1 for an empty `children` list holds.
- **Share arithmetic.** Dividing by the path product and the number of `parallel` branches never hands out more than `reserved_bound` in total; the asymmetric-branch case refuses too much, not too little.
- **Races.**
  - `capacity_member` only goes from false to true.
  - A code run is never `ROOT` (code versions are not nesting), so a code member is flagged at its own insert and the move and continue-as-new checks have no check-then-act window.
  - Resets of child runs and chain runs are refused (crates/application/src/runs/reset.rs:320-332).
  - A rerun creates a top-level run outside the group.
  - A graph continue-as-new keeps the chain's version (crates/infra-postgres/src/chain_progress.rs:1169).

Suspicion (not verified): with `PROPAGATE`, the parent does not wait for a cancelled child to settle (crates/domain/src/workflow.rs:569-570). If a member child's step fails by time-out and the parent's `onError` calls the child again, a new child with a full share can be live while the cancelled one still holds admitted iterations. The overlap would be bounded by the retry count and transient. Settling it needs a test in Task 039-22; I cannot settle it from the text.
