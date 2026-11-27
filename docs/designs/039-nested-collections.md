# Nested collections and the capacity program

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-27

--------------------------------------------------------------------------------

## Overview

General nesting of feature 014's primitives in release 0.3.0 (Owner decision 2026-11-24-1): a `forEach` whose body
holds another `forEach`, a loop, an approval or a child workflow, and the capacity program that makes nested admission
safe at any tenant limit, including 1. Feature 014 closed on root collections and handed this scope over (Owner decision
2026-11-12-6); its machinery is built and dormant (the capacity group, the native `Vacate`, `Acquire` and `Finish`
commands, bounded holder discovery, the ChildRoot worker-lease guard behind `child_root_capacity_guard`, off). This
design finishes it: it decides the items Architecture decision 0078 left open, adds the parts that are missing (the
creator of groups and members, the sweeper's caller and its coverage, the shared runnable predicate, the scoped-path
commands, fairness, receipt retention, the fence of the registry requirement), states the activation that the gate of
Architecture decision 0075 requires, and then opens the codec to nested documents.

The rules are [Architecture decision 0078](../adr/ADR-0078-capacity-group-lock-order-and-possession-protocol.md)
(accepted, delegate),
[Architecture decision 0154](../adr/ADR-0154-nested-capacity-membership-is-created-by-the-engine-and-the-sweeper-serves-every-member.md),
which decides its open items, and
[Architecture decision 0156](../adr/ADR-0156-a-loop-owns-one-capacity-group-every-member-child-has-a-chain-and-the-yield-is-a-row-of-its-own.md),
which amends both after the security and correctness reviews of 2026-11-27. The registers are in
`specs/039-nested-collections/research.md`; feature 014's registers (`specs/014-workflow-primitives/research.md`,
Decisions 701 to 715, Gaps 706 to 726) stay where they are and are cited. What each review changed is in the section
Revisions at the end.

### Terms

- **Capacity group (group).** The unit of tenant capacity that one root admission takes (`retained < limit`, Decision
  704): a header row `capacity_groups` with a possession state (`HELD`, `VACANT`, `RETIRED`), a holder, a revision, an
  epoch and `owns_unit`, true when the group holds the tenant unit its root admission charged.
- **Member.** A row of `capacity_group_members`: a **ChildRoot** member (a child workflow's chain started inside the
  group) or a **scoped-path** member: one iteration of a root collection (by run, root node and iteration index), or a
  whole root loop (by run and the loop's owner node, with the fixed iteration index 0), whose body nests work.
- **Possession.** The right of one member, the holder, to be granted worker leases (Owner question 706). Only the
  holder's work runs; the others wait. Nothing else is gated by possession.
- **Join.** A descendant becoming a member of its ancestor's group. A join charges no tenant unit (Decision 705), so a
  suspended ancestor cannot starve its descendants at any limit.
- **Yield request.** A row of `capacity_group_yields` that asks the holder to stop taking new leases so that possession
  can move (Technical Plan 3).
- **Sweeper.** The scheduler-tick caller of the commands (Decision 714, Owner question 705): level-triggered, bounded,
  one transaction per command.
- **Runnable predicate.** One SQL fragment that says whether a member has work a worker could be leased now, shared by
  the scheduler's candidate selection and by the sweeper (Gap 715).
- **Lease producers.** The two code paths that grant a worker lease: `crates/application/src/worker.rs` (a root path
  claim) and `crates/application/src/worker/scoped_claims.rs` (a scoped claim), both in the execution engine. The
  candidate window of `crates/infra-postgres/src/workers.rs` only proposes candidates; it grants nothing.
- **Fan-out bound.** The largest number of iterations of one group that can be live at once: the product of the local
  `maxConcurrency` down the nest (Requirement 039-014).
- **Quantum.** The time a holder keeps possession while another member waits before the sweeper asks it to yield (1
  second by default, Requirement 039-006).
- **Quiescent.** A member with no leased and no running step in its work: the state in which `Vacate` may take
  possession from it.
- **Join wait.** A row of `capacity_group_join_waits`, recorded when a child's join found the group header busy, which
  lets the sweeper ask the holder to yield so the join can go through (Technical Plan 2).
- **Registry requirement, serving gate, fence.** The operator records the requirement `nested-collections-v1` in the
  instance registry (ADR-0082); a process's serving gate answers whether it may use a capability (design 053,
  Requirement 053-006); a fence is the check in the database, here the owner-owned function
  `nested_collections_required(mode)` taking the requirement row `FOR KEY SHARE` (design 053's form (a), with the mode
  of Technical Plan 9).
- **Drain.** Removing the requirement in the two phases of Requirement 053-007: phase 1 marks it `draining`, phase 2
  checks that nothing depends on it and removes it.
- **Continuation chain.** The rows of `execution_chains` and `chain_segments` that bind a run and its continue-as-new
  successors; a ChildRoot member is bound to one.
- **Shard.** The scheduler shard of a run (`Shard::of(run id)`, `crates/domain/src/sharding.rs`); a group is served by
  the replica that holds its root run's shard.
- **Slot-held.** An iteration that holds a tenant unit (`slot_held` on `collection_iterations` and `loop_iterations`).
- **Register numbers.** A bare Decision, Gap or Owner question number below 1000 (Decision 705, Gap 724, Owner question
  706) is feature 014's (`specs/014-workflow-primitives/research.md`); this design's own are written 039-NNN. ADR-NNNN
  and "Architecture decision NNNN" name the same record under `docs/adr/`.

## The Problem

Feature 014 ships root collections, loops, approval, switch, declared operations and a restricted continue-as-new. A
nested document is refused by the codec (`crates/infra-ir/src/convert.rs`, `published_collection_definition`). The reason
is capacity: each root iteration takes one tenant unit; if an inner iteration or a child workflow started inside it
needed a unit of its own, an outer iteration holding its unit while it waits for inner work would deadlock the tenant at
a limit of 1 (ADR-0030 §14). Feature 014 chose to let descendants join their ancestor's unit (Decision 705) and to let
only one member of a unit lease work at a time (possession). The tenant limit then bounds groups, not iterations: inside
a group the inner collections are work of the member and are bounded by the group's fan-out bound (Requirement 039-014),
which this design adds so that a tenant at a limit of 1 cannot run thousands of inner iterations.

