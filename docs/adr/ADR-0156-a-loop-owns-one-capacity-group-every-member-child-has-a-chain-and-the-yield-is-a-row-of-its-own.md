# ADR-0156: A loop owns one capacity group, every member child has a chain, the yield is a row of its own, and the registry requirement is fenced

**Status:** Accepted (2026-11-27, delegate, Owner decision 2026-11-24-4)
**Decided by:** agent proposal with the revision of design 039 after its security and correctness reviews of 2026-11-27,
accepted under the owner's delegation (Owner decision 2026-11-24-4); recorded as Owner decision 2026-11-27-436. The
owner may overrule it.
**Refines / supersedes:** amends ADR-0154 decisions 1, 2, 3, 4a, 5 and 7 and ADR-0078 section 3 (the retiring `Finish`
writes the tenant usage row); supersedes neither.
**Enforced by:** `scripts/check-capacity-activation.py` (the lock sequence in the code's order, the allow-list of
command callers, `NOWAIT` or `SKIP LOCKED` on every group lock); `scripts/check-architecture.py` (every insert into
`workflow_versions` binds `nesting`); the triggers of design 039 Requirements 039-008 and 039-015 with their negative
privilege tests; the possession observer of design 039 Technical Plan 11 in every race test; the invariant check of
Task 039-20. Until they exist: review only.

## Context

The security and correctness reviews of design 039 (`docs/designs/039-nested-collections.reviews/2026-11-27/round-1/`)
found that ADR-0154 as written could not be built or did not hold: a ChildRoot member needs a continuation chain, which
a child without `chainPolicy` or started below the root path never has (`link_child_chain`,
`crates/infra-postgres/src/chain_progress.rs`); a root loop carries one tenant unit across its iterations
(`crates/infra-postgres/src/loop_progress.rs`), so a group per loop iteration would release it twice; a yield flag on
the header competes with the holder's own claims, which keep the header locked, and can strand a holder whose waiting
sibling stops being runnable; the guard runs in the execution engine, not the control plane; the requirement was read
without a lock, which races its removal (ADR-0131, option 6); and the unit had four decrement sites, not one.

## Decision

1. **A loop owns one group.** A root loop of a version published with nesting creates one group at its entry, which
   holds the unit the loop charged; each iteration is a scoped-path member of that group, the next one joining in the
   checkpoint that admits it. A grouped loop keeps its unit across a delay. A root collection keeps one group per root
   iteration.
