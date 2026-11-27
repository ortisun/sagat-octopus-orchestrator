# Third security re-read of design 039 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: 6b3e218c7c2336506c0d7998e2a919fc2dfdd7c4
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 2, Nit 2

## Previous re-read

1. Code workflows escape the fan-out bound: **partly fixed.** Lines 235-239 and 1235-1237 now give a code version a bound and refuse one that lists children, both when it is pinned at publication and when it is admitted as an active-version child. A code run that is already a member can still reach a version that lists children by a move or by continue-as-new. Repeated as finding 1.
2. The header's `owns_unit` check cannot be evaluated: **fixed.** Lines 172-186 add `root_owner_node` and `iteration_index`, the slot check against `collection_iterations` or `loop_scopes`, the partial unique index and the member namespace check. The grant at line 613 and Task 039-2 at lines 1051-1054 match.
3. The runtime role cannot run the join-wait upsert: **fixed.** The column grant is at lines 624-626, and Task 039-24 runs its test with `SAGAT_TEST_AS_RUNTIME_ROLE=1` (lines 1257-1259).
4. The lease fence keys on status instead of the lease token: **fixed.** The fence now keys on a new token (lines 879-888), with tests for a renewal, a cancellation, `LEASED`→`RUNNING` and a direct grant into `RUNNING` (lines 1242-1248).
5. The reservation and the share fail open at their edges: **partly fixed.**
   - (a) A null bound now raises (lines 188-190 and 604).
   - (b) `resolve` gets the default-limit case and a test against the SQL rule (lines 256-260).
   - The share is now write-once (lines 249-251).
   - (c) The null-share refusal (lines 248-249) contradicts the member-child test that finding 6's fix uses. Repeated as finding 3.
6. The relaxed slot check accepts an uncharged root iteration: **fixed in the text** (lines 291-295, 1062-1063 and 1202-1204). Its test for a member child depends on the discriminator in finding 3.
7. The `new_use` fence misses run-start paths and has no order against authorization: **fixed.** It is called once by the store's run creation, after authorization, and covers a reset and a rerun (lines 211-216, 858 and 1192-1195).

## Findings

1. Major — A code member child can move, or continue as new, into a version that lists children, and so escape the fan-out bound and the reservation (finding 1 of the previous re-read, partly fixed)
   Where: docs/designs/039-nested-collections.md:235-251, 277-283, 589-593; docs/designs/044-workflow-as-code.md:334-337, 377-389, 1968-1970
   What (verified in the text of both designs):
   - Design 039 checks a code version's `children` list only at publication of a pinning parent and at the admission of an active-version child.
   - Design 044 (Approved) gives a running code run two other ways to change version:
     - A move by a person with `run:operate` and `version:activate` re-pins an open run to another published code version of the same workflow. The move only checks that the steps that can still act fit the target envelope; nothing in that check limits the target's `children` list.
     - A continue-as-new may ask for the workflow's active version.
   - Scenario: a root `forEach` (concurrency 1) calls code version K1, whose `children` list is empty, so its bound is 1 and the admission passes. The author then publishes K2, which lists graph child G (a root `forEach` of concurrency 256), and moves the K1 run to K2, or K1 continues as new with `version: active`. The successor gets the flag and the share through the `chain_segments` copy, and nothing checks K2's bound. K2 starts children activation after activation, up to the tree budget of 1,024 runs (044:1969). Each G is checked alone against K2's share and passes. The result is up to about 262,144 live iterations in one group that reserves 1.
   Why: one tenant author can exceed Requirement 039-014, the `nested_live_iterations` ceiling and Owner decision 2026-11-04-6, on replicas shared with other tenants.
   Correction:
   - Add a `BEFORE UPDATE` trigger on `runs` that refuses a change of version of a `capacity_member` run unless the new version is closed (`NOT fan_out_open`) and its `fan_out_bound` is at most the run's `fan_out_share`.
   - Apply the same check in the `chain_segments` copy trigger when the successor's version differs from its predecessor's.
   - Simpler for 0.3.0: refuse a move, and a continue-as-new to another version, of a flagged run.
   - Add a test for each path.

