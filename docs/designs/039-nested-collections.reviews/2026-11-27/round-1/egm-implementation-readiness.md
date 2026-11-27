# EGM implementation-readiness review of design 039, round 1 (2026-11-27)

Doc: docs/designs/039-nested-collections.md
Reviewed-commit: d4124db28ec4afd3cb27ad1fe37ea9bfe23d72a5
Reviewer: egm-implementation-readiness
Note: the reviewer's output as returned, with this header added and machine paths made relative. Stage 9 of the EGM
review with the codebase-details lens, run once as the `egm-implementation-readiness` agent of the accepted EGM plugin,
with no conversation history. It read the revision that answers the security, correctness and EGM design reviews.

Counts: Blocker 1, Major 12, Nit 6

1. Blocker — No task builds starting a child workflow from inside a body, and today's child-start path only handles root-path steps
   Where: lines 109-118, 435-449, 620, 818-832 (Task 039-5), 867-871 (Task 039-10)
   What: Requirement 039-001 and the codec table accept a child workflow inside a body. Task 039-5 only changes `link_child_chain` to accept "any instance path", and Task 039-10 only opens the codec. The rest of the child path only handles root-path steps:
   - `pending_children` (`crates/infra-postgres/src/maintenance.rs:542-555`) filters `cardinality(instance_path_nodes) = 0`.
   - `start_child` (`crates/scheduler/src/passes.rs:6152` onward) looks the parent node up with the root lookup `parent_definition.node(node)` and reports the outcome with `finish_with(.., node, ..)`.
   - `child_started` (`crates/infra-postgres/src/execution.rs:4175-4194`) keys the parent step by (run, node) only.
   - The parent reference on the child run carries only `parent_node`.
   - A child's chain is created in `PgRunTx::commit` (`crates/infra-postgres/src/execution.rs:5652-5676`), and only when `self.def.continuation_policy()` is `Some`. A child without `chainPolicy` therefore never gets the one-segment chain the design relies on.

   The design also never says where in that commit the group `FOR UPDATE NOWAIT` and the member insert go (they must come after `initialize_chain_in_transaction`, because the member row needs the chain row), or how the child run's `capacity_member` value reaches the run INSERT.
   Why: a builder of Task 039-5 has no place to put the join, and Task 039-10 would open a shape the runtime cannot start. Feature 014 moved "the broader body families" to this design (`specs/014-workflow-primitives/tasks.md:63-68`), but the plan treats them as already built.
   Correction: add a lane-A task covering:
   - finding and starting pending children at non-root paths: the query, `PendingChild` carrying the instance path, the parent reference carrying the parent instance, finish and cancel of a path instance;
   - the commit branch that creates a chain for a member child with no policy, with the deadline rule of Technical Plan 2;
   - the exact place of the group lock and member insert in `PgRunTx::commit`;
   - binding `capacity_member` at the run INSERT.

   For a collection or loop owned at a non-root path, either cite the existing support with evidence or add the task.

2. Major — The lane split does not assign many of the files the tasks must change
   Where: lines 772-780; Tasks 039-7, 039-10, 039-17, 039-18, 039-21
   What:
   - `crates/application` is split only into "the two lease-producer files" (lane A) and "the definition service" (lane B). The guard needs an instance path, which changes the port method `worker_capacity_available` (`crates/application/src/execution.rs:924` and `:1689`). The scoped commands need new ports beside `crates/application/src/capacity_*.rs`. `Busy` and `NESTED_COLLECTIONS_INACTIVE` need error variants. `nested-collections-v1` needs a `Capability` value in `crates/application/src/instances.rs`. None of these files has an owner.
   - The publication path's version insert is in `crates/infra-postgres/src/definitions.rs:547`. That file is lane A's by the rule, but lane-B Tasks 039-10 and 039-21 must bind `nesting` and `fan_out_bound` in it.
   - `apps/execution-engine` and `apps/control-plane` (the switch, the capability listing, the sweep settings, wiring `with_child_root_capacity_guard`) and `crates/api-http` (mapping the new error codes) belong to no lane.

   Why: two lanes will edit the same files or leave them unedited.
   Correction: list every file each lane owns. Either move the `definitions.rs` change to lane A or allow lane B to edit it. Name the owner of the application ports, the error types, `instances.rs`, `apps/*` and `crates/api-http`.