2. **Every member child has a chain.** A child workflow started inside a member gets an execution chain, of one segment
   when it declares no `chainPolicy` (`max_segments` 1; the deadline its run's, or its creation plus `MAX_RUN_DEADLINE`),
   and `link_child_chain` accepts a parent step at any instance path for such a child. A child with the cancel policy
   `detach` is refused inside a nested body.
3. **The yield is a row of its own.** `capacity_group_yields` holds at most one row per group, inserted with
   `ON CONFLICT DO NOTHING` and no header lock, read by the guard with its `SKIP LOCKED` header read, deleted by
   `Vacate`, `Acquire` and by the sweeper when no other member is runnable any more. The sweeper vacates only a
   quiescent holder whose yield was requested or that has no runnable work, serves finishes before transfers, pages by
   tenant with the scheduler's weights, and skips in `Acquire` a member that was acquired and vacated within the last
   quantum without a lease.
4. **The unit has one owner.** The header records `owns_unit`. For a grouped iteration or loop, none of today's
   decrement sites decrements; the `Finish` or `FinishPath` that retires a group that owns the unit decrements once,
   taking `tenant_iteration_usage` `FOR UPDATE` after the group. This amends ADR-0078 section 3, under which no command
   wrote a quota row. The header is written outside the commands by the creation, by a join (compare and set of
   `revision`, `active_members` increased, under `NOWAIT`) and by nothing else.
5. **The tenant limit bounds groups; a fan-out bound bounds each group.** Publication refuses a nesting version whose
   product of local `maxConcurrency` down the nest exceeds the tenant policy value `nested_group_fan_out` (256 by
   default, at most 4,096), and an active-version child is checked at its admission.
6. **The requirement is fenced and removed in two phases.** The execution engine lists `nested-collections-v1` only when
   its guard, its creator and its sweeper all run; the control plane when its definition service does. Every write of
   new state calls the owner-owned definer function `nested_collections_required()` (`FOR KEY SHARE` on the requirement
   row, form (a) of design 053's Requirement 053-006). Removal drains in two phases (Requirement 053-007): phase 1 stops
   new publications and activations of nesting versions; phase 2 takes the group and member tables `IN SHARE MODE` and
   refuses while a group is not `RETIRED` or a run of a nesting version is not final.
7. **Flags the database enforces.** `runs.capacity_member` is copied to a chain successor and never cleared by trigger,
   and a member insert is refused for an unflagged run; `workflow_versions.nesting` is derived from the parsed document
   by one function every insert path calls.
8. **Bounded evidence.** Receipts are kept 15 minutes plus the latest 16 per group; transfers are summarised by one
   audit record per group and hour; a retired group is deleted with its root run only through the owner-role function
   `capacity_purge_retired`, which feature 020's purge calls; `sagat_app` holds no `DELETE` on headers or members.

## Considered options

1. **The decisions above** — each answers a finding verified in the code.
2. **ChildRoot members keyed by run instead of chain** — rewrites the built commands, their unique index and the guard;
   a one-segment chain reuses them.
3. **A group per loop iteration** — the carried unit would be released twice, or a fresh unit asked for inside the
   loop would deadlock at limit 1.
4. **The yield flag on the header** — almost always `Busy` under the holder's claims, and cleared only by a transfer.
5. **A per-tenant active-instance ceiling** — blocks admitted nested work at its ceiling, which ADR-0030 §14 forbids.

## Consequences

A grouped loop holds its unit through its delays, so a tenant at limit 1 with a delayed nesting loop admits nothing
else while it waits; that is the price of not releasing a unit twice. Compensation of a member that is already
finished runs outside possession and outside the unit, as a root collection's scope compensation does today. A
nesting version whose fan-out exceeds the policy is refused at publication, not throttled at run time. The deletion
of retired groups waits for feature 020's purge (design 039, Gap 039-008).

## Evidence

Design 039, Technical Plans 2 to 9 and 11 (`docs/designs/039-nested-collections.md`); research Decisions 039-012 to
039-018 (`specs/039-nested-collections/research.md`); the reports cited above.

## Revision of 2026-11-27 (design 039, EGM design critique)

Recorded under Owner decision 2026-11-24-4 after `docs/designs/039-nested-collections.reviews/2026-11-27/round-1/egm-design.md`
(Owner decisions 2026-11-27-449 to 2026-11-27-456).

- **Decision 1, amended.** A root loop has one scoped-path member for the whole loop, keyed by (run, the loop's owner
  node) with the fixed index 0, holding possession across its iterations and delays; the guard gates loop work on
  (run, root node). The member is finished, and the group retired, only when the loop scope's lifecycle is final
  (`SUCCEEDED`, `FAILED` or `CANCELLED`), never while it is `RUNNING`, `DELAYING` or `DRAINING`.
- **Decision 3, extended.** A join that finds the header busy records a join wait (`capacity_group_join_waits`); a wait
  a quantum old makes the sweeper insert the yield row. The yield row carries the header's epoch, and the guard and
  the sweeper act only on a row of the current epoch. The guard sets the holder's `last_leased_at` when it grants a
  lease, under the `SKIP LOCKED` header lock it holds; `Acquire` sets `acquired_at`; `Vacate` sets
  `last_vacated_unleased_at` when `last_leased_at` is older than `acquired_at`.
- **Decision 4, extended: the write set of the built commands.** ADR-0078 section 3 and feature 014's Decision 710
  limit `Vacate` and `Acquire` to the header, one audit row, one outbox row and one receipt. They now write the header
  (with `transfer_count`, `held_since`), the member row (`acquired_at` or `last_vacated_unleased_at`), the deletion of
  the yield row, one outbox row and one receipt, and no audit row; `capacity_group_transfer_receipts.audit_id` becomes
  nullable, and the `capacity.group.retired` record carries the transfer count. No hourly audit table.
- **Decision 5, extended.** Every version stores its fan-out bound at insert (`fan_out_bound`, computed by
  `fan_out_of`); a child's share is the policy value divided by the product of `maxConcurrency` down to its call;
  `nested_group_fan_out` is a column of `tenant_collection_policies`, written through its audited path. Whether an
  admission is a descendant depends only on the run's `capacity_member` flag, whatever its version.
- **Decision 6, amended: the drain rule.** `nested_collections_required(mode)`: `new_use` (a publication, an activation,
  the admission of a new run of a nesting version) answers yes only when the requirement is `active`; `continue` (the
  creator's group or member for a run already started) answers yes when it is `active` or `draining`; the guard grants
  member leases while it is `active` or `draining`. Phase 1 therefore stops new nesting work without halting the runs
  phase 2 waits for. Design 053 knows only the plain form (a); its follow-up Task 053-9 registers this "continues while
  draining" descriptor kind. The header gains `namespace_id` and `root_run_id`, its shard derived by `Shard::of`.
- **Decision 9, added: a version fence on the lease.** A trigger on `steps` refuses a step of a run whose
  `capacity_member` flag is set moving to `LEASED` unless the transaction set `sagat.capacity_guard` to `v1`, which only
  the guard sets, so a binary that does not run the guard (an older one not yet counted by the registry) cannot lease
  member work. It is a version fence, not a boundary against the platform's own role.

Possession is kept over joining without it, bounded only by the fan-out bound, because Owner question 706 and ADR-0075
keep the tenant limit on leased work; its visible cost is that the members of one group lease one at a time.

## Revision of 2026-11-27 (implementation-readiness review)

Recorded without changing the decisions above, under Owner decision 2026-11-24-4 (Owner decisions 2026-11-27-457 and
2026-11-27-458; design 039, Decisions 039-023 to 039-025). The yield row (decision 3) and the join wait are keyed by
(tenant, group) and carry no foreign key to `capacity_groups`, whose check would wait behind a holder's claim. The
deletion of retired groups is allowed only when made by the tables' owner, which is how `capacity_purge_retired`
runs; it deletes receipts, side rows, members, then headers. `FinishPath` writes `capacity_group_finish_receipts`
with kind `SCOPED_PATH`; `capacity_path_transfer_receipts` holds only `VACATE` and `ACQUIRE`. The member's
`last_leased_at` is written once by the claim transaction after the first lease grant of that member is persisted,
under the group lock it holds, last in the lock sequence. A child started at a non-root instance path is a member
child like any other (Task 039-25).

## Revision of 2026-11-27 (security re-read)

Recorded after the security re-read of design 039 (`docs/designs/039-nested-collections.reviews/2026-11-27/reread/security.md`),
under Owner decision 2026-11-24-4 (Owner decisions 2026-11-27-459 to 2026-11-27-463). The clauses above that this
revision supersedes stay as written; where they differ, this revision rules.

- **Decision 3.** "Inserted with `ON CONFLICT DO NOTHING`" is superseded: a yield row of an earlier epoch is replaced
  (`ON CONFLICT (tenant_id, group_id) DO UPDATE ... WHERE epoch < excluded.epoch`); both side tables are inserted with
  `INSERT ... SELECT ... WHERE EXISTS` a non-retired header of the same tenant and namespace; a join wait names its
  waiting parent step and carries `last_busy_at`, and only a live wait (its parent step pending, its run neither paused
  nor stopping, refreshed within two ticks) keeps a yield; the retiring `Finish` deletes its group's side rows.
- **Decision 5.** The bound is recursive over pinned children and sums the branches of a `parallel` node; a member
  child run carries `fan_out_share`, against which its own children are checked; the policy is tenant-scoped, its
  default ceiling 256 and its maximum 4,096 under the operator's ceiling; every root admission rechecks the version's
  bound against the effective policy; each group reserves its bound in `tenant_iteration_usage.nested_reserved`, under
  the operator ceiling `nested_live_iterations` (16,000 by default, at most 20,000), so one tenant's nested work stays
  under the 20,000 pending waits per replica of Owner decision 2026-11-04-6.
- **Decision 6.** `nested_collections_required()` takes a mode (`new_use`, `continue`; any other raises), and the
  insert triggers of the group and member tables call it in mode `continue`, so no writer can skip it. Phase 2 takes no
  table lock: its `FOR UPDATE` of the requirement row waits for every fence caller.
- **Decision 7.** The successor copy is an `AFTER INSERT` trigger on `chain_segments`, not a trigger on the run insert
  (`runs` has no chain column); the relaxed slot check admits `ADMITTED AND NOT slot_held` only for a flagged run; the
  header's `namespace_id` comes from its root run and `owns_unit` only when the root iteration or loop holds its slot;
  the lease fence refuses only a new grant and compares its setting with the running transaction's id.
- **Decision 8.** "Transfers are summarised by one audit record per group and hour" is superseded by the readiness
  revision above (no hourly table; `transfer_count` on `capacity.group.retired`). `capacity_purge_retired` is revoked
  from `PUBLIC`, executable by its owner only, and requires feature 020's purge authorization for its transaction and
  run.

## Revision of 2026-11-27 (second security re-read)

Recorded under Owner decision 2026-11-24-4 after `docs/designs/039-nested-collections.reviews/2026-11-27/reread-2/security.md`
(design 039, Decision 039-032). Unit ownership is checked against the slot the group names: the header carries its
root owner node and iteration index, set at insert and never changed, and one group at most owns the unit of one slot
(a partial unique index over the groups not retired that own a unit). A code version (design 044) whose envelope lists
children is refused as a member child, because its fan-out is not declared. The decisions above are otherwise unchanged.

## Revision of 2026-11-27 (third security re-read)

Recorded under Owner decision 2026-11-24-4 after `docs/designs/039-nested-collections.reviews/2026-11-27/reread-3/security.md`
(design 039, Decision 039-033). A run of a capacity group keeps its version in release 0.3.0: a move of a code run and
a continue-as-new to another version are refused for a flagged run, with triggers on `code_run_pins` and
`chain_segments` as the database's backstop (Gap 039-010 for a later, bounded move). A child's share is taken from its
group's fixed `reserved_bound`, never from the policy at the child's admission. A run's place in a group is
`runs.capacity_role` (`ROOT` or `MEMBER_CHILD`), bound at insert and never changed. The decisions above are otherwise
unchanged.

## Revision of 2026-11-27 (design 039, fourth security re-read)

Recorded under Owner decision 2026-11-24-4 (Owner decision 2026-11-27-479, Decision 039-034). The segment copy that
sets a successor's `capacity_member`, `fan_out_share` and `capacity_role` is a `SECURITY DEFINER` function owned by the
table's owner, with `search_path` pinned to `pg_catalog, pg_temp` and every relation schema-qualified; `sagat_app` holds
no `UPDATE` on `fan_out_share` or `capacity_role`, which are bound at the run's insert, and three checks on `runs` tie the
flag, the role and the share. A member run keeps its version in release 0.3.0; design 044's follow-up Task 044-42
carries the `code_run_pins` trigger, the skip reason and the completion refusal when this design lands first.