Much of that is built and dormant: the tables and the three native commands with their receipts (migrations
`20261210000000_capacity_finish_storage`, `20261212000000_scoped_finish_binding`, `20261225000000_capacity_discovery_index`,
`20261226000000_capacity_transfer_receipts`, `20261231000000_capacity_worker_membership_indexes`; `crates/infra-postgres/src/capacity_*.rs`), bounded holder discovery,
and the worker guard. What is missing is listed by feature 014 (`specs/014-workflow-primitives/tasks.md`, the "Current
frontier" notes; `capacity-*.md`), by the activation gate of ADR-0075 (conditions (a) to (l)), by the "Open" items of
ADR-0078 and by Gaps 715, 716, 718, 719, 720, 722, 724, 725 and 726 of feature 014. In short: nothing creates a group or
a member, nothing calls the commands, the sweeper would not finish non-holders or retire a vacant group whose members
are all final, scoped paths have no transfer commands, the guarded poll can starve a non-holder (Gap 724) or fill its
window with runs the claim refuses (Gap 725), fairness among members is not claimed (Gap 718), receipts grow without
bound (Gap 720), and the worker's complete lock sequence is not written where a check can read it. Two facts of the
code shape the answer: a ChildRoot member needs a continuation chain (`capacity_group_members.chain_id` references
`execution_chains`), which today only a child with `chainPolicy` started from the root path has
(`link_child_chain`, `crates/infra-postgres/src/chain_progress.rs`); and a root loop carries its one tenant unit from
iteration to iteration (`checkpoint` and `fire_delay` in `crates/infra-postgres/src/loop_progress.rs`).

Temporal runs child workflows inside loops without a tenant concurrency model; Windmill's for-loops nest without one.
Nesting that holds a tenant limit under a crash at any point is what this platform promises and neither of them does.

## Requirements

### Functional

- **Requirement 039-001** A `forEach` body MAY contain a `forEach`, a loop or a child workflow, to a depth of 4
  collection or loop levels, and a collection MAY stand beside loops, approvals and other nodes at the top level instead
  of only in a chain of collections ending at `end`; an approval inside a body opens only with feature 014's scoped
  compensation of approvals (Decision 295 of feature 014). A child workflow inside a nested body MUST NOT declare the
  cancel policy `detach` (`NESTED_CHILD_DETACH`, Gap 039-009). The definition service MUST accept such a document only
  when the requirement `nested-collections-v1` is recorded in the instance registry (ADR-0082) and its fence answers
  active (Requirement 039-011), and the codec MUST still refuse every shape this design does not list (Technical Plan 8).
  What an author sees: the members of one group, the root iteration (or root loop) and every child workflow started
  under it, lease work one at a time, whatever concurrency the inner nodes declare; inner iterations of one member run
  together within the fan-out bound. The reference and the release notes say so.
- **Requirement 039-002** A root admission of a version published with nesting (Technical Plan 8) MUST create a group
  with its first member in the admission's transaction: for a root collection, one group per root iteration; for a root
  loop, one group per loop, created at the loop's entry, with one scoped-path member for the whole loop that stays the
  holder from iteration to iteration (Technical Plan 2).
  Every admission made inside an ACTIVE member (a collection or loop iteration whose owning path lies within a
  scoped-path member's iteration, or any admission in the run of a ChildRoot member, a child's own collections included)
  is a descendant: it MUST join the group of its ancestor and MUST NOT reach the fresh-admission check or charge
  `tenant_iteration_usage` (Decision 705, ADR-0030 §14). Every child workflow started inside a member MUST get an
  execution chain, of one segment when it declares no `chainPolicy`, so that its membership is bound to a chain.
  Joining is bounded by the lifetime `maxInstances` budget, by the local scope slots and by the fan-out bound. Whether an
  admission is a descendant depends only on the run's `capacity_member` flag and the member that covers its path,
  whatever version the run executes: a child whose own version was published without nesting still joins, and its
  collections never ask for a fresh unit. An admission outside every member, in a version published without nesting (a
  top-level child workflow included), creates no group and behaves as today.
- **Requirement 039-003** Only the holder MAY be granted worker leases (ADR-0075 as built, Owner question 706), for a
  ChildRoot member's run and for the steps of a scoped-path member's iteration alike: both lease producers check
  possession by (run) for a ChildRoot member, by (run, root node, iteration index) for a collection's scoped-path member
  and by (run, root node) for a root loop's, with the instance path given to the guard, and a step of the root run
  outside every member is not gated. A process that the registry does not count, or whose serving gate answers neither
  active nor draining for `nested-collections-v1`, MUST grant no lease to a member's step; and the database refuses a
  step of a flagged run moving to `LEASED` in a transaction where the guard did not run (Technical Plan 9). Reports, heartbeats, timers, signals, retries, control nodes and child outcomes MUST NOT be gated by possession.
  Compensation steps under the path of a member that is already finished run outside possession (Technical Plan 4).
- **Requirement 039-004** Possession MUST move only by the fenced commands of ADR-0078 for ChildRoot members and by
  their scoped-path twins (Technical Plan 4), called only by the sweeper, each in its own transaction, holding no other
  lock or transaction of the caller. The header is written outside those commands only by the group's creation, by a
  join (compare and set of `revision`, `active_members` increased, under `NOWAIT`) and by nothing else; the retiring
  `Finish` releases the unit (Requirement 039-016). Outside the commands a member row is written only by the claim
  transaction of a lease producer, which sets the holder's `last_leased_at` once, after the first lease grant of a step of
  that member is persisted, while it still holds the `SKIP LOCKED` header lock (Technical Plan 4).
- **Requirement 039-005** The sweeper MUST serve every member state that can occur: a holder or non-holder whose head
  or iteration is final, whatever its outcome (succeeded, failed, cancelled, compensated, timed out, partial or
  compensation failed), with the last finish retiring the group (finish), a root loop's member only once the loop scope
  itself is final; a holder past its quantum while another member has runnable work, or while a join that is still live has waited for
  the header for at least a quantum (yield request); a join wait that is no longer live, and a yield request whose waiting members are no longer runnable and no live join waits (cleared); a
  quiescent holder whose yield was requested, or a holder with no runnable work while another member has runnable work
  (vacate); a vacant group with a member that has runnable work (acquire). A scoped-path member finishes by `FinishPath`
  (Technical Plan 4), which needs no certified success and serves a body that nests.
- **Requirement 039-006** No member MAY wait without bound while others run: when another member has runnable work and
  the holder has held possession for at least the quantum (`SAGAT_CAPACITY_QUANTUM_MS`, 1,000 by default, from 500,
  the sweeper's tick, to 60,000), the sweeper inserts a yield request, after which the guard grants the holder no new lease; once
  its leased and running steps end it is quiescent and is vacated, and `Acquire` chooses the member whose oldest
  runnable step became ready first, skipping a member that was acquired and then vacated within the last quantum without
  being granted a lease (its `last_leased_at` older than its acquisition). A yield request belongs to the epoch in which
  it was made: the guard and the sweeper act only on a yield row whose epoch equals the header's. The yield gates new leases only, never a report or a running step (Owner question 706).
- **Requirement 039-007** Candidate selection and the sweeper MUST use one shared runnable predicate, written as the
  candidate query of `crates/infra-postgres/src/workers.rs` has it (the non-FORWARD steps of a stopping run, its
  compensations, are runnable; a paused run's steps are not), and held equal to that query by a differential test that
  runs both on a generated corpus (Gap 715); the guarded poll MUST rotate its sample of ineligible runs (Gap 724) and
  MUST NOT count runs the claim refuses toward its eligible window (Gap 725).
- **Requirement 039-008** One ACTIVE ChildRoot membership per chain MUST be enforced by the unique index of ADR-0078
  section 5 and ADR-0075 condition (k), and one ACTIVE scoped-path membership per root iteration by a new partial unique
  index on (tenant, namespace, `original_run_id`, `root_owner_node`, `iteration_index`) for ACTIVE `SCOPED_PATH` rows;
  the existing `capacity_group_members_scoped_path_unique` includes `group_id` and covers every state, so it cannot. The
  header gains `namespace_id` and `root_run_id` (a foreign key to `runs`, indexed by tenant, namespace and root run for
  the purge and the receipt trigger), `root_owner_node text NOT NULL` and `iteration_index integer NOT NULL` (the root
  collection or root loop the group is created for, and the iteration for a collection, 0 for a loop; insertable, never
  updated, a trigger refusing a change), and `shard smallint NOT NULL`. An insert trigger MUST force a new header to `HELD`
  with `revision`, `epoch` and `active_members` 1 and set its `shard` to `run_shard(root_run_id)` (the SQL function of
  `20261017000000_scheduler_shards`, the same computation as `Shard::of` in `crates/domain/src/sharding.rs`), never
  taken from the caller, set its `namespace_id` from the root run's row (never the caller's value), and refuse a new
  header with `owns_unit` true unless the slot it names is held: for a collection, the `collection_iterations` row of
  (`root_run_id`, `root_owner_node`, `iteration_index`) with `slot_held` true; for a loop, the `loop_scopes` row of
  (`root_run_id`, `root_owner_node`) in lifecycle `RUNNING` or `DELAYING` (`loop_scopes` has no `slot_held`, and a
  loop's iterations are `CHECKPOINTED` and not slot-held during a delay), both read in the same transaction. A partial
  unique index on (`tenant_id`, `root_run_id`, `root_owner_node`, `iteration_index`) `WHERE owns_unit AND possession <>
  'RETIRED'` keeps two unit-owning groups from standing on one slot. The member insert trigger MUST force a new member
  to `ACTIVE` with a null `finished_at`, MUST refuse a member of a `RETIRED` group, and MUST refuse a member whose
  `namespace_id` differs from its header's. Both insert triggers also call the fence in mode `continue` (Requirement
  039-011) and raise `nested_collections_fence` when it answers no, so no writer can insert a group or a member without
  the fence. The group insert trigger, which sets `reserved_bound` from the root run's version (Technical Plan 2), raises
  when that bound is null instead of reserving nothing; a nesting version always records its bound at publication
  (Requirement 039-014), so only a defect reaches it.
- **Requirement 039-009** Transfer receipts MUST be bounded: per group, the latest 16 and every receipt younger than 15
  minutes are kept (the horizon in which a command's commit can still be uncertain to its caller, Decision 713); older
  ones are purged in batches of at most 1,000 by a bounded pass. A retired group's header, members and remaining
  receipts are deleted only with its root run, by the owner-role function `capacity_purge_retired` that feature 020's
  run purge calls in the transaction that purges that run (Gap 039-008); the runtime role holds no `DELETE` on the
  header or member tables. Membership creation, finish and retirement are audited one record each; transfers are not
  audited one by one: they are counted by metrics and by the header's `transfer_count`, which the `capacity.group.retired`
  record carries (so `capacity_group_transfer_receipts.audit_id` becomes nullable and a transfer receipt names no audit
  row).
- **Requirement 039-010** The worker's complete lock sequence MUST be written in
  `specs/039-nested-collections/contracts/worker-lock-sequence.json`, in the order the code takes the locks (Technical
  Plan 6), and checked against ordered markers in the code by `scripts/check-capacity-activation.py`, which MUST also
  refuse any `capacity_groups` row lock without `NOWAIT` or `SKIP LOCKED` (ADR-0075 condition (a)).
- **Requirement 039-011** Activation MUST follow the gate of ADR-0075 and the rules of design 053 (Requirements 053-006
  and 053-007) and ADR-0131: every condition (a) to (l) holds, then the operator records the requirement
  `nested-collections-v1`, which the registry refuses while a live engine or control plane does not list it. An
  execution engine lists it only when its worker guard (both lease producers), its creator and its `capacity_sweep`
  pass all run with `SAGAT_NESTED_COLLECTIONS=on`; a control plane lists it when its definition service runs with the
  switch on. Every write of new state MUST first call the owner-owned definer function
  `nested_collections_required(mode)`, which takes the capability's requirement row `FOR KEY SHARE` (form (a) of
  Requirement 053-006, with a mode) and raises on any mode other than `new_use` and `continue`: in mode `new_use` (a
  publication, an activation, and the creation of any run of a nesting version that is neither a member child nor a
  chain successor, which covers an API start, a schedule action, a batch start, a top-level child start, a reset and a
  rerun; it is called once, by the store's run creation, after authorization and the tenant and namespace resolution,
  so a caller without permission gets today's `404` or `403` and learns nothing about the version) it answers yes only
  when the row is `active`; in mode `continue` (an insert of a group or a member for a run already started) it
  answers yes when the row is `active` or `draining`, and it is called by the insert triggers of both tables
  (Requirement 039-008), never left to the writer. When it answers no to an insert, the trigger raises
  `nested_collections_fence`, the creator rolls back with `NESTED_COLLECTIONS_INACTIVE` and the admission waits like a
  busy join (Technical Plan 2), and the alert of Technical Plan 10 fires; a run start refused by `new_use` is answered
  `NESTED_COLLECTIONS_INACTIVE` on its own surface (Technical Plan 8). Member leases follow the same rule: the guard grants them while the requirement is
  `active` or `draining`. Removal takes the two phases of Requirement 053-007 (Technical Plan 9); rolling a binary back
  below the recorded requirement is unsupported.
- **Requirement 039-012** Crash, duplicate, stale fence, lost broker hint, cancellation, unknown or failed compensation
  and unknown COMMIT MUST be proven for nested scopes, in both race orders, with held locks or barriers, and with the
  possession observer of Technical Plan 11 checked at the end of every race test.
- **Requirement 039-013** Nested scopes MUST be offered in all six SDKs through the collection builders that exist, with
  conformance scenarios that build, publish and run them.
- **Requirement 039-014** Publication MUST refuse a nesting version whose fan-out bound exceeds the tenant's effective
  policy value `nested_group_fan_out`, with `NESTED_FAN_OUT_LIMIT`. The bound of a version is computed from its parsed
  document by `fan_out_of`, the companion of `nesting_of`, recursively: the product of the local `maxConcurrency` of
  every collection on a path from a root iteration down to its deepest level, a loop counting 1; the branches of a
  `parallel` node adding up (they run at once); a pinned child counting its own version's recorded bound, so a pinned
  child without collections still passes on the bound of the pinned children it calls; an active-version child
  counting the share left to it (below). A code version (design 044) has no collection document to read: its bound is
  1 when its envelope's `children` list is empty, and when that list is not empty its bound saturates at 4,097 and its
  `fan_out_open` is true, because a code run may start children by command without a declared count; so publication
  refuses a nesting version that pins such a code version as a child, and the admission of an active-version child
  refuses a code version whose envelope lists children (`NESTED_FAN_OUT_LIMIT`, permanent). Every version, nesting or
  not, records its bound at insert
  (`workflow_versions.fan_out_bound bigint`, nullable; a version that calls nothing and has no collection records 1;
  the computation saturates at 4,097, one more than the policy's maximum, so it never overflows). A version stored
  before the migration has a null bound, computed from its parsed document when it is read (no backfill). The share
  left to a child is the group's `reserved_bound` (below), which is fixed when the group is inserted and never changes,
  divided (rounding down) by the product of `maxConcurrency` on the path from the root iteration to the child call, the
  parallel branches of that path counted by their number; it is never computed from the policy in force when the child
  is admitted, so a policy lowered before a group is created and raised afterwards changes no share of that group. A
  run's place in a group is a column of its own, `runs.capacity_role text NULL` (`ROOT` for a root run of a nesting
  version, `MEMBER_CHILD` for a member child, pinned or active-version; null for every other run), bound at the run's
  `INSERT` by the store's run creation and never changed afterwards: `sagat_app` holds no `UPDATE` on it, and a trigger
  refuses any update of it unless `current_user` is the table's owner, which is the case only inside the
  `chain_segments` copy function of Requirement 039-015, a `SECURITY DEFINER` function owned by that role. Every member child, pinned or active-version, receives its share at its insert
  (`runs.fan_out_share integer NULL`, null on a run that is not a member child), and a grandchild's admission is
  checked against its parent run's share divided the same way, so the check crosses every run boundary; an
  active-version child whose current version's bound passes its share is refused with the same code, and so is any
  child whose parent run is flagged and whose parent's `capacity_role` is not `ROOT` while its `fan_out_share` is null
  (fail closed: a flagged parent with a null role, which only a defective writer could leave, is refused too; a child
  of a `ROOT` parent takes its share from the group). `fan_out_share` is written once: it is bound at the run's
  `INSERT`, `sagat_app` holds no `UPDATE` on it, and a trigger lets it go from null to a value only when `current_user`
  is the table's owner (the segment copy) and refuses any other write. Three checks on `runs` tie the flag, the role
  and the share together: `NOT capacity_member OR capacity_role IS NOT NULL`, `(capacity_role = 'MEMBER_CHILD') =
  (fan_out_share IS NOT NULL)` and `capacity_role IS DISTINCT FROM 'MEMBER_CHILD' OR capacity_member`; each writer
  meets them (a member child binds all three at insert, the segment copy sets all three in one `UPDATE`, a `ROOT` run's
  role is bound at insert and its flag set later in the transaction that creates its first group). The policy is a
  tenant setting only (a namespace value is refused): a column of `tenant_collection_policies`, `nested_group_fan_out
  integer NULL` (null and a missing row both mean 256), written through a new `QuotaResource::NestedGroupFanOut` on the
  audited `put_setting` path (`crates/infra-postgres/src/quotas.rs`), whose default limit and default ceiling are 256
  and whose maximum is 4,096: a tenant administrator may lower it, only an operator's ceiling raises what the tenant may
  set above 256 (the ceiling model of `crates/domain/src/quota.rs`). `resolve` in that module today returns the ceiling
  when nothing is set, except for `IterationConcurrency`; its default-limit case is extended to `NestedGroupFanOut`, so
  an operator ceiling of 4,096 never becomes the value of a tenant that set nothing, and a test compares the SQL rule
  the group trigger and the admission use with `resolve` over every combination of tenant value, namespace value
  (refused) and ceiling; it has its API entry, its contract row and the
  audit action of every quota setting, and `put_setting`'s `None` branch for each of `IterationConcurrency` and
  `NestedGroupFanOut` resets only its own column and deletes the row only when both are null. Every root admission of
  a nesting version checks the version's bound against the effective policy at that moment, so lowering the policy
  contains a tenant at its next admissions, not only at publication. **The tenant-wide ceiling.** Owner decision
  2026-11-04-6 accepts 20,000 pending waits per scheduler replica, alerting above 16,000; L groups of bound F could pass
  it alone (1,024 × 256 = 262,144). So each group reserves its bound when it is created: the group header records
  `reserved_bound` (the version's bound, or the effective policy value when the version calls an active-version child),
  and `tenant_iteration_usage` gains `nested_reserved bigint NOT NULL DEFAULT 0`, increased by the root admission that
  creates the group and decreased by the `Finish` that retires it, both in the transactions that already lock that row
  to charge and release the unit (Requirement 039-016), so the check adds no lock. A root admission of a nesting version
  whose `nested_reserved + reserved_bound` would pass the tenant's ceiling `nested_live_iterations` waits as an admission
  at its tenant limit does (not admitted, retried, no unit taken). `nested_live_iterations` is an operator ceiling only
  (`QuotaResource::NestedLiveIterations`, 16,000 by default, at most 20,000; a tenant administrator cannot raise it).
  So one tenant's nested work holds at most 16,000 live iterations by default, under the replica's 20,000; the
  replica-wide sum over tenants stays governed by Owner decision 2026-11-04-6's metrics, its alert and the shard
  spread. Compensation of finished members runs outside the unit (Technical Plan 4) and outside this reservation.
- **Requirement 039-015** The run row's `capacity_member` flag MUST be enforced by the database: it is `NOT NULL`; a run
  that becomes a successor of a chain takes its predecessor's flag through an `AFTER INSERT` trigger on `chain_segments`
  (a segment row names its `predecessor_run_id` and is inserted after its run row, in the same transaction and under
  the chain lock the successor path already takes; `runs` has no chain column, so a trigger on the run insert could
  not know the chain), which sets the new run's flag when its predecessor's is set and also copies `fan_out_share` and
  `capacity_role` (the trigger calls a `SECURITY DEFINER` function owned by the table's owner, with `search_path`
  pinned to `pg_catalog, pg_temp` and every relation schema-qualified, as Technical Plan 7's purge is written); a member insert is refused unless its run, or its chain's head, is flagged; the flag is never cleared
  (a trigger refuses the update). **A run of a group keeps its version in release 0.3.0.** A run whose
  `capacity_member` flag is set MUST NOT change the version it runs, since its bound was checked against that version
  only: a move of design 044 (Requirement 044-023, whose only writer of the current version is `sagat_move_code_run`
  on `code_run_pins.code_version_id`) of such a run is refused by a `BEFORE UPDATE` trigger on `code_run_pins` that
  raises `nested_member_version_fence` when `code_version_id` changes for a flagged run, and the move item records the
  reason `NESTED_RUN_NOT_MOVABLE`; a continue-as-new to another version (design 044's `version: active`, Requirement
  044-019) of a flagged run is refused at the command's completion with `COMMAND_INVALID` and the reason
  `NESTED_RUN_NOT_MOVABLE`, and, as the database's backstop, the `chain_segments` copy trigger raises
  `nested_member_version_fence` for a successor of a flagged predecessor whose `workflow_version_id` differs from its
  predecessor's. `runs.workflow_version_id` itself never changes (design 044 calls it the run's identity origin), and a
  graph continue-as-new keeps its version (`crates/infra-postgres/src/chain_progress.rs`), so these two paths are the
  only ways a member run could reach another version. Letting a member run move, checked against its share, is Gap
  039-010.
- **Requirement 039-016** The tenant unit MUST be released exactly once. A group created by a root admission records
  `owns_unit = true`; for a grouped iteration or loop none of today's decrement sites decrements (the collection
  checkpoint, the collection drain, the loop checkpoint at the loop's end or at a delay, the loop drain), and a grouped
  loop's delay neither releases nor charges again; the `Finish` that retires the group decrements once, under the guard
  `active_iterations > 0`, in its transaction. A descendant iteration (one admitted inside a member, which charges no
  unit) is recorded `ADMITTED` with `slot_held` false: the migration of Task 039-2 relaxes
  `collection_iterations_slot_consistent` and the matching check of `loop_iterations` to allow `ADMITTED AND NOT
  slot_held` (today both force `slot_held` for every admitted iteration), and the creator marks a descendant so; a
  `BEFORE INSERT OR UPDATE` trigger on both tables refuses a row `ADMITTED AND NOT slot_held` unless its run's
  `capacity_member` flag is set and either the scope's owner path is not the root (an inner collection or loop) or the
  run is a member child (`capacity_role = 'MEMBER_CHILD'`), so the database still refuses an uncharged root iteration of
  every run, a flagged root run included. The
  invariant MUST hold after every checkpoint, drain and cancellation, written as this check per tenant:
  `tenant_iteration_usage.active_iterations = (count of collection_iterations and loop_iterations with slot_held, whose
  run has no capacity group covering them) + (count of capacity_groups not RETIRED with owns_unit)`, where an iteration
  is covered when a scoped-path member of a non-retired group of its run has its root owner node and iteration index
  (or, for a loop, its root owner node); Task 039-20 gives the query.

### Non-functional

- **Non-functional requirement 039-001** The nested 100,000-instance workload (Owner decision 2026-11-04-7) runs on a
  separate database host, with a release build, as the runtime role, twice, and the report states the environment, the
  failures and the latency of a transfer (target: a waiting member acquires within two sweeper ticks of the holder's
  quiescence at the 99th percentile).
- **Non-functional requirement 039-002** With the requirement not recorded, a run MUST cost what it costs today: no
  added statement in its admission, claim or completion. With it recorded, a run that is no member pays at most the
  budget lines of Task 039-13: no added statement in its admission or completion, because the membership resolution of
  a collection or loop admission is skipped on the run row's `capacity_member` flag, which `lock_run` already reads;
  and none in its claim, because the guard's reads are skipped on the same flag, read with the run row the claim
  already locks; the guarded poll adds at most one candidate statement per poll. Both held by the budget suite of
  design 038.
- **Non-functional requirement 039-003** The sweeper's tick MUST be bounded: at most 256 candidates and 64 capped
  commands (`Vacate`, `Acquire`, `Finish`, `FinishPath` and their scoped twins) per tick per replica (the setting
  `SAGAT_CAPACITY_SWEEP_COMMANDS`, 64 by default, at most 512), and at most 64 yield inserts and deletes per tick on a
  budget of their own (`SAGAT_CAPACITY_SWEEP_YIELDS`, 64 by default, at most 512). At the 500-millisecond tick that is 128 capped
  commands per second. A contended group uses two capped commands per transfer and at most one transfer per quantum, so
  at the 1-second quantum a replica serves at most 64 contended groups' transfers per second when it serves no finish;
  each finish served in the same second removes half a group from that figure. The report of Task 039-15 states the
  number of contended groups per replica.

### Success criteria

- **Success criterion 039-001** At a tenant limit of 1, a root iteration that waits on an inner collection of 10
  iterations, each of which starts a child workflow (with and without `chainPolicy`), completes, and so do a child
  workflow that contains a collection and a root `while` loop of three iterations each containing a collection, with a
  delay between iterations, with `active_iterations` asserted after every checkpoint and before and after every
  `fire_delay`; the tenant never has two members of different groups running at
  once, because the unit is released only when its group retires (Requirement 039-016); lowering the limit below the
  current usage delays new root admissions and never blocks admitted nested work (the policy-reduction test of ADR-0030
  §14).
- **Success criterion 039-002** The race test of ADR-0075 condition (e) passes as the runtime role: holder and
  non-holder pollers race the sweeper's `Vacate` and `Acquire`, an unknown COMMIT is injected on the lease path, the
  possession observer of Technical Plan 11 never raises, and `Acquire` eventually succeeds.
- **Success criterion 039-003** With three members with runnable work, including a holder whose inner `forEach` runs
  with a concurrency above 1, each is granted a lease within 2Q + 6T plus the two longest steps that were running when
  the yields were requested, measured from the instant the member became runnable (Q the quantum, T the tick;
  Technical Plan 3).
- **Success criterion 039-004** The 100,000-instance nested workload of Non-functional requirement 039-001 completes twice
  with every instance run once and the consistency checks green.

### User stories

- **User story 039-01** As a workflow author, I write a `forEach` over customers whose body runs a `forEach` over their
  orders and calls a child workflow per order, and it runs with my tenant's limit; the child workflows of one
  customer's iteration lease work one at a time, because they share that iteration's group.
- **User story 039-02** As an operator, I enable nesting once every process runs a build that supports it, and I can see
  how long members wait for possession.

## Goals

- Nesting runs in production behind a recorded activation, with every condition of ADR-0075's activation gate held.
- No deadlock at any tenant limit, no member starved, no lock cycle through the group header.
- The tenant limit bounds groups, and the fan-out bound bounds what each group runs.
- A run without nesting pays nothing.

## Non-Goals

- Anything feature 014 already ships.
- Changing the transactional boundary or the lock order beyond what ADR-0078, ADR-0154 and ADR-0156 decide.
- Gating scheduler transitions, reports or heartbeats on possession (rejected by the owner, Owner question 706).
- A new tenant counter, a pause of admitted work or a drain lane (Decision 704, Owner question 701).
- Nesting deeper than 4 collection or loop levels in 0.3.0.
- Triggers on a workflow that has a `forEach` (Gap 042-003, taken over as Gap 039-006).
- A detached child inside a nested body (Gap 039-009).

## Open Questions

None for the owner. Technical gaps (`specs/039-nested-collections/research.md`):

- **Gap 039-001** A worker whose lease was reaped may still run as a zombie, and an activity that blocks waiting on a
  descendant keeps `Vacate` refused (feature 014's Gap 723). The quantum cannot preempt it; the HELD-age signal and its
  alert show it.
- **Gap 039-002** The quantum's default (1 second) is a starting value to measure under Non-functional requirement
  039-001.
- **Gap 039-003** Depth beyond 4 levels.
- **Gap 039-004** `onError` routing at the entry of a nested collection, as for a root one (Gap 042-008).
- **Gap 039-005** Work leased before a member joined a held group has no possession; the creator proves a joining chain
  has no leased or running step, which holds for a new child, and is asserted for every other join.
- **Gap 039-006** A workflow with a `forEach` cannot have triggers (taken over from Gap 042-003); opening it is a
  follow-up with its own test of the trigger start path against nested admission.
- **Gap 039-007** An approval inside a collection or loop body waits for feature 014's scoped compensation of approvals
  (Decision 295 of feature 014); if that is not built when Task 039-10 runs, the refusal stays and this gap stays open.
- **Gap 039-008** The deletion of retired groups depends on feature 020's run purge, which is not built; until it is,
  retired groups, their members and their last receipts are kept with their runs, and `capacity_purge_retired` is
  tested on its own.
- **Gap 039-009** A detached child (`CancelPolicy::Detach`) inside a nested body is refused; letting it leave its group
  needs a unit rule of its own.

## Owner decisions needed

None open. Under Owner decision 2026-11-24-4 the items ADR-0078 left open, the open question of this design's first
draft and the confirmation of the delegated rows are decided as delegate rows (Owner decisions 2026-11-26-140 to
2026-11-26-149, 2026-11-27-390 to 2026-11-27-395, 2026-11-27-436 to 2026-11-27-447, 2026-11-27-449 to
2026-11-27-463 and 2026-11-27-479):

| Item | Decision | Why |
|---|---|---|
| Accept ADR-0078 (Owner decision 2026-11-26-140) | Accepted as written | Its third architecture and PostgreSQL review found no Blocker or Major; the open items are decided by ADR-0154, not by editing it |
| The creator (Owner decision 2026-11-26-141) | The engine creates groups and members inside the transactions that admit the work; no API | Membership is a consequence of admission, never a user act; an API would add a way to put a chain into a group it does not belong to |
| Sweeper coverage (Owner decision 2026-11-26-142, refined by Owner decisions 2026-11-27-394 and 2026-11-27-443) | Finishes first, then the yield request, the clearing of a stale yield, the vacate of a yielded or idle holder, the acquire | Without the finishes a group's unit counts against admission for ever; without the yield a busy holder is never quiescent; without the clearing a yield can strand a lone holder |
| Fairness (Owner decision 2026-11-26-143, refined by Owner decision 2026-11-27-443) | A quantum of 1 second, oldest-ready-first `Acquire` that skips a member acquired and vacated without a lease, tenant-weighted paging | Without it a busy holder starves its siblings (Gap 718) and a noisy tenant delays another's finishes |
| Receipt retention (Owner decision 2026-11-26-144, refined by Owner decision 2026-11-27-438) | Latest 16 per group and 15 minutes; transfers counted on the header and reported by the retirement record (Owner decision 2026-11-27-453) | Receipts are idempotency keys for a retained command, needed while a commit can be uncertain; a permanent audit row per transfer grows at a rate the author controls |
| Scoped paths (Owner decision 2026-11-26-145) | Commands and a receipt table of their own, as Decision 706 decided, quiescence over the member's iteration path | A forEach in a forEach is a scoped path; without them the commonest nesting cannot run |
| The worker's lock sequence (Owner decision 2026-11-26-146) | Written as a checked contract file, in the order the code takes | A list in prose drifts; a checked list fails the build when the code changes the order |
| The guarded poll (Owner decision 2026-11-26-147) | Rotate the ineligible sample by a keyset cursor and count only claimable runs | Closes Gaps 724 and 725, which block activation |
| The first full-stack run (Owner decision 2026-11-26-148) | A dedicated full-stack nested run on the integrated branch before activation; the release-candidate run repeats it | The release event must not be the first time the composition runs |
| The delegated rows of 2026-11-04 on nesting (Owner decision 2026-11-26-149) | OD-2026-11-04-3 to OD-2026-11-04-8 confirmed as they stand for release 0.3.0 | They were taken under the earlier delegation and nothing found since contradicts them |
| ADR-0156 (Owner decision 2026-11-27-436) | Accepted: a loop owns one group, every member child has a chain, the yield is a row of its own, the unit has one owner, the fence of design 053 | The reviews of 2026-11-27 found the first answers unbuildable or unsafe |
| The fan-out bound (Owner decision 2026-11-27-437) | `nested_group_fan_out`, 256 by default, at most 4,096 | Without it a tenant at limit 1 runs thousands of inner iterations |
| Loop groups (Owner decision 2026-11-27-439, refined by Owner decision 2026-11-27-450) | One group per root loop with one member for the whole loop, keeping its unit and possession across iterations and delays | A loop carries its unit between iterations; a group or a member per iteration would release the unit twice, retire the group during a delay, or cost a transfer per iteration |
| Chains for member children (Owner decision 2026-11-27-440) | A one-segment chain for a child without `chainPolicy` | ChildRoot membership is bound to a chain; keying by run would rewrite the built commands |
| Detach (Owner decision 2026-11-27-441) | Refused inside a nested body | A detached child would keep its group's unit for as long as it runs |
| The fence and removal (Owner decision 2026-11-27-442) | Form (a) of design 053, two-phase removal, listing on the engine | The guard lives in the engine; an unlocked read of the requirement races the removal |
| Unit ownership, compensation, flags and purge (Owner decisions 2026-11-27-444 to 2026-11-27-447) | `owns_unit` with one decrement; compensation after retirement outside possession; database-enforced `capacity_member` and derived `nesting`; `capacity_purge_retired` | Each closes a path where the unit is released twice, never, or by a writer that forgot a flag |
| The drain rule (Owner decision 2026-11-27-449) | Phase 1 refuses new nesting runs, publications and activations; groups, members and member leases of runs already started continue while `draining` | Phase 1 must not stop the runs phase 2 waits for |
| Join waits (Owner decision 2026-11-27-451) | A busy join is recorded and, after a quantum, the sweeper asks the holder to yield | A busy holder's claims keep the header locked; without it a child of that holder may never join |
| The lease record and the header columns (Owner decision 2026-11-27-452, refined by Owner decision 2026-11-27-458) | The claim transaction sets the holder's `last_leased_at` after the first lease grant is persisted; the header gains `namespace_id` and `root_run_id`, its shard derived | The skip of an unserved member and the shard, purge and receipt rules need facts nothing recorded |
| Transfers and the built commands (Owner decision 2026-11-27-453) | No audit row per transfer and no hourly table: `transfer_count` in the retirement record; `audit_id` of a transfer receipt nullable | A permanent row per transfer grows at a rate the author controls; an hourly upsert inside every transfer adds a hot row for a count the header already holds |
| The fan-out bound's storage (Owner decision 2026-11-27-454, refined by Owner decision 2026-11-27-479) | Every version records its bound; the child's share is the group's fixed `reserved_bound` divided by the product down to the child call, and a run's place in a group is `capacity_role`, bound at insert; a member run keeps its version in 0.3.0 (an exception to Requirements 044-023 and 044-019); the policy is a column of `tenant_collection_policies` with the audited write of today | The bound could not be computed for a child or changed with an audit trail |
| The lease fence (Owner decision 2026-11-27-455) | A trigger refuses a lease of a flagged run's step unless the guard ran in the transaction | An older binary not yet counted could otherwise lease member work for up to a heartbeat |
| Children and scopes at non-root paths (Owner decision 2026-11-27-457) | A task of its own (Task 039-25) extends the child-start path and the scope entry to any instance path, before the codec opens | Today's child-start path handles only root-path steps |
| The implementation-readiness answers (Owner decision 2026-11-27-458) | Receipts: `FinishPath` in the finish receipts, a path transfer table for `VACATE` and `ACQUIRE`; purge exempted by the table's owner; side tables without a foreign key to the header; a null fan-out bound computed at read; a quota resource of its own for the policy; descendants recorded without a slot; the lease fence reads `runs` and covers every lease of a flagged run | Each makes a task buildable as written without changing a guarantee |

## Critical User Journeys (CUJs)

### CUJ 1: A collection of collections

*   **Given**: a tenant at limit 1 and a workflow whose `forEach` over 3 customers (concurrency 1) has a body that is a
    `forEach` over each customer's orders (5 each, concurrency 5, so a fan-out bound of 5) calling a child workflow per
    order, the child without `chainPolicy`.
*   **When**: it is published (after the operator recorded `nested-collections-v1`) and run, and the engine is killed
    once while the sweeper's `Acquire` is in flight.
*   **Then**: every inner iteration and every child runs once, each child with a chain of one segment; at most one member
    of the one group leases work at any instant; the killed `Acquire` is either committed or re-sent from the state; the
    run completes with the same result as on a run without the kill.

### CUJ 2: A holder that hogs

*   **Given**: a group whose holder has a long sequence of short steps and two child members with runnable work.
*   **When**: the holder runs.
*   **Then**: after a quantum the sweeper inserts a yield request, the guard grants the holder no new lease, its running
    steps end, the sweeper vacates it and the oldest waiting member acquires; each member is served within 2Q + 6T plus
    the two longest steps running when the yields were requested; if a waiting member is paused after the yield, the
    yield is cleared and the holder runs again.

### CUJ 3: Nesting not yet enabled

*   **Given**: a deployment where one engine still runs a build without the guard.
*   **When**: the operator records `nested-collections-v1`.
*   **Then**: the registry refuses while that engine is live; publication of a nested document is refused with
    `NESTED_COLLECTIONS_DISABLED`; root collections run as before.

## Technical Plan (Architecture)

### 1. What exists and is reused

The tables, constraints and triggers of the capacity migrations; `Vacate`, `Acquire`, `Finish` for ChildRoot members and
the scoped leaf finish; the receipts and their commit-uncertainty classifier (Decision 713, `confirmed_abort` in
`crates/infra-postgres/src/capacity_finish.rs`); bounded discovery and holder discovery (`capacity_discovery.rs`); the
worker guard (the port method `worker_capacity_available`, `crates/application/src/execution.rs`, implemented in
`crates/infra-postgres/src/execution.rs` and `crates/infra-postgres/src/execution/path.rs` over the module function
`available` of `crates/infra-postgres/src/execution/worker_capacity.rs`, behind `child_root_capacity_guard`); `scripts/check-capacity-activation.py` and the step-status-writer inventory
(`specs/014-workflow-primitives/contracts/step-status-writers.json`).

### 2. The creator and descendant admission

- **Only versions published with nesting.** `workflow_versions.nesting` is derived from the parsed document by one
  function (`nesting_of(&Definition)`) that every path inserting a version calls, the publication path and the ad hoc
  path of `crates/infra-postgres/src/execution/ad_hoc.rs` alike; `scripts/check-architecture.py` refuses an
  `INSERT INTO workflow_versions` that does not bind it, `fan_out_bound` and `fan_out_open`. The same insert binds
  `fan_out_bound` and `fan_out_open`, computed by `fan_out_of(&Definition)` (Requirement 039-014). A root admission creates a group only for a nesting version; a version
  published without nesting, or a top-level child workflow outside every member, creates no group and charges what it
  charges today. Inside a member, admissions are descendants whatever their version (Requirement 039-002).
- **The fence.** The insert triggers of `capacity_groups` and `capacity_group_members` call
  `nested_collections_required('continue')` for every inserted row and raise `nested_collections_fence` when it answers
  no, so no writer (the root collection admission, the root loop entry, the join in `PgRunTx::commit`) can insert a
  group or a member without it; the application calls it with `'new_use'` for the admission of a new run of a nesting
  version, a publication and an activation (Requirement 039-011); the function raises on any other mode. An answer of
  no to an insert makes the creator roll back with `NESTED_COLLECTIONS_INACTIVE`: the admission
  rolls back and the step or iteration is retried by the next tick with no attempt consumed, counted
  (`capacity_member_join_total{outcome="inactive"}`), as a busy join is. While the requirement is `draining`, runs
  already started keep creating their groups and members, so phase 2's drain check can pass (Technical Plan 9). The
  function is called once per nested root admission, per join and per publication: at the 100,000-instance workload of
  Non-functional requirement 039-001 that is at most a few hundred calls per second. A `FOR KEY SHARE` lock conflicts
  with no other `FOR KEY SHARE` lock and with no lock phase 1 or phase 2 takes except phase 2's own update of the row, so
  the calls never wait on one another; their cost (one row lock, a multixact when several overlap) is measured by Task
  039-15, which is why form (a) rather than form (b) fits this rate.
- **Group and first member of a root collection iteration.** When a root collection admission of a nesting version
  takes the tenant unit (`retained < limit`), the same transaction inserts the group (`HELD`, holder the new scoped-path
  member, `held_since` now, `owns_unit` true) and the member. The group row is new, so no lock on it is waited for. That
  initial `HELD` is the exception to ADR-0078 section 3's rule that possession moves only by the commands, recorded in
  ADR-0154.
- **Group of a root loop.** A root loop of a nesting version takes its unit at entry, as today (`enter_loop`), and the
  same transaction inserts the group (`owns_unit` true) with one scoped-path member for the whole loop, keyed by (run,
  the loop's owner node) with the fixed iteration index 0, as holder. Iterations are sequential, so possession never
  moves between them: every iteration's steps are work of that one member, and no member is created or finished per
  iteration. A grouped loop keeps its unit and its member across a delay: `checkpoint` does not decrement at a delay and
  `fire_delay` does not charge again. The loop's member is final, and `FinishPath` may finish it, only when the loop
  scope's lifecycle is final (`SUCCEEDED`, `FAILED` or `CANCELLED` in `loop_scopes`, never `READY`, `RUNNING`,
  `DELAYING` or `DRAINING`) and every ChildRoot member admitted under it is final; that finish retires the group and
  releases the unit.
- **Descendants.** Every admission inside an ACTIVE member is a descendant (Requirement 039-002): the collection
  admission of `crates/infra-postgres/src/primitives.rs` and the loop admissions of
  `crates/infra-postgres/src/loop_progress.rs` resolve, in their transaction and only when the run row's
  `capacity_member` flag is set, a `MembershipContext` (the member row that covers the admitting path or run, read
  without a lock on the header), and with one they skip the fresh-admission check and the charge of
  `tenant_iteration_usage`; they are bounded by `maxInstances`, the local scope slots and the fan-out bound.
- **Join.** A child workflow started inside a member's work is created by the existing parent-first path (the parent
  run `FOR NO KEY UPDATE`, then the tree-admission advisory lock, then the new chain and run rows, ADR-0030 §19 and
  ADR-0037 §6). Every such child gets an execution chain: a child without `chainPolicy` gets a chain of one segment
  (`max_segments` 1, `original_deadline_at` the child run's deadline, or, when it has none, its creation plus
  `MAX_RUN_DEADLINE`, 366 days, `crates/application/src/runs.rs`), and `link_child_chain`
  (`crates/infra-postgres/src/chain_progress.rs`) accepts a parent step at any instance path for a member child, not
  only the root path. The same transaction then locks the group `FOR UPDATE NOWAIT` (ADR-0078 rule 2), compares and sets
  `revision` and increases `active_members`, and inserts the ChildRoot member `ACTIVE`.
  **Where this happens today and what changes.** The child-start path handles only root-path steps today:
  `pending_children` (`crates/infra-postgres/src/maintenance.rs`) filters `cardinality(instance_path_nodes) = 0`;
  `start_child` (`crates/scheduler/src/passes.rs`) looks the parent node up with `parent_definition.node(node)` and
  reports with `finish_with(.., node, ..)`; `child_started` (`crates/infra-postgres/src/execution.rs`) keys the parent
  step by (run, node); the child run's parent reference carries only `parent_node`; and `PgRunTx::commit` creates a
  chain only when the child's definition has a `continuation_policy`. Task 039-25 extends each: `pending_children`
  returns children at any instance path, `PendingChild` and the child run's parent reference carry the parent
  instance path, `start_child`, `child_started`, the child's finish and its cancel key the parent step by (run, instance
  path, node); in `PgRunTx::commit` a member child without a policy takes a new branch that calls
  `initialize_chain_in_transaction` with one segment and the deadline above; then, in the same commit and after that
  chain exists (the member row references it), the group lock `NOWAIT`, the header's compare and set and the member
  insert; the child run's `capacity_member` is bound to true at the run's own `INSERT`, so the member-insert trigger of
  Requirement 039-015 finds it flagged. Collections and loops owned at a non-root path are stored already
  (`iteration_scopes` and `loop_scopes` carry `owner_path_nodes` and `owner_path_indices` in their unique keys,
  `20261029000000_collection_progress`, `20261031000000_loop_progress`); Task 039-25 also covers the scheduler's
  collection and loop entry for such an owner, proven by a test per shape, because no code path reached it while the
  codec refused nesting. A `Busy` answer rolls the
  creation back and the step that creates the child is retried by the next tick, with no attempt consumed, counted
  (`capacity_member_join_total{outcome="busy"}`); nothing waits on the group. In a second short transaction the creator
  then records the wait, a row of `capacity_group_join_waits` naming the waiting parent step (its run, instance path
  and node), inserted with `ON CONFLICT (tenant_id, group_id) DO UPDATE SET last_busy_at = now()` (and the waiting
  parent, when the row named another) and no header lock; each busy retry refreshes `last_busy_at`; the join that
  succeeds deletes the group's wait row in its own transaction. A wait is live only while its parent step is still
  pending, its run is neither paused nor stopping, and `last_busy_at` is within two ticks; the sweeper ignores a wait
  that is not live and deletes it (selection 4), so a member paused or cancelled after a busy join can never keep a
  yield on the holder. Both side tables are written with `INSERT ... SELECT ... WHERE EXISTS` a non-`RETIRED` header of
  the same tenant, namespace and group, read without a lock, so a side row is never inserted for a group that is gone
  or belongs to another tenant. The two side tables are:

  ```sql
  CREATE TABLE capacity_group_yields (
      tenant_id    uuid        NOT NULL,
      namespace_id uuid        NOT NULL,
      group_id     uuid        NOT NULL,
      epoch        bigint      NOT NULL,
      requested_at timestamptz NOT NULL,
      PRIMARY KEY (tenant_id, group_id));
  CREATE TABLE capacity_group_join_waits (
      tenant_id          uuid        NOT NULL,
      namespace_id       uuid        NOT NULL,
      group_id           uuid        NOT NULL,
      parent_run_id      uuid        NOT NULL,
      parent_path_nodes  text[]      NOT NULL,
      parent_path_indices integer[]  NOT NULL,
      parent_node        text        NOT NULL,
      first_busy_at      timestamptz NOT NULL,
      last_busy_at       timestamptz NOT NULL,
      PRIMARY KEY (tenant_id, group_id));
  ```

  Neither has a foreign key to `capacity_groups`, on purpose: the check of such a key takes `FOR KEY SHARE` on the
  header, which waits behind the `FOR UPDATE` a holder's claim keeps until its lease commits (the wait ADR-0075 names),
  and these inserts exist so as never to wait on the header. One row per group: a yield row of an earlier epoch is
  replaced by selection 3 (`ON CONFLICT (tenant_id, group_id) DO UPDATE ... WHERE epoch < excluded.epoch`) and ignored by
  the guard. Yield rows are deleted by `Vacate`, `Acquire` and selection 4; wait rows by the join that succeeds and, when
  not live, by selection 4; both by `capacity_purge_retired`. The sweeper pages over groups that are not `RETIRED`, so it
  never meets a side row of a retired group: the `Finish` (or `FinishPath`) that retires a group deletes its yield and
  wait rows in its transaction, a row that an insert racing that retirement still adds is deleted by
  `capacity_purge_retired` with its group, and the `WHERE EXISTS` insert keeps any later one from appearing. A
  held-header test inserts each while another connection holds the header `FOR UPDATE` and sees no wait. A holder whose claims keep the header locked therefore cannot starve a joining child: once a wait is a
  quantum old, the sweeper asks the holder to yield (Technical Plan 3, selection 3), the holder takes no new lease, and
  the header frees up for the join.
- **The tenant unit is released when the group retires** (Requirement 039-016). The four decrement sites of today,
  `primitives.rs` at the collection checkpoint and the collection drain and `loop_progress.rs` at the loop checkpoint
  (its end or a delay) and the loop drain, skip the decrement for an iteration or loop whose group owns the unit; the
  `Finish` (or `FinishPath`) that retires a group with `owns_unit` true decrements once, takes `tenant_iteration_usage`
  `FOR UPDATE` after the group (an amendment of ADR-0078 section 3, in ADR-0156), subtracts the group's
  `reserved_bound` from `nested_reserved` in the same update, and refuses with an integrity error when either counter
  would go below 0. A cancelled or failed root iteration whose ChildRoot members still compensate keeps its
  unit until they are final.
- **Flags.** `runs.capacity_member` is `NOT NULL DEFAULT false`, set by the creator for every member run and copied to
  a chain successor, with `fan_out_share` and `capacity_role`, by an `AFTER INSERT` trigger on `chain_segments` from the run its
  `predecessor_run_id` names (the segment row is inserted after the successor's run row, under the chain lock that path
  already takes, `crates/infra-postgres/src/chain_progress.rs`), which also refuses a successor whose version differs
  from a flagged predecessor's (Requirement 039-015); a
  member insert is refused unless its run or its chain head is flagged; an update that clears the flag is refused.
  `runs.capacity_role` (`ROOT` or `MEMBER_CHILD`) is bound at the run's insert by the store's run creation (`ROOT` for
  a root run of a nesting version, `MEMBER_CHILD` for a member child) and never changed (Requirement 039-014). A
  child member's run is flagged at its own `INSERT` (above); the root run of a scoped-path member is flagged by an
  `UPDATE` in the transaction that creates its first group (it already holds the run row there), so every later lease of
  that run, its top-level steps included, goes through the guard's transaction setting (Technical Plan 9), and a
  process whose gate is closed cannot lease them; that is the cost of nesting in a run, stated in the runbook.
- **The header's own columns.** The creator inserts the header with `namespace_id` and `root_run_id` (the run that
  admitted the root iteration or loop); the insert trigger sets `shard` to `run_shard(root_run_id)` (Requirement
  039-008).
- **New columns, with their types and defaults.** On `capacity_groups`: `namespace_id uuid NOT NULL`, `root_run_id uuid
  NOT NULL` (foreign key `(tenant_id, root_run_id)` to `runs`), `shard smallint NOT NULL`, `held_since timestamptz NULL`,
  `owns_unit boolean NOT NULL DEFAULT false`, `transfer_count bigint NOT NULL DEFAULT 0`, `reserved_bound bigint NOT
  NULL` (no default: the insert trigger sets it from the version and raises when the version's bound is null), `root_owner_node text NOT NULL`, `iteration_index
  integer NOT NULL` (both set at insert, never updated). On `capacity_group_members`: `acquired_at timestamptz NULL`, `last_leased_at timestamptz NULL`,
  `last_vacated_unleased_at timestamptz NULL`. On `runs`: `capacity_member boolean NOT NULL DEFAULT false`,
  `fan_out_share integer NULL` and `capacity_role text NULL CHECK (capacity_role IN ('ROOT', 'MEMBER_CHILD'))`. On `tenant_iteration_usage`: `nested_reserved bigint NOT NULL DEFAULT 0`. On
  `workflow_versions`: `nesting boolean NOT NULL DEFAULT false`, `fan_out_bound bigint NULL` and `fan_out_open boolean
  NOT NULL DEFAULT false` (Requirement 039-014),
  so today's two version inserts keep working until Task 039-21 binds both. Every one is additive (design 053's expand
  rule).
- **Grants.** The migration grants `sagat_app` exactly:
  - `capacity_groups`: `INSERT (tenant_id, group_id, root_run_id, root_owner_node, iteration_index, possession, holder_member_id, held_since, owns_unit)`
    (the trigger forces the rest: `namespace_id` from the root run, `shard`, the counters, and `reserved_bound` from
    the root run's version, its `fan_out_bound`, or the effective policy value when the version's `fan_out_open` is
    true, a new `workflow_versions` column, `boolean NOT NULL DEFAULT false`, bound by `fan_out_of` when the version
    calls an active-version child; the same trigger adds `reserved_bound` to the tenant's `nested_reserved`, on the
    `tenant_iteration_usage` row the admission already holds, and raises `nested_live_limit` when the sum would pass
    the ceiling, which the creator answers by waiting as at its tenant limit) and `UPDATE (revision, epoch, active_members, possession,
    holder_member_id, held_since, transfer_count)`, today's update list (`20261210000000_capacity_finish_storage`) plus
    the last two;
  - `capacity_group_members`: `INSERT` of today's binding columns (the trigger forces the state), `UPDATE (state,
    finished_at, acquired_at, last_leased_at, last_vacated_unleased_at)`;
  - `capacity_group_yields` and `capacity_group_join_waits`: `SELECT`, `INSERT`, `UPDATE (epoch, requested_at)` on the
    first, `UPDATE (parent_run_id, parent_path_nodes, parent_path_indices, parent_node, last_busy_at)` on the second
    (the columns its `ON CONFLICT ... DO UPDATE` assigns), `DELETE` on both;
  - `capacity_path_transfer_receipts`: `SELECT`, `INSERT`; the receipt tables: `DELETE` (the retention trigger decides,
    Technical Plan 7);
  - `runs`: `UPDATE (capacity_member)` only; `fan_out_share` and `capacity_role` are bound at the run's `INSERT`, and
    after it only the segment copy's `SECURITY DEFINER` function, owned by the table's owner, writes them (their
    triggers allow the null-to-value write only when `current_user` is that owner); `code_run_pins`: no new grant (its
    `BEFORE UPDATE` trigger of Requirement 039-015 refuses the move of a flagged run);
    `workflow_versions`: the `INSERT` it has, which now binds `nesting`, `fan_out_bound` and `fan_out_open`;
    `tenant_iteration_usage`: no new grant, since `sagat_app` already holds `UPDATE` on the whole table
    (`20261029000000_collection_progress`), which covers `nested_reserved` (the group's insert trigger, which runs with the writer's privileges, and the retiring `Finish` write it on the row
    the transaction already holds; a check constraint keeps it at or above 0), the same trust the runtime role has
    over `active_iterations` today;
  - no `DELETE` on the header or member tables;
  and adds the unique indexes of Requirement 039-008 (condition (k)) and the partial indexes the sweeper's selections
  read: `capacity_groups (tenant_id, shard, group_id) WHERE possession <> 'RETIRED'` (the page),
  `capacity_group_members (tenant_id, group_id) WHERE state = 'ACTIVE'` (finishes and runnable members),
  `capacity_group_yields (tenant_id, group_id)` (its key) and `capacity_group_join_waits (tenant_id, first_busy_at)`
  (selection 3's age).
- **Audit and telemetry.** `capacity.member.created`, `capacity.member.finished` and `capacity.group.retired` (which
  carries the group's `transfer_count`), values of the closed audit action vocabulary with their test and their rows in
  design 040's mapping table once it is built; and `capacity_members_created_total{kind}`.

### 3. The sweeper

A pass `capacity_sweep` on the scheduler tick, with no transaction open while it calls a command (ADR-0078 section 4).
A group is served by one replica: the one holding the shard of the group's root run, recorded on the header at
creation as `shard` by the insert trigger from the root run (a run's shard never changes); every command the sweeper
issues for the group reads that shard's fence `FOR SHARE` (the leaf lock of ADR-0050), and Task 039-7 makes the built
ChildRoot commands read the group's shard rather than the shard of the member run they name, with a test. Each tick it
pages, per held shard, with a keyset cursor, at most 256 candidates, taking candidates per tenant in round robin
weighted by the scheduler's tenant weights (the store method `tenant_weights`, `crates/application/src/shards.rs`, read
by `crates/scheduler/src/shards.rs`) through the scheduler's existing weighted helper `pick`
(`crates/scheduler/src/passes.rs`), and serves the selections in this order:

1. `HELD` groups whose holder's head or iteration is final (for a root loop's member, whose loop scope is final) →
   `Finish` (or `FinishPath`) the holder;
2. ACTIVE non-holder members whose head or iteration is final, in `HELD` or `VACANT` groups → `Finish` that member, the
   last one retiring the group;
3. `HELD` groups with no yield request of the current epoch whose holder has held possession for at least the quantum
   while another member has runnable work, or that have a live join wait (Technical Plan 2) at least a quantum old →
   insert a row of
   `capacity_group_yields` (Technical Plan 2), `epoch` the header's, with `ON CONFLICT (tenant_id, group_id) DO UPDATE`
   only when the stored epoch is older; no header lock, no receipt, possession does not move;
4. `HELD` groups with a join wait that is not live (its parent step no longer pending, its run paused or stopping, or
   `last_busy_at` older than two ticks) → delete the wait row; `HELD` groups with a yield request none of whose other
   members has runnable work any more and that have no live join wait, or whose yield row belongs to an earlier epoch →
   delete the yield row (the holder leases again);
5. `HELD` groups whose holder is quiescent (no leased or running step of its work) and either has a yield request of
   the current epoch or has no runnable work, while another member has runnable work → `Vacate` (which deletes the yield
   row and, when the holder's `last_leased_at` is older than its acquisition, sets its `last_vacated_unleased_at`);
6. `VACANT` groups with an ACTIVE member that has runnable work (the shared predicate) → `Acquire` for the member whose
   oldest runnable step became ready first, skipping a member whose `last_vacated_unleased_at` is within the last
   quantum; `Acquire` sets `held_since` and the member's acquisition instant.

A holder alone in its group is never vacated while it has runnable work, so a single-member group does not churn
between its steps; a yield request whose waiting member stopped being runnable (paused, timed out, finished) is
cleared by selection 4, so a lone holder is never stranded. At most `SAGAT_CAPACITY_SWEEP_COMMANDS` capped commands per
tick (selections 1, 2, 5 and 6, 64 by default) and `SAGAT_CAPACITY_SWEEP_YIELDS` yield inserts and deletes on their own
budget (selections 3 and 4, 64 by default); candidates beyond the page and commands beyond the caps are
deferred to the next tick and counted (`capacity_sweep_deferred_total{reason}`, `reason` in `candidates`, `commands`,
`yields`). The operation key is derived from the whole tuple each command fingerprints (ADR-0078 section 4); a
`FingerprintConflict` means "rediscover".

**Timing.** One transfer costs at most Q + S + 3T: up to one tick to see the quantum, the longest step S running when
the yield was requested, one tick to see quiescence and one tick to `Acquire`. With three members, the last one served
waits at most 2Q + S_H + S_A + 6T from the instant it became runnable; at Q = 1 s and T = 0.5 s that is 5 seconds plus
the two longest steps.

### 4. Scoped-path commands and the scoped guard

`VacatePath`, `AcquirePath` and `FinishPath` serve scoped-path members. `FinishPath` writes its receipt to the existing
`capacity_group_finish_receipts` with kind `SCOPED_PATH` (that table already has the discriminator, the scoped binding
and its checks, `20261212000000_scoped_finish_binding`, and `scoped_leaf_finish.rs` writes there); `VacatePath` and
`AcquirePath` write a new table, `capacity_path_transfer_receipts`, which holds only `VACATE` and `ACQUIRE` and mirrors
`capacity_group_transfer_receipts` (`20261226000000_capacity_transfer_receipts`) column for column, with the chain
binding replaced by the scoped binding of the finish receipts (`original_run_id`, `root_owner_node`,
`iteration_index`, under the same checks) and `audit_id` nullable from the start (Technical Plan 7); its operation key,
fingerprint and retention trigger are those of the ChildRoot table. They use the same compare and set of revision and
epoch, and the lock order of every
capacity command (ADR-0030 §18, ADR-0078 rule 1): the member's chain when it has one, then the run that owns the path,
then the group `NOWAIT`, then the shard `FOR SHARE`, then, for a retiring finish of a group that owns the unit,
`tenant_iteration_usage` `FOR UPDATE`, written in the checked contract of Technical Plan 6. Quiescence is over the
member's iteration path: no leased or running step whose path starts with the member's root node and iteration index.

`FinishPath` finishes a scoped-path member whose iteration is final, whatever its outcome, and whatever its body
contains: its proof is read from the source facts in its transaction (for a collection's member, the iteration's
lifecycle is `CHECKPOINTED` or `DRAINED` in `collection_iterations`; for a root loop's member, the loop scope's
lifecycle in `loop_scopes` is `SUCCEEDED`, `FAILED` or `CANCELLED`, so a loop that is `RUNNING`, `DELAYING` or
`DRAINING` is never finished; no step under the path prefix is non-final; and every ChildRoot member admitted under that
iteration or loop is final). It needs no certified success, so it is not the scoped leaf
finish of feature 014 (`crates/application/src/leaf_certification.rs`, `crates/infra-postgres/src/scoped_leaf_finish.rs`),
which stays for a depth-1 ordinary leaf; a test per terminal outcome (succeeded, failed, cancelled, compensated, timed
out, partial, compensation failed) and per body shape (leaf, nested collection, loop, child).

**The scoped guard.** Today the guard refuses every claim on a run that has a `SCOPED_PATH` member (`UnsupportedScope`
in `worker_capacity.rs`), and ADR-0075 requires scoped paths to fail closed. With the requirement recorded, both lease
producers (`crates/application/src/worker.rs` and `crates/application/src/worker/scoped_claims.rs`) pass the instance
path to the guard, which grants a lease on a step whose path lies within a scoped-path member's iteration (or, for a
root loop, under the loop's owner node) only when that member holds its group and no yield row of the header's epoch
exists for the group, keyed by (run, root node, iteration index) for a collection and (run, root node) for a loop; when
it approves, the claim goes on through the checks that can still skip the step without a lease (`activity_admitted`,
`compensation_admitted` and `script_execution_admitted` in `scoped_claims.rs`, `database_claim` in `worker.rs`); only
once the first lease grant of a step of that member is persisted does the claim transaction set the holder's
`last_leased_at`, still under the `SKIP LOCKED` header lock, and only once per transaction, so a member held back by a
quota, a breaker or a database refusal is recorded as never leased; a step of the root run outside every member is not
gated, so a nested admission never halts the root run's top-level steps.
The shared predicate of Technical Plan 5 gains the same scoped clause. Without the requirement the refusal stays.
ADR-0154 amends ADR-0075's scoped-path bullet to this rule; race tests cover a holder and a non-holder iteration
claiming at once, and a claim racing `VacatePath`.

**Compensation after retirement.** A scope's compensation is planned only once every iteration is checkpointed or
drained (`plan_scope_compensation`, `crates/domain/src/execution/scoped_sagas.rs`, called from
`crates/scheduler/src/passes.rs`), when the groups of the successful iterations may
already have retired and released their units. A compensation step under the path of a member that is finished, or of
a retired group, is leased outside possession and outside the unit, as a root collection's scope compensation is leased
today after its iterations released their slots. The tenant limit bounds forward work; compensation is bounded by the
steps it compensates. A limit-1 test compensates a successful iteration with a nested body after a later iteration
fails.

### 5. The runnable predicate and the guarded poll

Two texts, kept apart on purpose so that the differential test compares two things. The candidate query keeps its own
text, the `claimable_query!` macro of `crates/infra-postgres/src/workers.rs`, and its capacity clause (today the
ChildRoot-only `capacity_window!`, judged per run) gains a per-path scoped clause judged per instance path, as the
guard is. The sweeper's predicate is a separate macro, `runnable_member_work!`, in `crates/infra-postgres/src/
capacity_sweep.rs`, used only by the sweeper, written from the candidate query as it stands: a stopping run's
non-FORWARD steps (its compensations) are runnable, a paused run's steps are not. The differential test runs both, the
candidate query with a session that serves every activity, on a generated corpus that includes paused, stopping and
compensating runs and scoped members, and asserts the same answer per member. The sweeper has no worker session, so the
predicate cannot know whether a worker serves an activity, a quota or a breaker allows it; the skip of Technical Plan 3
selection 6 keeps such a member from taking possession back every tick. The guarded poll samples ineligible runs by a
rotating keyset cursor instead of always the four oldest (Gap 724): the cursor lives in the memory of the process
that polls, one per (tenant, namespace), on `(ready_at, run_id)`, starts again from the lowest key when a page returns
fewer rows than asked, and is lost harmlessly on a restart (the next poll starts from the lowest key) and counts toward its eligible window only runs whose
claim the guard would accept (Gap 725). A run without membership takes the path of today unchanged (Non-functional
requirement 039-002). Between the guard's unlocked header read and its `SKIP LOCKED` read the holder may change (Gap
726): the second read rechecks the holder, the revision and the yield row, and refuses on any difference; the cost of
the statement's generic plan is measured by Task 039-13.

### 6. The worker's lock sequence, checked

`specs/039-nested-collections/contracts/worker-lock-sequence.json` lists the locks of a guarded claim in the order the
code takes them (`crates/application/src/worker/scoped_claims.rs` and `crates/infra-postgres/src/execution.rs`,
`lock_chain_for_run` and `lock_path_revision`): the chain, the run, the drain advisory lock, the binding advisory lock,
the worker session, the secret advisory locks, the group `SKIP LOCKED` after an unlocked header read, once per instance,
the activity quota locks, the breaker, the database and script locks, and last the holder's member row, written
(`last_leased_at`) once after the first lease grant of the member is persisted, under the group lock still held; each
with the function that takes it. Each lock is marked in the code by a comment `// lock-seq: <sequence id>/<step>` on the
line that takes it (for example `// lock-seq: guarded-claim/group`), and `scripts/check-capacity-activation.py` reads
the file and checks, per sequence, that the markers of the named function appear in the listed order and that no
`capacity_groups ... FOR` lacks `NOWAIT` or `SKIP LOCKED`. A sequence whose named function does not exist yet is
reported as pending, not passed, and Task 039-17's activation record requires none pending. Task 039-3 writes the
file with the guarded-claim sequence only (its code exists) and the check; each lane-A task that writes a creator,
join or command adds its sequence to the file and its markers to the code in the same change (Tasks 039-5, 039-7,
039-25). The file also lists the lock
sequences of the creator (Technical Plan 2: parent run, tree-admission lock, new chain and run, group `NOWAIT`), of the
join, and of every capacity command (Technical Plan 4, with the retiring finish's `tenant_iteration_usage` after the
group). The check holds an allow-list of the callers of the commands (the `capacity_sweep` pass only; ADR-0078
"Enforced by") and of the one place that enables the guard (`.with_child_root_capacity_guard(` in the engine's
composition, `apps/execution-engine/src/lib.rs`, where Task 039-17 adds the call; no production file calls it today), replacing today's ban of every production call (`ACTIVATION_CALL`), and the check that ADR-0075 condition (g) names. A
real-PostgreSQL test per writer of a group or member row runs it with the group held on another connection and expects
`Busy` or a skip, never a wait.

### 7. Receipts and retention

A bounded pass deletes, per group, receipts older than 15 minutes that are not among the latest 16, in batches of
1,000. The receipt tables carry `ENABLE ALWAYS` append-only triggers that refuse `DELETE`
(`20261226000000_capacity_transfer_receipts`, `20261210000000_capacity_finish_storage`); the migration replaces them by
triggers that allow a `DELETE` only of a receipt that the retention rule releases (older than 15 minutes and not among
its group's latest 16, checked by the trigger itself, `SECURITY INVOKER`, `search_path` pinned to `pg_catalog, pg_temp`,
every relation in its body written with its schema, `public.`, so no temporary table of the caller can stand in for one)
or made by the owner of the table, which the trigger tests as `current_user` equal to the table's owner
(`pg_get_userbyid(relowner)` of its `pg_class` row); a delete by `capacity_purge_retired` is such a delete, since a
`SECURITY DEFINER` function runs as its owner, the schema-owning role, and `sagat_app` can never be that role. The
migration grants `sagat_app` `DELETE` on the receipt tables only. The header and member guards
(`capacity_groups_guard`, `capacity_group_members_guard`) are replaced so that a `DELETE` is allowed only of a
`RETIRED` group and its members, and only when made by the table's owner in the same way. `capacity_purge_retired(
tenant, namespace, root_run)` (owner-owned, `SECURITY DEFINER`, `search_path` pinned to `pg_catalog, pg_temp` with
every relation written with its schema, `REVOKE ALL ... FROM PUBLIC` and executable by its owner only: it is called
from inside feature 020's purge function, itself an owner-owned definer function, so its caller is the owner and no
runtime role can call it directly) first requires the purge authorization feature 020 records for this transaction and
run (a row `(pg_current_xact_id(), tenant, root_run)` of `retention_purge_authorizations`,
`specs/020-visibility-analytics-archival/data-model.md`) and raises without it, then finds the `RETIRED` groups whose
`root_run_id` is that run and deletes, in this order, their transfer receipts and path transfer receipts, their finish receipts, their yield and
join-wait rows, their members, then the headers. Feature 020's purge turns the run row into a stub rather than
deleting it (constitution, Principle I), so the header's foreign key to `runs` never blocks it, and the purge calls the
function in its transaction before it writes the stub. A negative privilege test deletes a kept receipt and a retired
group as `sagat_app` and is refused; a test runs the function as the owner and sees every row gone. The down migration refuses while any group or member exists.

Transfers are not audited one record each: the header gains `transfer_count`, increased by each `Vacate` and
`Acquire`, and the `capacity.group.retired` record carries it. This changes the write set of the built `Vacate` and
`Acquire` (`crates/infra-postgres/src/capacity_transfer.rs`), which today insert one audit row per transfer and
store its id in `capacity_group_transfer_receipts.audit_id`, a `NOT NULL` column
(`20261226000000_capacity_transfer_receipts`): the migration of Task 039-2 makes the column nullable, and Task 039-7
changes the commands to write no audit row, to increase `transfer_count`, to delete the yield row, to set `held_since`
and the member's acquisition instant (`Acquire`) and `last_vacated_unleased_at` (`Vacate`). ADR-0156 records this
as an amendment of ADR-0078 section 3 and of Decision 710.

### 8. The codec opening

Today `published_collection_definition` (`crates/infra-ir/src/convert.rs`) accepts only an optional prefix of plain
source activities, then a chain of `forEach` nodes ending at `end`, each body one to eight activities, conditions,
switches, timers or event waits with local routes and one return; it refuses `chainPolicy`, `triggers`, an approval in a
body (pending scoped compensation, Decision 295 of feature 014), and a loop, approval, continue-as-new or return beside a
collection. This design opens, under the registry requirement:

| Shape | Today | After this design |
|---|---|---|
| A `forEach` inside a body | refused | accepted to a depth of 4 collection or loop levels, within the fan-out bound; a scoped-path member per root iteration |
| A loop (`while`, `repeatUntil`) inside a body | refused | accepted within the same depth |
| A child workflow inside a body | refused (not a body node kind) | accepted, except with cancel policy `detach` (`NESTED_CHILD_DETACH`); a ChildRoot member with its own chain that joins the group |
| A collection beside loops, conditions, switches, timers, event waits, child workflows and approvals (under the approval row below) at the top level | refused | accepted; each root collection iteration that may nest takes one group, each root loop that may nest one group; continue-as-new and return beside a collection stay refused |
| An approval inside a body | refused, pending scoped compensation | accepted only once Decision 295's dependency is met; Task 039-10 checks it and otherwise keeps the refusal (Gap 039-007) |
| `chainPolicy` with a collection | refused | still refused (continuation of a collection is not in this design) |
| `triggers` on a workflow with a collection | refused | still refused (Gap 039-006) |
| Source activities with `onError`, compensation, an effect policy or a database operation | refused | still refused |

The definition service refuses a nested document while `nested-collections-v1` is not recorded
(`NESTED_COLLECTIONS_DISABLED`) or its fence answers no, and a document whose fan-out bound passes the tenant's policy
(`NESTED_FAN_OUT_LIMIT`). The codec reads no switch: a stored nested document stays readable with the requirement
removed (the switch is read at publication only, as design 042 does).

The new codes, each with one surface and one meaning. A publication or activation refusal is an `IrError` variant
mapped by `crates/api-http/src/error.rs` (`definition_details`) to `422 VALIDATION_FAILED` with `details.reason`, as
`LAKE_DISABLED` is, listed in the `details.reason` text of `specs/001-platform-foundation/contracts/openapi.yaml`; a
runtime refusal is a closed `FailureCode` of the step (`crates/domain/src/execution/event.rs`), listed in
`events.asyncapi.yaml` and `docs/sdk-compatibility.md` with `sdks/error-catalog.json`.

| Code | Surface | Answer | Retryable | Meaning |
|---|---|---|---|---|
| `NESTED_COLLECTIONS_DISABLED` | publication, activation | 422 `VALIDATION_FAILED`, `details.reason` | no | the requirement is not recorded, or it is `draining` (the fence's `new_use` answers no) |
| `NESTED_FAN_OUT_LIMIT` | publication | 422 `VALIDATION_FAILED`, `details.reason` with the node and the bound | no | the version's bound passes the tenant's policy |
| `NESTED_CHILD_DETACH` | publication | 422 `VALIDATION_FAILED`, `details.reason` with the node | no | a child inside a nested body declares `detach` |
| `NESTED_FAN_OUT_LIMIT` | an active-version child's admission | the parent's child step fails with `FailureCode` `NESTED_FAN_OUT_LIMIT`, permanent (no retry, the parent's `onError` applies) | no | the child version's bound passes what is left to it |
| `NESTED_COLLECTIONS_INACTIVE` | the creator and a join (scheduler) | never returned: the insert trigger raises `nested_collections_fence`, the admission rolls back and is retried by the next tick, counted `inactive` | (internal) | the fence's `continue` answers no; after phase 2 a run cannot be admitted, so this only follows an operator's removal error |
| `NESTED_COLLECTIONS_INACTIVE` | the creation of a new run of an active nesting version that is neither a member child nor a chain successor (API start, schedule action, batch start, a top-level child start, a reset, a rerun), checked once by the store's run creation after authorization and tenant and namespace resolution, while the requirement is `draining` or absent | the API answers `409 CONFLICT` with `details.reason`; a schedule action and a batch item record it as their failure reason; a top-level child start fails the parent's child step with the permanent `FailureCode` `NESTED_COLLECTIONS_INACTIVE` | no | the fence's `new_use` answers no |
| `NESTED_RUN_NOT_MOVABLE` | a move of a code run (design 044) and the completion of a code continue-as-new to another version, for a run whose `capacity_member` flag is set | the move item is skipped with the reason; the completion is refused `COMMAND_INVALID` with the reason; the database's triggers raise `nested_member_version_fence` as the backstop | no | a member run keeps its version in release 0.3.0 (Requirement 039-015, Gap 039-010) |

A publication made while the requirement is `draining` therefore gets `NESTED_COLLECTIONS_DISABLED`; there is no
separate code for `draining`.

### 9. Activation and removal

The order: every condition of ADR-0075 (a) to (l) proven by its task; every engine runs its guard, creator and sweeper
with `SAGAT_NESTED_COLLECTIONS=on` and every control plane its definition service with it, so both list
`nested-collections-v1` (Requirement 039-011); the operator records the requirement; the first membership is created by
the first nested admission after that.

**A process that was not live at the activation.** It can reconnect and serve until its next heartbeat. It grants no
member lease, because both lease producers ask the serving gate whether the process uses `nested-collections-v1`
(`ServingGate::uses` of design 053, Requirement 053-006; `requires` in `crates/application/src/instances.rs` until 053 is
built), and a gate that does not answer yes leaves the guard failing closed on every member step (`UnsupportedScope`
for a scoped path, a refusal for a ChildRoot run); a test has a silent engine poll a member's step and be granted
nothing.

**An older binary.** A binary from before this design runs no guard and asks no gate, and Requirement 053-006 lets a
process not counted at the activation reconnect and serve for up to one heartbeat. So the database fences the lease
itself: a trigger `BEFORE INSERT OR UPDATE OF status, lease_token_hash ON steps` (an insert can carry its status from a
bind, the first `INSERT` of the step-status-writer inventory) refuses only a new grant, recognised by its lease token
rather than its status: a row with `NEW.lease_token_hash IS NOT NULL` that is inserted, or updated with
`NEW.lease_token_hash IS DISTINCT FROM OLD.lease_token_hash` (`steps_lease_consistent` makes every grant set a token,
so a grant straight into `RUNNING` is caught as well as one into `LEASED`), when its run's `capacity_member` flag is set, unless the transaction set the local setting
`sagat.capacity_guard` to `v1:` followed by its own `pg_current_xact_id()`, which the trigger compares with the running
transaction's id. The generic step writer sets `status` in every update it makes (`crates/infra-postgres/src/execution.rs`),
so a heartbeat's lease renewal or a cancellation of a step that stays `LEASED` with the same token fires the trigger
and passes: it is no new grant. A value left in a pooled session by mistake, or set by `ALTER ROLE ... SET`, names
another transaction and passes nothing. The trigger
reads the flag from `runs` by primary key, the row the claim has already locked (copying it onto `steps` would need an
update of every step when a root run is flagged later). The claim of a flagged run sets the setting (`SET LOCAL sagat.capacity_guard = 'v1:<pg_current_xact_id()>'`, in
the claim transaction, before it grants) for every lease of that run, its top-level steps outside every member included,
and only the guard's code path sets it. An older binary therefore cannot lease member work at all. The budget suite of
design 038, `integration_claim_transition_efficiency` (`tests/integration/claim-transition-efficiency/work_budget.rs`),
counts top-level statements only, so the trigger's lookup adds no counted statement; Task 039-13 measures its cost on a
claim of a run that is no member (one index lookup) and states it.
This is a version fence, not a boundary against the platform's own role: the runtime role could set the value, and
only this design's code does. A test in the mixed-writer suite of design 053 (Task 053-6) has an N-1 engine poll a
member's step and be refused.

**Deactivation.** Two phases (Requirement 053-007). Phase 1 marks the capability `draining`:
`nested_collections_required('new_use')` then answers no, so no nesting version is published or activated and no new
run of a nesting version is admitted; `nested_collections_required('continue')` still answers yes, so runs already
started keep creating their groups and members, and the guard keeps granting member leases, so those runs reach a
final state. Phase 2 takes no table lock: as for every form (a) fence of design 053, it takes the requirement row
`FOR UPDATE`, which waits for every transaction that called the fence (they hold the row `FOR KEY SHARE` until they
end, and every group or member insert calls it through its trigger), runs the drain check (no group is not `RETIRED`,
and no run of a nesting version is not final), refuses on any, and removes the row; an insert that calls the fence
after that sees no row and is refused, so a run admitted after phase 2 is refused at its admission, never stranded
half-grouped. The descriptor names fence form (a) with the "continues while draining" mode and that check. Design 053 knows only the plain form
(a), which refuses every write once the row is `draining`, and a `ServingGate::uses` that answers no to a `draining`
row; its follow-up Task 053-9 registers the "continues while draining" descriptor kind and the gate answer it needs,
and Task 039-18 waits for it. A test drains with a nested run in flight and sees that run finish and phase 2 then
pass. A binary rollback below the recorded requirement is unsupported (an older binary cannot parse a stored nested
version).

### 10. Telemetry and operability

`capacity_sweep_commands_total{operation, outcome}` (`operation` in `vacate`, `acquire`, `finish`, `vacate_path`,
`acquire_path`, `finish_path`, `yield_insert`, `yield_delete`; `outcome` in `applied`, `replayed`, `busy`, `stale`,
`refused`, `uncertain`), `capacity_group_held_seconds` (a histogram of the age of `HELD`),
`capacity_member_wait_seconds` (ready to acquired), `capacity_member_join_total{outcome}` (`created`, `busy`,
`inactive`), `capacity_sweep_deferred_total{reason}`, `capacity_sweep_backlog` (candidates left after a page),
`capacity_groups_stalled{state}` (groups not `RETIRED` with no transfer, finish or member change for 10 minutes, by
state), `capacity_members_unfinished` (members whose head or iteration became final more than ten sweeper tick intervals ago,
a duration read from the head's or iteration's final instant, and which are still `ACTIVE`), `worker_claim_skipped_total{reason}` (existing, gaining `yield_requested` and `scoped_not_holder`), the refusal
class enumerated with a test (ADR-0075 condition (h)); alerts on a group held over 5 minutes, on joins answered `busy`
for 5 minutes, on `capacity_groups_stalled` above 0 for 10 minutes and on `capacity_members_unfinished` above 0 for 10
minutes, and on `capacity_member_join_total{outcome="inactive"}` above 0 for 5 minutes (a nesting run refused by the
fence, which after a phase 1 means the drain rule is not being applied); and `docs/runbooks/workflow-primitives.md`
gains the nesting section (enable, read the wait, a stuck holder, a
stalled group, the two-phase removal).

### 11. The possession observer

A test-only trigger on `steps`, installed by the race tests and never by a migration, raises when a step moves to
`LEASED` while another member of the same group has an unexpired `LEASED` or `RUNNING` step. Every race test of
Requirement 039-012 runs with it and checks at its end that it never raised, so "never two members with a live lease"
is observed at every transition, not sampled.

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A lock cycle through the group header | Nothing waits on the header (ADR-0078 rule 1); the checked lock sequence in the code's order; a held-group test per writer |
| Two members lease at once | Possession gates every lease grant; the possession observer; Success criterion 039-002 races it with an unknown COMMIT |
| A member starves | The quantum, the yield row, oldest-ready-first acquire with the skip of an unserved member; Success criterion 039-003 |
| A lone holder is stranded by a stale yield | Selection 4 clears a yield whose waiting members stopped being runnable; the pause and time-out tests |
| A group's unit never frees | Finishes served first, `FinishPath` for every terminal outcome, compensating members runnable, the stalled-group alert |
| A unit is released twice | `owns_unit` and one decrement at retirement; the invariant of Requirement 039-016 tested at checkpoint, drain and cancellation |
| A nested admission halts the root run | The scoped guard gates only the steps inside a member's iteration |
| Descendants charge the tenant again and deadlock at limit 1 | The membership context skips the fresh-admission check; the limit-one tests, a multi-iteration loop included |
| A tenant at limit 1 runs thousands of inner iterations | The fan-out bound at publication and at an active-version child's admission |
| A noisy tenant delays another's finishes | Finishes first and tenant-weighted paging; the noisy-tenant test |
| A process without the guard leases a member's step | The engine lists the capability only with guard, creator and sweeper; the serving gate; the lease fence trigger refuses a binary that does not run the guard; the fence and the two-phase removal |
| A busy holder starves a child waiting to join | The join wait and the yield it triggers after a quantum; the 64-poller held-load test |
| A drain halts the nested runs it waits for | Phase 1 refuses only new nesting runs, publications and activations; runs already started continue; the inactive-join alert |
| A loop's group retires in the middle of the loop | One member per root loop, finished only when the loop scope is final; the delay test |
| A successor run escapes the guard | `capacity_member` enforced by triggers |
| Transfers fill the audit table | No audit row per transfer: a counter on the header, reported at retirement; receipts kept 15 minutes |
| Receipts or retired groups grow without bound | Retention of Requirement 039-009; `capacity_purge_retired` with feature 020's purge (Gap 039-008) |
| A run without nesting pays for it | No membership, no change to its path; the design-038 budget suite |
| A zombie worker or a blocking activity holds possession | Gap 039-001; the HELD-age alert |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Membership commits with the admission; each command compares and sets revision and epoch in one transaction; commit uncertainty re-sends the retained command (Decision 713); the unit has one owner and one release |
| II. Modular domain, ports and adapters | Pass | Possession rules in `crates/domain/src/execution/capacity.rs`; statements in `crates/infra-postgres`; the sweeper in `crates/scheduler` |
| III. External effects are at-least-once; sagas compensate | Pass | Nesting adds no external effect; compensation inside a nested scope keeps feature 037's rules and runs after retirement outside possession |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive schema; the codec opens only under a registry requirement fenced by form (a) of design 053; stored documents stay readable; groups exist only for versions published with nesting. Condition: the schema is not rolled back once a receipt exists (ADR-0078 section 6), and a binary is not rolled back below the recorded requirement |
| V. Code, tenant boundary and secrets are isolated | Pass | Every row carries tenant and namespace; the runtime role gets column-scoped inserts under insert invariants and no delete of headers or members; the fan-out bound keeps a tenant within its limit; no tenant label |
| VI. Observability and audit are part of the feature | Pass | Member creation, finish and retirement audited, the retirement with its transfer count; metrics with closed labels; HELD-age and stalled-group alerts |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Each transfer writes its outbox hint; a lost hint is recovered by the level-triggered sweeper |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Requirement 039-012's matrix in both orders with barriers and the possession observer; the 100,000-instance workload on a separate host |
| Delivery Constraints | Pass | PostgreSQL as authority, no new component; the latency target measured on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Joining | One tenant unit per root, deadlock-free at limit 1 (Decision 705) | A unit per descendant: deadlock at limit 1 (ADR-0030 §14) |
| Possession and a sweeper | The tenant limit bounds what is leased at once, not only what is admitted (Owner question 706, ADR-0075): at limit L, at most L members lease work | Joining without possession, bounded by the fan-out bound alone: the limit would then bound instances while up to L × the bound members (child workflows included) lease work at once, which is not the limit the owner chose; the cost of possession is that the members of one group lease one at a time (Requirement 039-001) |
| A quantum and a yield row | Fairness among members (Gap 718); a busy holder is never quiescent without it; the row needs no header lock the holder's claims keep | The yield on the header: almost always `Busy` under the holder's claims |
| A fan-out bound | The tenant limit bounds groups, not iterations | No bound: a tenant at limit 1 runs thousands of iterations |
| A scoped guard and `FinishPath` | A forEach in a forEach must lease and finish | Leaving scoped paths fail closed: the commonest nesting cannot run |
| Scoped-path commands of their own | Decision 706; the binding differs from a ChildRoot's | One command with a discriminated binding: rewrites the built and tested ChildRoot commands |
| A chain for every member child | ChildRoot membership is bound to a chain | Keying ChildRoot members by run: rewrites the built commands, the unique index and the guard |
| One group and one member per root loop | A loop carries its unit from iteration to iteration, and its iterations are sequential | A group per loop iteration: the carried unit is released twice; a member per iteration: a transfer per iteration and a group retired during a delay |
| A join wait | A busy holder's claims keep the header locked, so a joining child needs a way through | Retrying the join each tick alone: a busy holder can starve it |
| Receipt retention and a transfer counter | Bounded storage (Gap 720) | Keep for ever: unbounded growth at a rate the author controls; an hourly audit table: a hot upsert inside every transfer for a count the header holds |
| A lease fence trigger | An older binary runs no guard | Trusting every binary to run the guard: one not yet counted could lease member work for a heartbeat |
| A checked lock-sequence file | ADR-0078 Open; prose drifts | Prose in the ADR |
| The fence of design 053 and a two-phase removal | An unlocked read of the requirement races its removal | A one-phase `verify` (rejected by ADR-0131, option 6) |

## Alternatives Considered

- **Keeping nesting in feature 014** with its original criteria: rejected by Owner decision 2026-11-12-6.
- **Gating scheduler transitions on possession**: rejected by the owner (Owner question 706).
- **Inline `Vacate` in the worker or step transaction**: rejected by the owner (Owner question 705).
- **Counting each descendant against the tenant limit**: deadlocks at a limit of 1.
- **A per-tenant active-instance ceiling instead of the fan-out bound**: it would block admitted nested work at its
  ceiling, which ADR-0030 §14 forbids; the fan-out bound is checked before any work is admitted.
- **Letting a detached child leave its group**: it needs a unit of its own whose charge could block admitted work;
  refused for now (Gap 039-009).
- **Joining without possession**, with concurrency bounded only by the fan-out bound: simpler (no sweeper, no
  transfers), but the tenant limit would bound admitted groups rather than leased work; possession stays because Owner
  question 706 and ADR-0075 keep the limit on what runs. Its visible cost, members of one group leasing one at a time,
  is stated in Requirement 039-001.
- **A member per loop iteration** (the first answer to the correctness review): a transfer and two audit records per
  iteration, and a group that retires during a delay; replaced by one member per loop.

## Non-Functional Requirements

See Requirements, Non-functional (Non-functional requirements 039-001 to 039-003).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Four slices in order; slices 2 and 3 can run in parallel in two lanes. Every file has one owner, with one listed
exception: the rule of `scripts/check-architecture.py` that refuses an `INSERT INTO workflow_versions` without `nesting`,
`fan_out_bound` and `fan_out_open` is added by lane A in Task 039-21, in the change that makes the inserts pass it.

| Lane | Owns |
|---|---|
| A | `crates/infra-postgres` (every statement, migration and adapter, `src/definitions.rs` and `src/execution/ad_hoc.rs` included, so the version inserts that bind `nesting` and `fan_out_bound` are lane A's, calling lane B's `nesting_of` and `fan_out_of`); `crates/scheduler`; in `crates/application`, the two lease producers (`src/worker.rs`, `src/worker/scoped_claims.rs`), the execution ports (`src/execution.rs`, `worker_capacity_available` with its instance path), the capacity ports (`src/capacity_*.rs`, the scoped commands beside them) and the error variants they return (`Busy`, `NESTED_COLLECTIONS_INACTIVE`); `apps/execution-engine` (the switch, the capability listing, the sweep and quantum settings, the guard's wiring); the move's skip check of a member run (`crates/domain/src/batch.rs`, `crates/application/src/runs/batch_items.rs`) and the completion refusal of a member run's continue-as-new to another version (the code completion path of design 044's Task 044-22), when they are written by this design's tasks rather than design 044's follow-up Task 044-42; the test targets |
| B | `crates/domain`; `crates/infra-ir`; in `crates/application`, the definition service (`src/definitions.rs`, its `IrError` variants) and `src/instances.rs` (the `nested-collections-v1` value of `Capability`); `crates/api-http` (the mapping of the new codes of Technical Plan 8); `apps/control-plane` (its switch and listing); the six SDKs; `scripts/` |

Order points: Task 039-4's domain functions (lane B) merge before Tasks 039-5, 039-7 and 039-8 (lane A) call them;
Task 039-2's migration (lane A) merges before any lane-B code reads the new columns, and its new columns all have
defaults (`nesting` false, `fan_out_bound` null), so today's version inserts keep working until Task 039-21; Task
039-18, which alone creates `nested_collections_required(mode)` in a migration of its own, waits for design 053's Task
053-5 (the requirement row's `active`/`draining` state, `ServingGate::uses`) and Task 053-9; Task 039-23 waits for
Task 053-6 (the mixed-writer suite); Task 039-25 merges before Task 039-10 opens the codec to a child inside a body;
Task 039-19's `code_run_pins` trigger lands in the change that runs design 044's Task 044-6, or after it: when this
design lands first, `code_run_pins` does not exist yet and design 044's follow-up Task 044-42 creates the trigger, the
skip reason and the completion refusal with the table; Task 039-17 (activation) closes both.

**Slice 1, rules and checks** (lane B): Tasks 039-1, 039-3, 039-4. **Slice 2, creator, predicate, sweeper** (lane A):
Tasks 039-2, 039-5, 039-6, 039-7, 039-8, 039-9, 039-18, 039-19, 039-20, 039-21, 039-22, 039-23, 039-24, 039-25.
**Slice 3, the codec and the SDKs** (lane B): Tasks 039-10, 039-11; Task 039-21 (lane A) binds what Task 039-10
computes, after it. **Slice 4, proof and activation** (both): Tasks 039-12 to
039-17.

- [ ] T039-1 Write this design out, decide the open items of ADR-0078 in ADR-0154 and ADR-0156, accept ADR-0078 and
  record the delegate rows.
  Refs: Requirement 039-004, Requirement 039-005
- [ ] T039-2 Migration: column-scoped `INSERT` grants on groups and members with the insert trigger of Requirement
  039-008 (header `HELD` with counters 1, `shard` and `namespace_id` from the root run, `owns_unit` only when the
  `collection_iterations` row it names is slot-held or the `loop_scopes` row it names is `RUNNING` or `DELAYING`,
  `reserved_bound` from the version, raising when the version's bound is null, and added to `nested_reserved` under the tenant's
  ceiling; member `ACTIVE`, in its header's namespace; no member of a `RETIRED` group; both calling the fence in mode
  `continue`); `root_owner_node` and `iteration_index` on the header, insertable and never updated, and the partial
  unique index on (`tenant_id`, `root_run_id`, `root_owner_node`, `iteration_index`) `WHERE owns_unit AND possession <>
  'RETIRED'`, with tests that a second unit-owning group on one slot and a member in another namespace are refused; the unique index on ACTIVE ChildRoot memberships per chain and the partial unique index on ACTIVE scoped-path
  memberships per root iteration or root loop; the new columns of Technical Plan 2 with their types and defaults
  (`namespace_id`, `root_run_id` with its foreign key and index, `shard smallint NOT NULL` set by the trigger from
  `run_shard(root_run_id)`, `held_since`, `owns_unit`, `transfer_count`, `reserved_bound` on the header; `acquired_at`,
  `last_leased_at`, `last_vacated_unleased_at` on the member; `fan_out_share` on runs, written once by trigger; `nested_reserved` on
  `tenant_iteration_usage` with its check constraint; `fan_out_open` on versions) and the exact grant list of Technical Plan 2; `capacity_group_yields` and
  `capacity_group_join_waits` with their keys and no foreign key to the header, and a held-header test of both inserts;
  `capacity_group_transfer_receipts.audit_id` made nullable; the relaxed `collection_iterations_slot_consistent` and
  `loop_iterations` checks that allow a descendant `ADMITTED AND NOT slot_held`, with the trigger that allows it only for
  a flagged run whose scope's owner path is not the root or which is a member child (Requirement 039-016); the lease fence trigger on `steps` (Technical Plan 9);
  `capacity_member` on runs (`NOT NULL`, the successor-copy trigger on `chain_segments`, the member-insert and
  never-cleared triggers of Requirement 039-015); the `nesting` flag and `fan_out_bound` on versions; `nested_group_fan_out` on
  `tenant_collection_policies` (`integer NULL`); `INSERT` grants on the scoped columns of the finish receipts, which
  `FinishPath` writes with kind `SCOPED_PATH`; `capacity_path_transfer_receipts` (only `VACATE` and `ACQUIRE`, mirroring
  `capacity_group_transfer_receipts` with the scoped binding and a nullable `audit_id`); the retention triggers on the
  receipt tables and the replaced delete guards of headers and members, both exempting a delete by the table's owner,
  with `capacity_purge_retired` deleting receipts, side rows, members, then headers; the four partial indexes of
  Technical Plan 2; the `nested-collections-v1` registry value (the fence function is Task 039-18's); the down
  migration's refusal while any group or member exists;
  with the migration checklist and negative privilege tests for each grant, trigger and function.
  Refs: Requirement 039-008, Requirement 039-009, Requirement 039-011, Requirement 039-015
  Owner-gate: OD-2026-11-26-141, OD-2026-11-27-462
- [ ] T039-3 Write `worker-lock-sequence.json` with the guarded-claim sequence in the code's order (Technical Plan 6;
  the other sequences are added by the lane-A tasks that write their code) and extend
  `scripts/check-capacity-activation.py` with the `// lock-seq: <sequence id>/<step>` marker check (a sequence whose
  function does not exist yet reported pending, not passed), the `NOWAIT`/`SKIP LOCKED` rule, the allow-list of command
  callers (the sweeper only) and of the guard's enablement (the engine's composition), and the check of ADR-0075
  condition (g); negative fixtures seen failing.
  Refs: Requirement 039-010
  Owner-gate: OD-2026-11-26-146