3. Major — Several order points between lanes and with design 053 are missing, and one function belongs to two tasks
   Where: lines 778-780, 790-806 (Task 039-2), 807-812 (Task 039-3), 898-906 (Task 039-18), 926-930 (Task 039-23)
   What:
   - **Duplicated function.** Task 039-2's migration and Task 039-18 both create `nested_collections_required(mode)`.
   - **Missing dependency on Task 053-5.** The function reads the requirement row's `active`/`draining` state. `platform_requirements` has no `state` column today (`crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql:49-59`); Task 053-5 adds it. Only Task 039-18 waits, and it waits for Task 053-9.
   - **Missing dependency on Task 053-6.** Task 039-23 needs the mixed-writer suite of Task 053-6, which is not built.
   - **Lock-sequence checker against code that does not exist yet.** Task 039-3 (slice 1, lane B) writes `worker-lock-sequence.json` and the ordered-marker check for the creator, the join and the scoped commands, which lane A writes later. No marker syntax is defined, and nothing says whether the check passes before the markers exist.
   - **Task 039-7 not ordered after Task 039-4.** Task 039-7 needs Task 039-4's scoped command rules and `FinishPath` proof, but only Tasks 039-5 and 039-8 are ordered after 039-4.
   - **New version columns break today's inserts.** If the new `workflow_versions` columns are `NOT NULL` without defaults, both existing version inserts fail from the merge of Task 039-2 until Task 039-21 lands.

   Why: each gap gives either a red CI between merges or a function written twice in different forms.
   Correction:
   - Give the function to one task.
   - Add "after Task 053-5" to that task and "after Task 053-6" to Task 039-23.
   - Define the marker syntax, for example `// lock-seq: <contract id>/<step>`, and say which lane-A task adds each marker. Have Task 039-3 start with only the guarded-claim sequence, or let the check skip sequences whose function does not exist yet.
   - Order Task 039-7 after Task 039-4.
   - State defaults: `nesting boolean NOT NULL DEFAULT false`, and the `fan_out_bound` rule (see finding 9).

4. Major — `capacity_group_yields` and `capacity_group_join_waits` have no keys, and a natural foreign key would make their "no header lock" inserts wait
   Where: lines 444-447, 486-491, 796 (Task 039-2), 931-935 (Task 039-24)
   What: only the column lists are given. There is no primary key or unique constraint, which `ON CONFLICT DO NOTHING` needs. ADR-0156 says "at most one row per group"; a key of (tenant, group) and a key of (tenant, group, epoch) behave differently when a stale-epoch row exists. Every other capacity table has a composite foreign key to `capacity_groups`, so a builder will add one. That foreign key's check takes `FOR KEY SHARE` on the header, which conflicts with the `FOR UPDATE` the holder's guard keeps through its lease commit. The sweeper's yield insert and the creator's join-wait insert would then wait on the header. ADR-0075 already names this exact behaviour ("the foreign-key check of that insert waits unconditionally behind a worker's lock on G"). The `NOWAIT`/`SKIP LOCKED` source check cannot see it.
   Why: it breaks ADR-0078 rule 1 ("nothing waits on the header") and undoes the reason the yield moved off the header.
   Correction:
   - Write the full DDL: the primary key (for example `(tenant_id, group_id)` for both), no foreign key to `capacity_groups` (or say explicitly that it is omitted, and why), and who deletes the rows (`Vacate`, `Acquire`, a successful join, `capacity_purge_retired`).
   - Add a held-header test for both inserts.
   - Name the "partial indexes the sweeper's selections need" (line 802) with their columns and predicates.