2. Major — The reservation is taken at the policy in force when the group is created, but a child's share is computed from the policy in force when the child is admitted
   Where: docs/designs/039-nested-collections.md:243-246, 255, 261-267, 614-619
   What (verified in the text):
   - For a version that calls an active-version child (`fan_out_open`), the group's `reserved_bound` is "the effective policy value" at group creation.
   - The share of each child is "the effective policy value divided … by the product", read when that child is admitted.
   - A tenant administrator may lower the policy and reset it to 256 (the `None` branch).
   - Scenario: the tenant sets `nested_group_fan_out` to 1. It admits up to its tenant limit L of groups of an open version; each reserves 1, and every root admission passes because the bound of an open version is relative to the policy. It then clears the setting, so the policy is 256 again. Every child admitted afterwards gets a share of 256 divided by the path product. With L = 1,024 that is again the 262,144 live iterations that the reservation was added to prevent (line 266), against a reservation of 1,024.
   Why: the reservation of Requirement 039-014 can be undone by the tenant's own setting, with no operator involved.
   Correction:
   - Compute a child's share from the group's `reserved_bound`, which is immutable after insert, never from the current policy: share = `reserved_bound` divided by the path product.
   - Add a test: lower the policy, admit, raise the policy, then admit a child and see it bounded by the reservation.

3. Nit — `fan_out_share` cannot both mark a member child and fail closed on a null share (finding 5(c) of the previous re-read, partly fixed)
   Where: docs/designs/039-nested-collections.md:246-251, 291-295, 594-595, 629, 1238-1239
   What (verified in the text):
   - A root run that holds scoped-path members is flagged (line 594) and has a null share: line 246 says the share is null on a run that is no member, and line 294 uses `fan_out_share IS NOT NULL` to mean a member child.
   - Line 248 refuses an active-version child "whose parent run is flagged while its `fan_out_share` is null". Read literally, that refuses every active-version child called from a root iteration, which is User story 039-01.
   - If the root run is given a share to avoid that, the slot trigger at line 294 treats it as a member child, and an uncharged root iteration of that run is accepted again (finding 6 of the previous re-read).
   - The write-once rule lets any `UPDATE` set a value from null at any time, and `sagat_app` holds that grant (line 629). So the marker is also not fixed at insert.
   - It is also not stated whether a pinned member child gets a share. Grandchildren of a pinned child need one.
   Correction:
   - Use a discriminator of its own that is set at the run's `INSERT` and never changed, for example `capacity_role` with the values `ROOT` and `MEMBER_CHILD`. A successor gets it from the segment copy.
   - Refuse a child only when its parent is a `MEMBER_CHILD` with a null share.
   - Give every member child a share, pinned or not.
   - Allow the null-to-value write of `fan_out_share` only from the segment-copy trigger (`pg_trigger_depth() > 0`).

4. Nit — The architecture check does not require version inserts to bind `fan_out_open`
   Where: docs/designs/039-nested-collections.md:608-610, 614-617, 630, 1021-1023, 1215-1218
   What (verified in the text):
   - `reserved_bound` depends on `fan_out_open` (line 615), and the column defaults to false.
   - The rule in `scripts/check-architecture.py` (Task 039-21) refuses a version insert that does not bind `nesting` and `fan_out_bound`, but not one that omits `fan_out_open`.
   - A version-insert path that forgets the column records an open version as closed. Its groups then reserve the bound recorded at publication rather than the policy value.
   Correction: extend the rule and its negative fixture to require `fan_out_open` as well.

No finding, after looking:
- Tenant boundary:
  - Side rows are inserted only `WHERE EXISTS` a header of the same tenant and namespace.
  - The header takes `namespace_id` from the root run, and a member must share its header's namespace.
  - The new 409 comes after authorization and tenant resolution, so a caller without permission gets today's 404 or 403.
  - Metric labels are closed sets with no tenant label.
- Secrets and personal data: no new payload reaches the journal, logs, audit or errors. The retirement record carries only `transfer_count`.
- Least privilege: the column grants cover every upsert, and there is no `DELETE` on headers or members. `capacity_purge_retired` is owner-only, gated on feature 020's authorization row and written with schema-qualified relations. The retention and delete-guard triggers pin their `search_path`.
- Injection and parsing: no dynamic SQL. The ad hoc path cannot carry a collection: `validate_definition` in crates/application/src/runs/ad_hoc.rs:612-639 allows only database operations, timers, conditions, `parallel` and `end`.
- Races:
  - Graph continue-as-new keeps its version (crates/infra-postgres/src/chain_progress.rs:1169, 1278), so the successor copy is sound for graph runs.
  - Phase 2's `FOR UPDATE` waits for every `FOR KEY SHARE` taken by a fence caller, and phase 1's update does not conflict with them.
  - The lease fence's setting is bound to the running transaction.
- Registers: Decision 039-032 is in specs/039-nested-collections/research.md:42, and Owner decisions 2026-11-27-459 to -463 are in docs/owner-decisions.md:67-71.