- [ ] T039-4 Domain: the quantum and the yield, the oldest-ready choice with the skip, the scoped-path command rules and
  the `FinishPath` proof over the lifecycle, the membership context, the fan-out bound, the sweeper's six selections in
  their order, as pure functions with property tests.
  Refs: Requirement 039-005, Requirement 039-006, Requirement 039-014
  Owner-gate: OD-2026-11-26-142, OD-2026-11-26-143, OD-2026-11-26-145, OD-2026-11-27-443
- [ ] T039-5 The creator: group and first member at a root collection admission or root loop entry of a nesting version
  only, after `nested_collections_required('continue')` (the run's admission after `'new_use'`), with `namespace_id`
  and `root_run_id` on the header; one member for a whole root loop, kept across its iterations and delays; ChildRoot
  join with a chain
  for every member child (one segment without `chainPolicy`, `link_child_chain` at any instance path), the header's
  compare and set and `active_members` under `NOWAIT`; `Busy` and `NESTED_COLLECTIONS_INACTIVE` retried by the next tick
  with no attempt consumed and counted, a busy join recording its join wait; the descendant admissions of `primitives.rs` and `loop_progress.rs` through the
  membership context behind the run flag, each recorded `ADMITTED` with `slot_held` false; the root run flagged in the
  transaction that creates its first group; the creator's and the join's lock sequences added to
  `worker-lock-sequence.json` with their markers; the four decrement sites skipped for grouped iterations and loops and one
  decrement at retirement; audit and metrics; the limit-one tests (a child with and without `chainPolicy`, a child
  containing a collection, a three-iteration `while` with a delay containing a collection with `active_iterations`
  asserted after every checkpoint and before and after `fire_delay`, a cancelled iteration whose children compensate), the policy-reduction test, a chainless child
  inside a body refused a lease while not the holder, and a test that a top-level child of a version published before
  the requirement creates no group.
  Refs: Requirement 039-002, Requirement 039-016, Success criterion 039-001
  Owner-gate: OD-2026-11-26-141, OD-2026-11-27-391, OD-2026-11-27-456, OD-2026-11-27-439, OD-2026-11-27-440, OD-2026-11-27-444
- [ ] T039-6 The sweeper's predicate `runnable_member_work!` in `crates/infra-postgres/src/capacity_sweep.rs`, written
  from the candidate query of `workers.rs` (compensations of stopping runs runnable, paused runs not), and the per-path
  scoped clause added to `claimable_query!`'s capacity clause, the two kept as separate texts; the differential test
  that runs both on a corpus with paused, stopping and compensating runs and scoped members; the guarded poll's rotating
  sample (the cursor in process memory, one per (tenant, namespace), on `(ready_at, run_id)`) and claimable window (Gaps
  715, 724, 725); a limit-1 test that
  cancels a root iteration whose two ChildRoot members both compensate.
  Refs: Requirement 039-007
  Owner-gate: OD-2026-11-26-147
- [ ] T039-7 `VacatePath`, `AcquirePath` and `FinishPath` with their receipts and quiescence over the iteration path,
  the chain first in their lock order, the retiring finish's unit release, the `FinishPath` proof over the iteration's
  or the loop scope's lifecycle; the built ChildRoot commands fenced on the group's shard and releasing the unit when
  they retire a group that owns it; the changed write set of the built `Vacate` and `Acquire` in
  `crates/infra-postgres/src/capacity_transfer.rs` (no audit row per transfer, `transfer_count`, the yield row deleted,
  `held_since`, `acquired_at`, `last_vacated_unleased_at`); the scoped guard in both lease producers with the instance
  path (a change of the port `worker_capacity_available`), the yield row of the header's epoch, the loop gate on (run,
  root node), `last_leased_at` set once after the first lease grant of the member is persisted, and
  `sagat.capacity_guard` set for every lease of a flagged run; `FinishPath` writing `capacity_group_finish_receipts`
  (kind `SCOPED_PATH`) and the path transfers writing `capacity_path_transfer_receipts`; the commands' lock sequences
  added to `worker-lock-sequence.json` with their markers; after Task 039-4; tests "acquired, leased, vacated",
  "acquired, never leased, vacated" and "acquired, refused by the activity quota, vacated" (the last two are skipped
  by the next `Acquire`), a yield
  row of an earlier epoch ignored by the guard, a loop with a delay whose member is not finished while the scope is
  `DELAYING`; compensation after
  retirement leased outside possession; the commit uncertainty classifier shared; a test per terminal outcome (seven)
  and body shape of `FinishPath`, the race tests of the scoped guard, and the limit-1 test of a compensated successful
  iteration.
  Refs: Requirement 039-003, Requirement 039-004, Requirement 039-005, Requirement 039-016
  Owner-gate: OD-2026-11-26-145, OD-2026-11-27-390, OD-2026-11-27-445, OD-2026-11-27-450, OD-2026-11-27-452, OD-2026-11-27-453
