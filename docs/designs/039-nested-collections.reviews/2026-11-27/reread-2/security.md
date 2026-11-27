# Second security re-read of design 039 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: 7036dda695a196913a36819a60bbe2258c42556c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the reviewer's output as returned, with this header added and machine paths made relative.

Counts: Blocker 0, Major 1, Nit 6

## Previous re-read

1. Fixed. The bound now recurses through pinned children, adds up the branches of a `parallel` node, stores each child's share on `runs.fan_out_share`, is a tenant-only policy under an operator ceiling, is checked again at every root admission, and each group reserves its bound under `nested_live_iterations`. Lines 217-252, 1185-1199. A new way around it through code workflows is finding 1.
2. Fixed. The lease fence now refuses only a new grant, with tests that a renewal and a cancellation still pass. Lines 850-860, 1202-1207. A remaining gap is finding 4.
3. Fixed. A join wait names its waiting parent step and carries `last_busy_at`; only a live wait keeps a yield, and selection 4 deletes a stale one. There is a pause-after-busy-join test. Lines 512-521, 639-642, 1214-1216.
4. Fixed. The successor copy is an `AFTER INSERT` trigger on `chain_segments`. Lines 253-259, 564-568, 1163-1166. I checked that `predecessor_run_id` exists and cannot be changed: `crates/infra-postgres/migrations/20261102000000_execution_chains.up.sql:48,139-145`.
5. Fixed. Both insert triggers call the fence in mode `continue` and raise `nested_collections_fence`. Lines 180-181, 204-207, 452-456.
6. Fixed. Phase 2 takes no table lock; its `FOR UPDATE` on the requirement row waits for every fence caller. Lines 876-881.
7. Fixed. The function raises on an unknown mode, and a refused run start has its own row in the codes table. Lines 202, 455-456, 831. The list of run-start paths is incomplete (finding 7).
8. Fixed. The setting is `v1:` followed by `pg_current_xact_id()`, compared with the running transaction. Lines 855-860, 1206-1207.
9. Fixed as asked: the relaxed slot check applies only to flagged runs. Lines 265-270, 1166-1168. A remaining gap is finding 6.
10. Fixed. The purge function is revoked from `PUBLIC`, executable by its owner only, gated on feature 020's `retention_purge_authorizations` row (checked in `specs/020-visibility-analytics-archival/data-model.md:229-231`), and every relation is written with its schema. Lines 772-783, 1112-1116.
11. Partly fixed. `namespace_id` is now derived from the root run (line 176). The `owns_unit` check (lines 176-179) cannot be built as written. Repeated as finding 2.
12. Fixed. Side rows are inserted only `WHERE EXISTS` a non-retired header, and the retiring `Finish` deletes them. Lines 519-521, 548-552, 1215-1216.
13. Fixed. ADR-0156 has a dated revision that rules over the clauses it supersedes (`docs/adr/ADR-0156-...md:135-162`); D039-031 (`specs/039-nested-collections/research.md:41`) and Owner decisions 2026-11-27-459 to -463 (`docs/owner-decisions.md:67-71`) are appended, and the cited range is widened (lines 369-372).

## Findings