5. Major — Task 039-2's column and grant list does not match the Technical Plan
   Where: lines 172-174, 462-466, 474-477, 790-806
   What:
   - Requirement 039-008 and Technical Plan 3 have the insert trigger set a header `shard` "recorded on the header at creation". `capacity_groups` has no `shard` column (`20261210000000_capacity_finish_storage.up.sql:4-24`), and Task 039-2 does not add one.
   - Technical Plan 2's grants give `UPDATE` on the member's `last_leased_at` and `last_vacated_unleased_at` only. Task 039-7's commands also write the member's `acquired_at` and the header's `held_since` and `transfer_count`. Today's header grant is only `(revision, epoch, active_members, possession, holder_member_id)` (`20261210000000_capacity_finish_storage.up.sql:176`). Task 039-2 says only "the `UPDATE` grant extended to them".
   - Types and defaults of `owns_unit`, `transfer_count`, `held_since` and `acquired_at` are not given.

   Correction: add `shard smallint NOT NULL`, set by the trigger from the existing SQL `run_shard(root_run_id)` (the same function as `Shard::of`, `crates/domain/src/sharding.rs:46`). Give one exact grant list per table and column, and the type, nullability and default of every new column.

6. Major — The deletion order for retired groups can never be carried out
   Where: lines 176-184, 586-597, 862-866 (Task 039-9)
   What:
   - The receipt trigger allows deleting a kept receipt only "of a group whose root run row no longer exists".
   - The header gains a foreign key `root_run_id` → `runs`, so the run row cannot be deleted while the header exists.
   - Receipts reference the header, so the header cannot be deleted before its receipts.
   - Feature 020's purge does not delete the run row at all: it "turns the run row into a stub" (`docs/constitution.md`, Principle I).

   So `capacity_purge_retired` can never delete the latest 16 receipts, and therefore never the header or the members.
   Why: Task 039-9's test of `capacity_purge_retired` cannot pass as specified.
   Correction: base the trigger's exemption on something `sagat_app` cannot fake, for example the run's stub or purge state as written by the retention role. Or let the trigger allow the delete when `current_user` is the owner role that runs the definer function. Then state the delete order inside the function: receipts, then members, then header.

7. Major — Which table `FinishPath` writes its receipt to is ambiguous
   Where: lines 515-516, 800 (Task 039-2), 839-841 (Task 039-7)
   What: Technical Plan 4 gives `VacatePath`, `AcquirePath` and `FinishPath` "a receipt table of their own (`capacity_path_transfer_receipts`)". Task 039-2 grants `INSERT` "on the scoped columns of the finish receipts", which points to `capacity_group_finish_receipts`. That table already has the `SCOPED_PATH` discriminator and checks (`20261212000000_scoped_finish_binding.up.sql:64-101`), and `scoped_leaf_finish.rs:562` writes there. The retention triggers also depend on which tables exist.
   Correction: state that `FinishPath` writes `capacity_group_finish_receipts` (kind `SCOPED_PATH`) and that `capacity_path_transfer_receipts` holds only `VACATE`/`ACQUIRE`, or the reverse. Give the new table's DDL, mirroring `capacity_group_transfer_receipts` with the scoped binding and a nullable `audit_id`.

8. Major — `last_leased_at` would be written when the guard approves, before the lease is actually granted
   Where: lines 146, 160, 536-538, 572-574, 844-847
   What: in both lease producers the guard runs before admission checks that can still skip the step without a lease:
   - `crates/application/src/worker/scoped_claims.rs:239-253`: after the guard come `activity_admitted`, `compensation_admitted` and `script_execution_admitted`, each of which `continue`s;
   - `crates/application/src/worker.rs:1413-1453`: after the guard comes `database_claim`.

   Writing `last_leased_at` "when it grants", inside the guard, records a lease that was never made. The skip of a member "acquired and vacated without a lease" then fails for exactly the case it exists for: a member held back by an activity quota or a database refusal. The checked lock sequence fixes the member-row write right after the group lock and before the quota locks, which commits the code to that early write.
   Correction: write `last_leased_at` once per claim transaction, after the first `LeaseGrant` of a member step is persisted, while the group lock is still held. Move the member-row entry in `worker-lock-sequence.json` after the quota, breaker, database and script locks. Add a test: "acquired, refused by the activity quota, vacated" is skipped by the next `Acquire`.