- [ ] T039-8 The `capacity_sweep` pass: six selections in order, finishes first, served by the replica holding the
  group's shard, tenant-weighted pages, the yield budget, deferral counted, operation keys from the whole tuple, no lock
  held across a command; held-lock, restart, lost-hint and single-member no-churn tests; pause-after-yield and
  sibling-times-out-after-yield tests; selection 3 on a join wait a quantum old; transfers counted per quantum; a noisy tenant beside a limit-1 tenant; an
  activity no worker serves.
  Refs: Requirement 039-004, Requirement 039-005, Requirement 039-006
  Owner-gate: OD-2026-11-26-142, OD-2026-11-26-143, OD-2026-11-27-443
- [ ] T039-9 Receipt retention pass (15 minutes and the latest 16) and the retirement record's transfer count, with
  their tests;
  `capacity_purge_retired` (revoked from `PUBLIC`, executable by its owner only; it requires feature 020's
  `retention_purge_authorizations` row for this transaction and run; receipts, side rows, members, then headers; the
  receipt and guard triggers exempting a delete by the table's owner, every relation written with its schema) and its
  tests: run as the owner with the authorization, refused without it, refused as `sagat_app`; called by feature 020's
  purge before it writes the run's stub, when it exists (Gap 039-008).
  Refs: Requirement 039-009
  Owner-gate: OD-2026-11-27-438, OD-2026-11-27-447, OD-2026-11-27-453, OD-2026-11-27-463
- [ ] T039-10 Open the codec to the shapes of Technical Plan 8 under the registry requirement; `nesting_of` and
  `fan_out_of` computed from the parsed document, the bound stored for every version; `NESTED_CHILD_DETACH` and `NESTED_FAN_OUT_LIMIT`; a `parse_stored`
  test with the requirement removed; refusals of every unlisted shape.
  Refs: Requirement 039-001, Requirement 039-014
  Owner-gate: OD-2026-11-27-437, OD-2026-11-27-441, OD-2026-11-27-454
- [ ] T039-11 The six SDKs: nested bodies through the existing builders, a compile test that every present call still
  builds, and the scenarios `nested-collections` (build) and `nested-collections-runtime`.
  Refs: Requirement 039-013
- [ ] T039-12 The race matrix of Requirement 039-012 and ADR-0075 condition (e) as the runtime role, with the possession
  observer (Success criterion 039-002), and the limit-one and fairness tests (Success criteria 039-001 and 039-003).
  Refs: Requirement 039-012, Success criterion 039-001, Success criterion 039-002, Success criterion 039-003
- [ ] T039-13 The design-038 budget lines for a run without nesting, with the requirement off and on, for admission,
  claim and completion (Non-functional requirement 039-002) in `integration_claim_transition_efficiency`, which counts
  top-level statements only; the cost of the lease fence trigger's lookup on a claim of a run that is no member,
  measured and stated; and the generic-plan cost of the guard's second read (Gap 726).
  Refs: Non-functional requirement 039-002
- [ ] T039-14 The dedicated full-stack nested run on the integrated branch before activation.
  Refs: Requirement 039-011
  Owner-gate: OD-2026-11-26-148
- [ ] T039-15 The 100,000-instance nested workload on a separate database host, twice, with the dated report, which
  states the rate of `nested_collections_required` calls and their cost (Technical Plan 2).
  Refs: Non-functional requirement 039-001, Success criterion 039-004
- [ ] T039-16 Telemetry, the alerts (HELD age, busy joins, inactive joins, stalled groups, unfinished members), the refusal-class
  enumeration test, and the runbook's nesting section.
  Refs: Requirement 039-011
- [ ] T039-17 Activation: the configuration switch and the capability listing of Requirement 039-011 (the engine only
  with guard, creator and sweeper), the fence and the two-phase removal with its descriptor, the registry requirement,
  the gate record of ADR-0075 (a) to (l) with each condition's evidence, `docs/traceability.md`, the roadmap row and
  the release notes, which state that the members of one group lease one at a time (Requirement 039-001); feature
  014's moved clauses closed here. Whenever `code_run_pins` exists, the activation's gate record also requires the
  move and continue-as-new tests of Task 039-19 (or of design 044's Task 044-42) to pass.
  Refs: Requirement 039-011
  Owner-gate: OD-2026-11-27-393, OD-2026-11-27-442
- [ ] T039-18 The fence function `nested_collections_required(mode)` (`FOR KEY SHARE` on the requirement row,
  owner-owned, `search_path` pinned; `new_use` yes only when `active`, `continue` yes when `active` or `draining`) called
  by the creator, a new run's admission, publication and activation; the guard's gate answering yes while `draining`;
  phase 2's drain check after its `FOR UPDATE` on the requirement row, with no table lock; the insert triggers of both
tables calling the function in mode `continue` and raising `nested_collections_fence`; an unknown mode raising; the
run-start refusal `NESTED_COLLECTIONS_INACTIVE` on each surface of Technical Plan 8; held-lock tests of a creator racing each phase in both orders; a drain
  with a nested run in flight that finishes, after which phase 2 passes; the function created by this task alone, in a
  migration of its own, after Task 053-5 (the requirement row's state and `ServingGate::uses`) and Task 053-9; a test in `apps/execution-engine/tests/registry.rs`
  that an engine with the guard off and the passes on does not list the capability; a test that a silent engine's poll
  of a member's step grants nothing; `new_use` called once by the store's run creation for every run of a nesting
  version that is neither a member child nor a chain successor, after authorization and tenant and namespace
  resolution, with tests that a reset and a rerun of a nesting-version run are refused while draining and that a caller
  without permission gets `404` or `403`, never the `409`.
  Refs: Requirement 039-011, Requirement 039-003
  Owner-gate: OD-2026-11-27-442, OD-2026-11-27-449, OD-2026-11-27-462
- [ ] T039-19 The `capacity_member` triggers of Requirement 039-015: the `AFTER INSERT` trigger on `chain_segments`
  that copies the predecessor run's flag, `fan_out_share` and `capacity_role` onto the successor run and refuses a
  successor whose `workflow_version_id` differs from a flagged predecessor's, the member-insert refusal and the
  never-cleared rule; `runs.capacity_role` bound at insert and its never-changed trigger (the segment copy excepted);
  the `BEFORE UPDATE` trigger on `code_run_pins` that refuses a change of `code_version_id` for a flagged run, and the
  completion of a code continue-as-new to another version of a flagged run refused `COMMAND_INVALID` with
  `NESTED_RUN_NOT_MOVABLE`; tests: a continue-as-new successor of a member child, flagged by the segment trigger,
  refused a lease while not the holder; a member insert refused for an unflagged run; a move of a flagged code run
  skipped `NESTED_RUN_NOT_MOVABLE` with its pin unchanged; a code continue-as-new of a flagged run asking
  `version: active` for another version refused at its completion, and the segment trigger refusing the same successor
  when the completion check is bypassed; an update of `capacity_role` or `fan_out_share` by `sagat_app` refused (no
  grant), and by any role other than the table's owner refused by the trigger; the three checks on `runs` refusing a
  flagged run with a null role, a member child with a null share and an unflagged member child; a child of a flagged
  parent with a null role refused; the trigger on `collection_iterations` and
  `loop_iterations` that allows `ADMITTED AND NOT slot_held` only for a flagged run whose scope's owner path is not the
  root or whose `capacity_role` is `MEMBER_CHILD`, with tests that a root iteration of an unflagged run,
  and a root iteration of a flagged root run, admitted without its charge are refused.
  Refs: Requirement 039-015
  Owner-gate: OD-2026-11-27-446, OD-2026-11-27-462
- [ ] T039-20 The invariant of Requirement 039-016 as a consistency check run by the integration suites after every
  checkpoint, drain and cancellation test of Tasks 039-5 and 039-7, per tenant: `active_iterations` of
  `tenant_iteration_usage` equals the count of `collection_iterations` and `loop_iterations` rows with `slot_held` whose
  root owner node and iteration index (a loop: its root owner node) no scoped-path member of a non-`RETIRED` group of
  the same run covers, plus the count of non-`RETIRED` `capacity_groups` with `owns_unit`; the query is written in the
  task's support module and seen failing on a seeded double release.
  Refs: Requirement 039-016
  Owner-gate: OD-2026-11-27-444
- [ ] T039-21 (lane A, after Task 039-10) `nesting_of` and `fan_out_of` bound by every version-insert path
  (`crates/infra-postgres/src/definitions.rs` and `crates/infra-postgres/src/execution/ad_hoc.rs`) and the rule of
  `scripts/check-architecture.py` that refuses an `INSERT INTO workflow_versions` that does not bind `nesting`,
  `fan_out_bound` and `fan_out_open`, with a negative fixture for each of the three.
  Refs: Requirement 039-002
  Owner-gate: OD-2026-11-27-446
- [ ] T039-22 The fan-out bound at an active-version child's admission, refused with `NESTED_FAN_OUT_LIMIT`, and the
  wrapped-collection test (a one-item outer `forEach` around a `forEach` of concurrency above the policy value refused
  at publication; one within it runs at most the bound at once); the share left to a child as the group's
  `reserved_bound` divided by the product down to the child call, never the current policy; a version with a null bound counted as its computed product;
  `QuotaResource::NestedGroupFanOut` on the audited `put_setting` path with its API and contract entries (tenant scope
  only, a namespace value refused; default limit and default ceiling 256, maximum 4,096), the `None` branch of each
  resource resetting only its own column, 256 when no row; `QuotaResource::NestedLiveIterations` as an operator ceiling
  only (16,000 by default, at most 20,000); `runs.fan_out_share` bound at every member child's insert, pinned or
  active-version, and the grandchild check against it; `fan_out_of` recursive over pinned children and summing `parallel` branches, and `fan_out_open`;
  `reserved_bound` and `nested_reserved` written by the group's insert trigger and the retiring `Finish`; tests that a
  write is audited, that clearing one setting keeps the other, that a lowered value refuses the next root admission of
  an existing version, that a pinned grandchild's bound counts through a pinned child without collections, that a
  `parallel` of children counts each branch, that a grandchild of an active-version child is checked against its
  parent's share, and that a root admission passing `nested_live_iterations` waits; the child-admission refusal as
  the permanent step failure `NESTED_FAN_OUT_LIMIT`; a code version (design 044) whose envelope lists children refused
  as a pinned child at publication and as an active-version child at admission, and one whose envelope lists none
  admitted with a bound of 1; the group insert trigger raising when the version's bound is null; `resolve`'s default-limit
  case extended to `NestedGroupFanOut` and the SQL rule tested against `resolve`; a child of a `MEMBER_CHILD` parent
  with a null `fan_out_share` refused, a child of a flagged parent with a null `capacity_role` refused, and an
  active-version child of a `ROOT` parent admitted with its share taken from the group; `fan_out_share` write-once (a
  second write refused, and a null-to-value update by any role other than the table's owner refused); a test that lowers the policy, admits a root iteration of an open version, raises the policy, then
  admits a child of that group and sees its share bounded by the group's `reserved_bound`.
  Refs: Requirement 039-014
  Owner-gate: OD-2026-11-27-437, OD-2026-11-27-454, OD-2026-11-27-459
- [ ] T039-23 The lease fence: the trigger `BEFORE INSERT OR UPDATE OF status, lease_token_hash ON steps` that refuses
  only a new grant (Technical Plan 9) for a run whose `capacity_member` flag (read from `runs`) is set unless
  `sagat.capacity_guard` is `v1:` with this transaction's `pg_current_xact_id()`; after Task 053-6; a test that a
  top-level step of a flagged root run leases through the guard's setting and is refused without it; tests that a lease
  renewal, a cancellation and a `LEASED` to `RUNNING` move of a flagged run's leased step pass without the setting; a
  test that a direct grant into `RUNNING` (a new token written with `status = 'RUNNING'`) is refused without it; a test that a value set for
  another transaction (session level, or by `ALTER ROLE ... SET`) passes nothing; a test that a claim without the guard is refused; the
  mixed-writer test of design 053 (Task 053-6) in which an N-1 engine polls a member's step and is refused.
  Refs: Requirement 039-003
  Owner-gate: OD-2026-11-27-455, OD-2026-11-27-460
- [ ] T039-24 Join waits: `capacity_group_join_waits` written after a busy join (with the waiting parent and
  `last_busy_at`, refreshed on each busy retry, through `INSERT ... SELECT ... WHERE EXISTS` a non-retired header of the
  same tenant and namespace) and deleted by the join that succeeds; selection 3 on a live wait a quantum old; selection 4
  deleting a wait that is not live; a pause-after-busy-join test (the waiting member's run paused after its busy join:
  the wait goes stale, the yield is cleared and the holder leases again); a test that a side row for a retired or
  another tenant's group is never inserted; a held-load test with 64 holder pollers on one group and a child that joins
  within two quanta plus the longest holder step, run with `SAGAT_TEST_AS_RUNTIME_ROLE=1` so the wait's upsert uses
  only the column grants of Technical Plan 2.
  Refs: Requirement 039-005, Requirement 039-006
  Owner-gate: OD-2026-11-27-451, OD-2026-11-27-461
- [ ] T039-25 (lane A) Children and scopes at non-root paths: `pending_children` returns children at any instance
  path; `PendingChild` and the child run's parent reference carry the parent instance path; `start_child`,
  `child_started`, the child's finish and its cancel key the parent step by (run, instance path, node); in
  `PgRunTx::commit` a member child without `chainPolicy` gets a one-segment chain through
  `initialize_chain_in_transaction` (deadline of Technical Plan 2), and after that chain exists the group `NOWAIT`, the
  header's compare and set and the member insert; the child run's `capacity_member` bound at its `INSERT`; the
  scheduler's collection and loop entry for an owner at a non-root path (the scope tables already key by owner path);
  tests per shape (a child inside a collection body, a child inside a loop body, a collection inside a collection, a
  loop inside a collection) seen failing first on today's root-only paths; before Task 039-10 opens the codec.
  Refs: Requirement 039-001, Requirement 039-002
  Owner-gate: OD-2026-11-27-457

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers (`scripts/check-architecture.py`, `scripts/check-agent-config.py`, `scripts/check-design-docs.py`),
`scripts/check-capacity-activation.py`, the unit tests of the crates it changes and the suites it touches,
also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. A test is seen failing for the right reason first; races use held locks or
barriers, never sleeps, and run with the possession observer. `integration_workflow_primitives` and
`fault_workflow_primitives` run unmodified and with the nested tests by name; the budget suite of design 038 runs for
every change of a claim or a completion. A hand mutation per rule (take the group with a plain `FOR UPDATE`, skip the
quantum, let a non-holder lease, decrement the unit at a grouped checkpoint, leave the successor flag unset) is caught
by a named test.

## Revisions

- **2026-11-26, first architecture review** (`docs/designs/039-nested-collections.reviews/2026-11-26/round-1/architecture.md`):
  the scoped guard and `FinishPath`, descendant admission, groups only for versions published with nesting, the unit
  released at retirement, the activation's configuration and removal, and the yield (Decision 039-011).
- **2026-11-27, security and correctness reviews** (`docs/designs/039-nested-collections.reviews/2026-11-27/round-1/`):
  the capability listed by the engine that runs the guard; the predicate as the candidate query has it; the fan-out
  bound; the fence of design 053 and the two-phase removal; the `capacity_member` triggers; the yield as a row, cleared
  when stale, and vacate only of a yielded or idle holder; finishes first and tenant-weighted paging; the skip of an
  unserved member; hourly transfer audit and 15-minute receipts; the purge function; insert invariants; the join's
  header write; the lock sequence in the code's order; `nesting` derived; a chain for every member child; one group
  per root loop; `owns_unit` and one release; compensation after retirement; detach refused; timing restated; stalled
  alerts; the possession observer (Decisions 039-012 to 039-018, ADR-0156).
- **2026-11-27, EGM design critique** (`docs/designs/039-nested-collections.reviews/2026-11-27/round-1/egm-design.md`):
  the drain rule (phase 1 refuses only new nesting runs, publications and activations; runs already started continue,
  design 053 follow-up Task 053-9); one member per root loop, finished only when the loop scope is final; join waits;
  the guard's `last_leased_at`; the header's `namespace_id` and `root_run_id`; no audit row per transfer and no hourly
  table; the fan-out bound stored for every version and the policy column; possession weighed against joining without
  it, and the visible cost stated; the lease fence trigger for an older binary; terms, lanes, budgets named, the yield
  epoch, the gate row of Task 039-5 and the rate of the fence (Decisions 039-019 to 039-022, ADR-0156 revision).
- **2026-11-27, EGM implementation-readiness review**
  (`docs/designs/039-nested-collections.reviews/2026-11-27/round-1/egm-implementation-readiness.md`): Task 039-25 for
  children and scopes at non-root paths; every file with one lane; the order points on Tasks 053-5 and 053-6 and the
  lock-sequence markers; the side tables' keys without a foreign key to the header; the new columns, their types and
  the exact grants; the purge exempted by the table's owner; `FinishPath`'s receipt table; `last_leased_at` after the
  lease is persisted; the null fan-out bound and the policy's quota resource; descendants without a slot and the
  invariant's query; the error-code table; the two predicate texts and the cursor's home; the lease fence's trigger
  events, flag source and budget (Decisions 039-023 to 039-025).
- **2026-11-27, security re-read** (`docs/designs/039-nested-collections.reviews/2026-11-27/reread/security.md`): the
  fan-out bound recursive over pinned children and `parallel` branches, the share carried on member child runs, the
  tenant-scoped policy rechecked at every root admission and the operator ceiling `nested_live_iterations` reserved
  per group; the lease fence on new grants only, its setting bound to the transaction; live join waits; the successor
  copy on `chain_segments`; the fence called by the insert triggers, phase 2 without table locks, the run-start
  refusal; the relaxed slot check for flagged runs only; the purge function revoked from `PUBLIC` and gated on
  feature 020's authorization; the header's `namespace_id` and `owns_unit` checked by its trigger; side rows inserted
  only for a live header; ADR-0156 and the registers aligned (Decisions 039-026 to 039-031).
- **2026-11-27, second security re-read** (`docs/designs/039-nested-collections.reviews/2026-11-27/reread-2/security.md`):
  code versions (design 044) that list children refused as members; the header's root owner node and iteration index
  with the slot check and a unique unit owner per slot, members in their header's namespace; the column grant of the
  join-wait upsert; the lease fence keyed on a new lease token; a null bound raising, `resolve`'s default for the
  fan-out policy, a null share refused and the share written once; the relaxed slot check narrowed to inner scopes and
  member children; `new_use` once in the store's run creation, after authorization, covering reset and rerun
  (Decision 039-032).
- **2026-11-27, third security re-read** (`docs/designs/039-nested-collections.reviews/2026-11-27/reread-3/security.md`):
  a member run keeps its version in release 0.3.0 (a move of a flagged code run and a code continue-as-new to another
  version refused, with database backstops on `code_run_pins` and `chain_segments`); a child's share taken from the
  group's fixed `reserved_bound`, never from the current policy; the run's place in a group as `capacity_role`, bound at
  insert and never changed, with every member child given a share; the architecture rule requiring `fan_out_open`
  (Decision 039-033, Gap 039-010).
- **2026-11-27, fourth security re-read** (`docs/designs/039-nested-collections.reviews/2026-11-27/reread-4/security.md`):
  the version fence of member runs ordered against design 044 (Task 039-19's `code_run_pins` trigger lands with or
  after Task 044-6, design 044's follow-up Task 044-42 carries it when this design lands first) and owned in the lane
  table, the activation requiring the move and continue-as-new tests; `capacity_role` and `fan_out_share` bound at
  insert, written afterwards only by the segment copy's `SECURITY DEFINER` function, with three checks tying flag, role
  and share; the refusal keyed on a flagged parent that is not `ROOT`; Owner decision 2026-11-27-479 refining
  2026-11-27-454 (Decision 039-034).