1. Major — Code workflows (design 044) escape the fan-out bound and the tenant-wide reservation
   Where: docs/designs/039-nested-collections.md:217-231, 241-246, 445-451; docs/designs/044-workflow-as-code.md:4, 17-26, 252-262, 1116, 1943
   What (verified in the text of both designs):
   - `fan_out_of` reads a graph document, and design 039 never mentions code versions.
   - Design 044 is Approved for 0.3.0. A code run can "start a child" by command: up to 256 commands per activation, with no limit across activations. A graph parent can call a code child (044:1943).
   - A code child started inside a nested body is a member child. Its children are admissions in a ChildRoot member's run, so they join the group and charge no tenant unit (Requirement 039-002).
   - Scenario: a root `forEach` with concurrency 1 calls a pinned code version K. K starts 256 children of a pinned graph version G, whose root `forEach` has concurrency 256. That is 65,536 live iterations in one group. The group's `reserved_bound` is K's recorded bound, which is undefined, or 1 by the rule "calls nothing and has no collection".
   Why: Requirement 039-014, the `nested_live_iterations` ceiling and Owner decision 2026-11-04-6 can all be passed by one tenant author, on scheduler replicas shared with other tenants.
   Correction:
   - Define `fan_out_of` for a code version. When its `envelope.children` is not empty, the bound saturates at 4,097 and `fan_out_open` is true, so publication refuses it as a pinned child under nesting.
   - At the admission of an active-version child, refuse a code version that lists children (`NESTED_FAN_OUT_LIMIT`).
   - Alternatively, refuse a code version as a member child in 0.3.0.
   - Add one test for each path.

2. Nit — The header's `owns_unit` check cannot be evaluated when the header is inserted (finding 11 of the previous re-read, still open)
   Where: docs/designs/039-nested-collections.md:176-179, 573-575, 587, 1021; crates/infra-postgres/migrations/20261210000000_capacity_finish_storage.up.sql:4-24, 36-37, 54-57; crates/infra-postgres/migrations/20261031000000_loop_progress.up.sql:4-57, 70
   What (verified in the code):
   - The header has no root owner node or iteration index.
   - The holder member is inserted after the header: the member's foreign key to the header is immediate, while the header's holder foreign key is deferred. So a `BEFORE INSERT` trigger on the header cannot tell which iteration the group belongs to.
   - `loop_scopes` has no `slot_held` column. During a delay, the loop's `loop_iterations` rows are `CHECKPOINTED` with `NOT slot_held`.
   - A trigger written as "some slot-held iteration of `root_run_id`" would let two `owns_unit` groups stand on one slot, so the unit is released twice.
   - Separately, nothing requires a member's `namespace_id` to equal the header's.
   Correction:
   - Add `root_owner_node` and `iteration_index` to the header, insertable but immutable.
   - Check them against the `collection_iterations` row (`slot_held`), or for a loop against the `loop_scopes` row (lifecycle `RUNNING` or `DELAYING`).
   - Add a partial unique index on `(tenant_id, root_run_id, root_owner_node, iteration_index) WHERE owns_unit AND possession <> 'RETIRED'`.
   - Have the member trigger require `member.namespace_id = header.namespace_id`.

3. Nit — The runtime role cannot run the join-wait upsert
   Where: docs/designs/039-nested-collections.md:513-515, 598-599, 1211-1213
   What (verified in the text): the join wait is written with `ON CONFLICT ... DO UPDATE SET last_busy_at = now()` plus the parent columns. The exact grant list gives `UPDATE` only on `capacity_group_yields (epoch, requested_at)`, and none on `capacity_group_join_waits`. PostgreSQL requires `UPDATE` on the assigned columns, so as `sagat_app` every busy join's wait insert fails. A busy holder can then starve a joining child again, or someone widens the grant ad hoc.
   Correction: grant `UPDATE (parent_run_id, parent_path_nodes, parent_path_indices, parent_node, last_busy_at)` on `capacity_group_join_waits`, and run Task 039-24's held-load test with `SAGAT_TEST_AS_RUNTIME_ROLE=1`.

4. Nit — The lease fence looks for `status = 'LEASED'`, not for a new lease token
   Where: docs/designs/039-nested-collections.md:851-856, 1202-1204; crates/infra-postgres/src/execution.rs:7103-7112; crates/infra-postgres/src/execution/path.rs:1245; crates/infra-postgres/migrations/20261009000100_execution.up.sql:160
   What: verified that the generic step writers can set a new `lease_token_hash` while writing `status = 'RUNNING'`. It is a suspicion, not verified, that some producer, today or in an older binary, grants straight into `RUNNING`; such a grant would pass the fence. `steps_lease_consistent` guarantees that every grant sets a token.
   Correction: refuse when `NEW.lease_token_hash IS NOT NULL AND (TG_OP = 'INSERT' OR NEW.lease_token_hash IS DISTINCT FROM OLD.lease_token_hash)`. That gives the same answer for renewals, cancellations and `LEASED`→`RUNNING`, and it also covers a direct grant into `RUNNING`. Add that test.