9. Major — Versions stored before the migration get no fan-out bound, and the policy's write path is misidentified
   Where: lines 213-217, 799-800, 919-925 (Task 039-22)
   What:
   - Every version "records its own bound at insert", but versions stored before the migration get none, and the bound needs the parsed document, so SQL cannot backfill it. A pinned child of such a version has no "recorded bound". The design-critique review raised this (`egm-design.md#7`), and the revision answers it only for new inserts.
   - `iteration_policy_change` (`crates/infra-postgres/src/quotas.rs:97-107`) only takes an advisory lock. The audited write of `max_active_iterations` is `put_setting` with `QuotaResource::IterationConcurrency` (`quotas.rs:979-1015`), which also deletes the whole `tenant_collection_policies` row when the value is `None`. That would drop `nested_group_fan_out` with it.
   - Nothing says what bound applies when a tenant has no policy row.

   Correction:
   - Define a backfill, either a one-run Rust backfill or "a version without a recorded bound counts as its root collections' `maxConcurrency` product, computed at read".
   - Add a `QuotaResource` value for the fan-out policy, with its API and contract entry and its audit action.
   - Change the `None` branch to reset only its own column.
   - State the value used when no policy row exists (256).

10. Major — The unit invariant cannot be written as SQL, because every admitted iteration must be marked slot-held
    Where: lines 71, 225-230, 911-913 (Task 039-20)
    What: `collection_iterations_slot_consistent` (`20261029000000_collection_progress.up.sql:263-265`) and the matching `loop_iterations` check (`20261107000000_loop_drain.up.sql:59-70`) force `slot_held = true` for every `ADMITTED` iteration. Descendant (inner) iterations, which charge no unit, are therefore slot-held too. Nothing links an inner iteration to the group that covers it. So "slot-held iterations without a group" is undefined, and Task 039-20's check cannot be written.
    Correction: either relax the checks to allow `ADMITTED AND NOT slot_held` for a descendant, in the Task 039-2 migration, or define "without a group" as "no ACTIVE or FINISHED scoped-path member of a non-retired group covers the iteration's root ancestor path". Give the check's SQL in the task.

11. Major — The new error codes have no contract
    Where: lines 113-115, 199, 405-408, 627-628, 868
    What: `NESTED_COLLECTIONS_DISABLED`, `NESTED_COLLECTIONS_INACTIVE`, `NESTED_CHILD_DETACH` and `NESTED_FAN_OUT_LIMIT` are named without:
    - an HTTP status or a `retryable` value (Principle VI);
    - the contract file that lists them;
    - which code a publication gets when the requirement is recorded but `draining` (Technical Plan 8 says "or its fence answers no" without a code);
    - how `NESTED_FAN_OUT_LIMIT` reaches the author at a child's admission: that refusal happens in a scheduler pass, so it must be a closed `FailureCode` on the parent's child step, and whether that failure is permanent is not stated.
    Correction: add a table of code, surface (publication, activation, admission, child step), status, `retryable` and contract file. Name the failure code and its permanence for the child-admission refusal.

12. Major — Task 039-6 leaves the shared predicate's form and the poll's rotation state to the builder
    Where: lines 162-166, 554-563, 833-838
    What:
    - **Form not chosen.** "One SQL fragment ... used by candidate selection" does not say whether it is a Rust `concat!` macro like `capacity_window!`, a SQL function or a view.
    - **Self-comparing test.** If `claimable_query!` (`crates/infra-postgres/src/workers.rs:79-177`) is refactored to embed it, the differential test compares the fragment with itself again (the security review's `security.md#2`).
    - **No scoped clause in today's window.** The current window clause (`workers.rs:44-55`) is ChildRoot-only and judged per run, while the scoped guard is per instance path.
    - **Rotation state undefined.** The "rotating keyset cursor" for the sample of ineligible runs has no stated home (process memory, worker session or a row) and no key.
    Correction: state that `claimable_query!` keeps its own text and gains a per-path scoped clause, that `runnable_member_work` is a separate macro used only by the sweeper, and that the differential test runs both. Name the cursor's home and its key, for example per process and (tenant, namespace), on `(ready_at, run_id)`.

13. Major — The lease fence trigger is under-specified, and it may contradict Non-functional requirement 039-002
    Where: lines 136-139, 238-244, 646-653, 926-930 (Task 039-23)
    What:
    - **Trigger events.** Steps can be inserted with a status taken from a bind (the first `INSERT` in `specs/014-workflow-primitives/contracts/step-status-writers.json`, `crates/infra-postgres/src/execution.rs`), so `UPDATE OF status` alone may not cover every path to `LEASED`.
    - **Cost for runs that are not members.** Reading `runs.capacity_member` adds a lookup to every lease of every run. The design-038 budget suite is never named, so it is unclear whether it counts trigger-internal statements.
    - **The root run's flag.** A scoped-path member's run is the root run, so the root run must be flagged, but when that happens is not stated. Once it is flagged, every lease of its top-level steps, which Requirement 039-003 says are "not gated", still needs `SET LOCAL sagat.capacity_guard`. On a process whose gate is closed, the root run's top-level steps would therefore be refused.
    Correction:
    - State `BEFORE INSERT OR UPDATE OF status`.
    - Say how the flag is read: a lookup on `runs`, or the flag copied onto `steps`.
    - Name the budget suite (`integration_claim_transition_efficiency`) and whether it counts trigger statements.
    - State that the root run is flagged in the transaction that creates its first group, and that the claim sets the setting for every lease of a flagged run, including top-level steps.

14. Nit — Technical Plan 1 names the guard function wrongly
    Where: line 391
    What: `crates/infra-postgres/src/execution/worker_capacity.rs` defines `available`. `worker_capacity_available` is the port method (`crates/application/src/execution.rs:924`, `:1689`), implemented at `crates/infra-postgres/src/execution.rs:5633` and `crates/infra-postgres/src/execution/path.rs:348`.
    Correction: name the port method and the module function separately.

15. Nit — Two functions are cited in the wrong place
    Where: lines 479, 545
    What: `plan_scope_compensation` is in `crates/domain/src/execution/scoped_sagas.rs` and is called from `crates/scheduler/src/passes.rs`. `tenant_weights` is a store method (`crates/application/src/shards.rs:71`) used by `crates/scheduler/src/shards.rs:232`. Paging per tenant by weight already exists as the scheduler's `pick` helper (`passes.rs:6111-6120`).
    Correction: cite the defining files, and `pick` for the paging.

16. Nit — The fan-out product can overflow a 32-bit column
    Where: lines 210-214
    What: four levels of `maxConcurrency` up to 256 (`iteration_scopes_limits`, `20261029000000_collection_progress.up.sql:167-170`) multiply to 2^32, which overflows `integer`.
    Correction: store `fan_out_bound` as `bigint`, or saturate at 4,097, and state which.

17. Nit — The activation check's ban also covers enabling the guard
    Where: lines 579-580, 807-812
    What: `ACTIVATION_CALL` (`scripts/check-capacity-activation.py:15-17`) bans `.with_child_root_capacity_guard(` as well as the three commands. The allow-list names only the sweeper as caller, so Task 039-17's engine wiring would fail the check.
    Correction: add the engine composition's guard enablement to the allow-list, naming the file.

18. Nit — Label values, settings and suites are not closed or named
    Where: lines 246-248, 506, 671-680, 943-944
    What:
    - `capacity_sweep_commands_total{operation, outcome}` has no list of label values.
    - `capacity_members_unfinished` counts "10 ticks", which nothing records; a duration would be measurable.
    - The quantum setting has no environment variable name.
    - "The three checkers" are not named.
    Correction: list the label values, use "10 × tick" as a duration, name the quantum variable (for example `SAGAT_CAPACITY_QUANTUM_MS`), and name the checkers.

19. Nit — The design index shows the wrong task count
    Where: `docs/designs/README.md:15`
    What: it lists "0 of 22"; the doc has 24 tasks (T039-1 to T039-24).
    Correction: update the row.