5. Nit — The reservation and the share have edges that fail open
   Where: docs/designs/039-nested-collections.md:224-238, 242-243, 589-593; crates/domain/src/quota.rs:654-660; crates/infra-postgres/migrations/20261009000100_execution.up.sql:273
   What (verified in the code):
   - (a) The group trigger cannot parse a document, and what it does with a null `fan_out_bound` is unspecified. A `COALESCE(..., 0)` would reserve nothing.
   - (b) `resolve` returns the ceiling when nothing is set; only `IterationConcurrency` is special-cased. So an operator ceiling of 4,096 silently becomes the effective value for every tenant that set nothing, contrary to "null and a missing row both mean 256". The SQL copy of the effective-policy rule in the trigger can drift from the domain's.
   - (c) What a grandchild check does when a flagged member run's `fan_out_share` is null is unspecified. `sagat_app` holds `UPDATE` on all of `runs`, so the share can also be rewritten after insert.
   Correction:
   - The trigger raises on a null bound.
   - Extend the `IterationConcurrency` default-limit case in `resolve` to `NestedGroupFanOut`, and test the SQL rule against `resolve`.
   - Refuse an active-version child when its parent is flagged and its share is null.
   - Make `fan_out_share` write-once with a trigger (null to a value only, which is how the segment copy writes it).

6. Nit — The relaxed slot check still accepts an uncharged root iteration in a flagged root run
   Where: docs/designs/039-nested-collections.md:265-270, 569-572, 1166-1168
   What (verified in the text): a root run is flagged when it creates its first group. After that, any root iteration of that run can be `ADMITTED AND NOT slot_held`, and the database accepts it, in exactly the runs where the creator is most complex.
   Correction: allow `ADMITTED AND NOT slot_held` only when the run is flagged and either the scope's owner path is not the root, or the run is a member child (`fan_out_share IS NOT NULL`).

7. Nit — The `new_use` fence misses some run-start paths, and the 409 has no stated order against authorization
   Where: docs/designs/039-nested-collections.md:202-209, 831; crates/infra-postgres/migrations/20270107000000_run_resets.up.sql:8-48; crates/application/src/runs/ad_hoc.rs:547, 574-575
   What:
   - Verified: run resets create new runs (`run_resets.new_run_id`, `NewRun.pending_reset`), and line 831 lists only API, schedule, batch and top-level child starts. While draining, a reset of a nesting-version run passes `continue` at its group insert, which defeats phase 1. After phase 2, its admission is refused on every tick, contrary to "this only follows an operator's removal error".
   - Suspicion: if the fence answers before authorization or version resolution, the 409 tells a caller without permission that the workflow's active version nests.
   Correction:
   - Call `new_use` once, in the store's run creation, for any run of a nesting version that is neither a member child nor a chain successor. That covers reset and rerun.
   - State that the check runs after authorization and tenant/namespace resolution, so such a caller gets today's 404 or 403.

No finding, after looking:
- Tenant boundary in API bodies, errors, metrics and timing: no new API surface beyond the quota endpoint, every row carries the tenant, labels are closed sets, and side rows check the header's tenant and namespace.
- Secrets and personal data in the journal, logs, audit and errors: nothing new is written.
- `SECURITY DEFINER` and search path: owner-only purge, schema-qualified bodies, and a pinned fence.
- Dynamic SQL, parsing and egress: none added. `maxConcurrency` is a static 1..256 (`crates/domain/src/workflow.rs:2243`).
- The `chain_segments` copy, phase 2's lock order and the replay of a purged receipt: no window.
