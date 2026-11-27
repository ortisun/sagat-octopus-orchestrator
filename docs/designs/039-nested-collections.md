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
  group) or a **scoped-path** member (one iteration of a root collection or of a root loop, by run, root node and
  iteration index, whose body nests work).
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
- **Requirement 039-002** A root admission of a version published with nesting (Technical Plan 8) MUST create a group
  with its first member in the admission's transaction: for a root collection, one group per root iteration; for a root
  loop, one group per loop, created at the loop's entry, with a scoped-path member per iteration (Technical Plan 2).
  Every admission made inside an ACTIVE member (a collection or loop iteration whose owning path lies within a
  scoped-path member's iteration, or any admission in the run of a ChildRoot member, a child's own collections included)
  is a descendant: it MUST join the group of its ancestor and MUST NOT reach the fresh-admission check or charge
  `tenant_iteration_usage` (Decision 705, ADR-0030 §14). Every child workflow started inside a member MUST get an
  execution chain, of one segment when it declares no `chainPolicy`, so that its membership is bound to a chain.
  Joining is bounded by the lifetime `maxInstances` budget, by the local scope slots and by the fan-out bound. An
  admission outside every member, in a version published without nesting (a top-level child workflow included), creates
  no group and behaves as today.
- **Requirement 039-003** Only the holder MAY be granted worker leases (ADR-0075 as built, Owner question 706), for a
  ChildRoot member's run and for the steps of a scoped-path member's iteration alike: both lease producers check
  possession by (run) for a ChildRoot member and by (run, root node, iteration index) for a scoped-path member, with the
  instance path given to the guard, and a step of the root run outside every member is not gated. A process that the
  registry does not count, or whose serving gate does not use `nested-collections-v1`, MUST grant no lease to a member's
  step. Reports, heartbeats, timers, signals, retries, control nodes and child outcomes MUST NOT be gated by possession.
  Compensation steps under the path of a member that is already finished run outside possession (Technical Plan 4).
- **Requirement 039-004** Possession MUST move only by the fenced commands of ADR-0078 for ChildRoot members and by
  their scoped-path twins (Technical Plan 4), called only by the sweeper, each in its own transaction, holding no other
  lock or transaction of the caller. The header is written outside those commands only by the group's creation, by a
  join (compare and set of `revision`, `active_members` increased, under `NOWAIT`) and by nothing else; the retiring
  `Finish` releases the unit (Requirement 039-016).
- **Requirement 039-005** The sweeper MUST serve every member state that can occur: a holder or non-holder whose head
  or iteration is final, whatever its outcome (succeeded, failed, cancelled, compensated, timed out, partial or
  compensation failed), with the last finish retiring the group (finish); a holder past its quantum while another
  member has runnable work (yield request); a yield request whose waiting members are no longer runnable (cleared); a
  quiescent holder whose yield was requested, or a holder with no runnable work while another member has runnable work
  (vacate); a vacant group with a member that has runnable work (acquire). A scoped-path member finishes by `FinishPath`
  (Technical Plan 4), which needs no certified success and serves a body that nests.
- **Requirement 039-006** No member MAY wait without bound while others run: when another member has runnable work and
  the holder has held possession for at least the quantum (1 second by default, from 500 milliseconds, the sweeper's
  tick, to 60 seconds), the sweeper inserts a yield request, after which the guard grants the holder no new lease; once
  its leased and running steps end it is quiescent and is vacated, and `Acquire` chooses the member whose oldest
  runnable step became ready first, skipping a member that was acquired and then vacated within the last quantum without
  being granted a lease. The yield gates new leases only, never a report or a running step (Owner question 706).
- **Requirement 039-007** Candidate selection and the sweeper MUST use one shared runnable predicate, written as the
  candidate query of `crates/infra-postgres/src/workers.rs` has it (the non-FORWARD steps of a stopping run, its
  compensations, are runnable; a paused run's steps are not), and held equal to that query by a differential test that
  runs both on a generated corpus (Gap 715); the guarded poll MUST rotate its sample of ineligible runs (Gap 724) and
  MUST NOT count runs the claim refuses toward its eligible window (Gap 725).
- **Requirement 039-008** One ACTIVE ChildRoot membership per chain MUST be enforced by the unique index of ADR-0078
  section 5 and ADR-0075 condition (k), and one ACTIVE scoped-path membership per root iteration by a new partial unique
  index on (tenant, namespace, `original_run_id`, `root_owner_node`, `iteration_index`) for ACTIVE `SCOPED_PATH` rows;
  the existing `capacity_group_members_scoped_path_unique` includes `group_id` and covers every state, so it cannot. An
  insert trigger MUST force a new header to `HELD` with `revision`, `epoch` and `active_members` 1 and its `shard`
  derived from the root run, a new member to `ACTIVE` with a null `finished_at`, and MUST refuse a member of a `RETIRED`
  group.
- **Requirement 039-009** Transfer receipts MUST be bounded: per group, the latest 16 and every receipt younger than 15
  minutes are kept (the horizon in which a command's commit can still be uncertain to its caller, Decision 713); older
  ones are purged in batches of at most 1,000 by a bounded pass. A retired group's header, members and remaining
  receipts are deleted only with its root run, by the owner-role function `capacity_purge_retired` that feature 020's
  run purge calls in the transaction that purges that run (Gap 039-008); the runtime role holds no `DELETE` on the
  header or member tables. Membership creation, finish and retirement are audited one record each; transfers are
  counted by metrics and by the header's `transfer_count`, and summarised by one audit record per group and hour.
- **Requirement 039-010** The worker's complete lock sequence MUST be written in
  `specs/039-nested-collections/contracts/worker-lock-sequence.json`, in the order the code takes the locks (Technical
  Plan 6), and checked against ordered markers in the code by `scripts/check-capacity-activation.py`, which MUST also
  refuse any `capacity_groups` row lock without `NOWAIT` or `SKIP LOCKED` (ADR-0075 condition (a)).
- **Requirement 039-011** Activation MUST follow the gate of ADR-0075 and the rules of design 053 (Requirements 053-006
  and 053-007) and ADR-0131: every condition (a) to (l) holds, then the operator records the requirement
  `nested-collections-v1`, which the registry refuses while a live engine or control plane does not list it. An
  execution engine lists it only when its worker guard (both lease producers), its creator and its `capacity_sweep`
  pass all run with `SAGAT_NESTED_COLLECTIONS=on`; a control plane lists it when its definition service runs with the
  switch on. Every write of new state, the creator's insert of a group or a member, a publication and an activation of a
  nesting version, MUST first call the owner-owned definer function `nested_collections_required()`, which takes the
  capability's requirement row `FOR KEY SHARE` and answers whether it is `active` (form (a) of Requirement 053-006); when
  it answers no, the creator refuses with `NESTED_COLLECTIONS_INACTIVE` and the admission waits like a busy join
  (Technical Plan 2). Removal takes the two phases of Requirement 053-007 (Technical Plan 9); rolling a binary back below
  the recorded requirement is unsupported.
- **Requirement 039-012** Crash, duplicate, stale fence, lost broker hint, cancellation, unknown or failed compensation
  and unknown COMMIT MUST be proven for nested scopes, in both race orders, with held locks or barriers, and with the
  possession observer of Technical Plan 11 checked at the end of every race test.
- **Requirement 039-013** Nested scopes MUST be offered in all six SDKs through the collection builders that exist, with
  conformance scenarios that build, publish and run them.
- **Requirement 039-014** Publication MUST refuse a nesting version whose fan-out bound exceeds the tenant's policy
  value `nested_group_fan_out` (256 by default, at most 4,096), with `NESTED_FAN_OUT_LIMIT`: the bound is the product of
  the local `maxConcurrency` of every collection on a path from a root iteration down to its deepest level, a loop
  counting 1, a pinned child counting its version's recorded bound, and an active-version child checked at its
  admission against the bound left to it (the child admission is refused with the same code). So a tenant at limit L
  runs at most L groups and at most L × `nested_group_fan_out` iterations at once.
- **Requirement 039-015** The run row's `capacity_member` flag MUST be enforced by the database: it is `NOT NULL`; a run
  inserted as a successor of a chain copies the chain head's flag (a trigger, under the chain lock the successor path
  already takes); a member insert is refused unless its run, or its chain's head, is flagged; the flag is never cleared
  (a trigger refuses the update).
- **Requirement 039-016** The tenant unit MUST be released exactly once. A group created by a root admission records
  `owns_unit = true`; for a grouped iteration or loop none of today's decrement sites decrements (the collection
  checkpoint, the collection drain, the loop checkpoint at the loop's end or at a delay, the loop drain), and a grouped
  loop's delay neither releases nor charges again; the `Finish` that retires the group decrements once, under the guard
  `active_iterations > 0`, in its transaction. The invariant `active_iterations = slot-held iterations without a group
  + groups not RETIRED whose owns_unit is true` MUST hold after every checkpoint, drain and cancellation.

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
  commands (`Vacate`, `Acquire`, `Finish`, `FinishPath` and their scoped twins) per tick per replica (configurable to
  512), and at most 64 yield inserts per tick on a budget of their own. At the 500-millisecond tick that is 128 capped
  commands per second. A contended group uses two capped commands per transfer and at most one transfer per quantum, so
  at the 1-second quantum a replica serves at most 64 contended groups' transfers per second when it serves no finish;
  each finish served in the same second removes half a group from that figure. The report of Task 039-15 states the
  number of contended groups per replica.

### Success criteria

- **Success criterion 039-001** At a tenant limit of 1, a root iteration that waits on an inner collection of 10
  iterations, each of which starts a child workflow (with and without `chainPolicy`), completes, and so do a child
  workflow that contains a collection and a root `while` loop of three iterations each containing a collection, with
  `active_iterations` asserted after every checkpoint; the tenant never has two members of different groups running at
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
  orders and calls a child workflow per order, and it runs with my tenant's limit.
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
2026-11-26-149, 2026-11-27-390 to 2026-11-27-395 and 2026-11-27-436 to 2026-11-27-447):

| Item | Decision | Why |
|---|---|---|
| Accept ADR-0078 (Owner decision 2026-11-26-140) | Accepted as written | Its third architecture and PostgreSQL review found no Blocker or Major; the open items are decided by ADR-0154, not by editing it |
| The creator (Owner decision 2026-11-26-141) | The engine creates groups and members inside the transactions that admit the work; no API | Membership is a consequence of admission, never a user act; an API would add a way to put a chain into a group it does not belong to |
| Sweeper coverage (Owner decision 2026-11-26-142, refined by Owner decisions 2026-11-27-394 and 2026-11-27-443) | Finishes first, then the yield request, the clearing of a stale yield, the vacate of a yielded or idle holder, the acquire | Without the finishes a group's unit counts against admission for ever; without the yield a busy holder is never quiescent; without the clearing a yield can strand a lone holder |
| Fairness (Owner decision 2026-11-26-143, refined by Owner decision 2026-11-27-443) | A quantum of 1 second, oldest-ready-first `Acquire` that skips a member acquired and vacated without a lease, tenant-weighted paging | Without it a busy holder starves its siblings (Gap 718) and a noisy tenant delays another's finishes |
| Receipt retention (Owner decision 2026-11-26-144, refined by Owner decision 2026-11-27-438) | Latest 16 per group and 15 minutes; transfers summarised by one audit record per group and hour | Receipts are idempotency keys for a retained command, needed while a commit can be uncertain; a permanent audit row per transfer grows at a rate the author controls |
| Scoped paths (Owner decision 2026-11-26-145) | Commands and a receipt table of their own, as Decision 706 decided, quiescence over the member's iteration path | A forEach in a forEach is a scoped path; without them the commonest nesting cannot run |
| The worker's lock sequence (Owner decision 2026-11-26-146) | Written as a checked contract file, in the order the code takes | A list in prose drifts; a checked list fails the build when the code changes the order |
| The guarded poll (Owner decision 2026-11-26-147) | Rotate the ineligible sample by a keyset cursor and count only claimable runs | Closes Gaps 724 and 725, which block activation |
| The first full-stack run (Owner decision 2026-11-26-148) | A dedicated full-stack nested run on the integrated branch before activation; the release-candidate run repeats it | The release event must not be the first time the composition runs |
| The delegated rows of 2026-11-04 on nesting (Owner decision 2026-11-26-149) | OD-2026-11-04-3 to OD-2026-11-04-8 confirmed as they stand for release 0.3.0 | They were taken under the earlier delegation and nothing found since contradicts them |
| ADR-0156 (Owner decision 2026-11-27-436) | Accepted: a loop owns one group, every member child has a chain, the yield is a row of its own, the unit has one owner, the fence of design 053 | The reviews of 2026-11-27 found the first answers unbuildable or unsafe |
| The fan-out bound (Owner decision 2026-11-27-437) | `nested_group_fan_out`, 256 by default, at most 4,096 | Without it a tenant at limit 1 runs thousands of inner iterations |
| Loop groups (Owner decision 2026-11-27-439) | One group per root loop, keeping its unit across delays | A loop carries its unit between iterations; a group per iteration would release it twice |
| Chains for member children (Owner decision 2026-11-27-440) | A one-segment chain for a child without `chainPolicy` | ChildRoot membership is bound to a chain; keying by run would rewrite the built commands |
| Detach (Owner decision 2026-11-27-441) | Refused inside a nested body | A detached child would keep its group's unit for as long as it runs |
| The fence and removal (Owner decision 2026-11-27-442) | Form (a) of design 053, two-phase removal, listing on the engine | The guard lives in the engine; an unlocked read of the requirement races the removal |
| Unit ownership, compensation, flags and purge (Owner decisions 2026-11-27-444 to 2026-11-27-447) | `owns_unit` with one decrement; compensation after retirement outside possession; database-enforced `capacity_member` and derived `nesting`; `capacity_purge_retired` | Each closes a path where the unit is released twice, never, or by a writer that forgot a flag |

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
worker guard (`worker_capacity_available`, `crates/infra-postgres/src/execution/worker_capacity.rs`, behind
`child_root_capacity_guard`); `scripts/check-capacity-activation.py` and the step-status-writer inventory
(`specs/014-workflow-primitives/contracts/step-status-writers.json`).

### 2. The creator and descendant admission

- **Only versions published with nesting.** `workflow_versions.nesting` is derived from the parsed document by one
  function (`nesting_of(&Definition)`) that every path inserting a version calls, the publication path and the ad hoc
  path of `crates/infra-postgres/src/execution/ad_hoc.rs` alike; `scripts/check-architecture.py` refuses an
  `INSERT INTO workflow_versions` that does not bind it. The creator acts only for such versions; a version published
  without nesting, or a top-level child workflow outside every member, creates no group and charges what it charges
  today.
- **The fence.** Before it inserts a group or a member, the creator calls `nested_collections_required()` in the
  admitting transaction (Requirement 039-011). An answer of no refuses with `NESTED_COLLECTIONS_INACTIVE`: the
  admission rolls back and the step or iteration is retried by the next tick with no attempt consumed, counted
  (`capacity_member_join_total{outcome="inactive"}`), as a busy join is.
- **Group and first member of a root collection iteration.** When a root collection admission of a nesting version
  takes the tenant unit (`retained < limit`), the same transaction inserts the group (`HELD`, holder the new scoped-path
  member, `held_since` now, `owns_unit` true) and the member. The group row is new, so no lock on it is waited for. That
  initial `HELD` is the exception to ADR-0078 section 3's rule that possession moves only by the commands, recorded in
  ADR-0154.
- **Group of a root loop.** A root loop of a nesting version takes its unit at entry, as today (`enter_loop`), and the
  same transaction inserts the group (`owns_unit` true) with the first iteration's scoped-path member as holder. Each
  later iteration's member joins the same group in the checkpoint transaction that admits that iteration; the finished
  iteration's member is finished by `FinishPath` on the sweeper's next tick. A grouped loop keeps its unit across a
  delay: `checkpoint` does not decrement at a delay and `fire_delay` does not charge again. The group retires when the
  loop has ended or drained and every member is final.
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
  `revision` and increases `active_members`, and inserts the ChildRoot member `ACTIVE`. A `Busy` answer rolls the
  creation back and the step that creates the child is retried by the next tick, with no attempt consumed, counted
  (`capacity_member_join_total{outcome="busy"}`); nothing waits on the group.
- **The tenant unit is released when the group retires** (Requirement 039-016). The four decrement sites of today,
  `primitives.rs` at the collection checkpoint and the collection drain and `loop_progress.rs` at the loop checkpoint
  (its end or a delay) and the loop drain, skip the decrement for an iteration or loop whose group owns the unit; the
  `Finish` (or `FinishPath`) that retires a group with `owns_unit` true decrements once, takes `tenant_iteration_usage`
  `FOR UPDATE` after the group (an amendment of ADR-0078 section 3, in ADR-0156), and refuses with an integrity error
  when the counter is already 0. A cancelled or failed root iteration whose ChildRoot members still compensate keeps its
  unit until they are final.
- **Flags.** `runs.capacity_member` is `NOT NULL DEFAULT false`, set by the creator for every member run and copied to
  a chain successor by a trigger on the run insert of a successor (under the chain lock that path already takes); a
  member insert is refused unless its run or its chain head is flagged; an update that clears the flag is refused.
- **Grants.** The migration grants `sagat_app` column-scoped `INSERT` on the header and member tables (ADR-0075
  condition (l)), with the insert trigger of Requirement 039-008, and adds the unique indexes of Requirement 039-008
  (condition (k)); no `DELETE` on either table.
- **Audit and telemetry.** `capacity.member.created`, `capacity.member.finished` and `capacity.group.retired`, values of
  the closed audit action vocabulary with their test and their rows in design 040's mapping table once it is built; and
  `capacity_members_created_total{kind}`.

### 3. The sweeper

A pass `capacity_sweep` on the scheduler tick, with no transaction open while it calls a command (ADR-0078 section 4).
A group is served by one replica: the one holding the shard of the group's root run, recorded on the header at
creation as `shard` by the insert trigger from the root run (a run's shard never changes); every command the sweeper
issues for the group reads that shard's fence `FOR SHARE` (the leaf lock of ADR-0050), and Task 039-7 makes the built
ChildRoot commands read the group's shard rather than the shard of the member run they name, with a test. Each tick it
pages, per held shard, with a keyset cursor, at most 256 candidates, taking candidates per tenant in round robin
weighted by the scheduler's tenant weights (`tenant_weights`, `crates/scheduler/src/shards.rs`), and serves the
selections in this order:

1. `HELD` groups whose holder's head or iteration is final → `Finish` (or `FinishPath`) the holder;
2. ACTIVE non-holder members whose head or iteration is final, in `HELD` or `VACANT` groups → `Finish` that member, the
   last one retiring the group;
3. `HELD` groups with no yield request whose holder has held possession for at least the quantum while another member
   has runnable work → insert a row of `capacity_group_yields (tenant_id, namespace_id, group_id, epoch,
   requested_at)` with `ON CONFLICT DO NOTHING`; no header lock, no receipt, possession does not move;
4. `HELD` groups with a yield request none of whose other members has runnable work any more → delete the yield row
   (the holder leases again);
5. `HELD` groups whose holder is quiescent (no leased or running step of its work) and either has a yield request or
   has no runnable work, while another member has runnable work → `Vacate` (which deletes the yield row);
6. `VACANT` groups with an ACTIVE member that has runnable work (the shared predicate) → `Acquire` for the member whose
   oldest runnable step became ready first, skipping a member whose `last_vacated_unleased_at` is within the last
   quantum (set by a `Vacate` of a member that was granted no lease since its `Acquire`); `Acquire` sets `held_since`.

A holder alone in its group is never vacated while it has runnable work, so a single-member group does not churn
between its steps; a yield request whose waiting member stopped being runnable (paused, timed out, finished) is
cleared by selection 4, so a lone holder is never stranded. At most 64 capped commands per tick (selections 1, 2, 5 and
6) and 64 yield inserts and deletes on their own budget; candidates beyond the page and commands beyond the caps are
deferred to the next tick and counted (`capacity_sweep_deferred_total{reason}`, `reason` in `candidates`, `commands`,
`yields`). The operation key is derived from the whole tuple each command fingerprints (ADR-0078 section 4); a
`FingerprintConflict` means "rediscover".

**Timing.** One transfer costs at most Q + S + 3T: up to one tick to see the quantum, the longest step S running when
the yield was requested, one tick to see quiescence and one tick to `Acquire`. With three members, the last one served
waits at most 2Q + S_H + S_A + 6T from the instant it became runnable; at Q = 1 s and T = 0.5 s that is 5 seconds plus
the two longest steps.

### 4. Scoped-path commands and the scoped guard

`VacatePath`, `AcquirePath` and `FinishPath` serve scoped-path members with a receipt table of their own
(`capacity_path_transfer_receipts`), the same compare and set of revision and epoch, and the lock order of every
capacity command (ADR-0030 §18, ADR-0078 rule 1): the member's chain when it has one, then the run that owns the path,
then the group `NOWAIT`, then the shard `FOR SHARE`, then, for a retiring finish of a group that owns the unit,
`tenant_iteration_usage` `FOR UPDATE`, written in the checked contract of Technical Plan 6. Quiescence is over the
member's iteration path: no leased or running step whose path starts with the member's root node and iteration index.

`FinishPath` finishes a scoped-path member whose iteration is final, whatever its outcome, and whatever its body
contains: its proof is read from the source facts in its transaction (the iteration's lifecycle is `CHECKPOINTED` or
`DRAINED`, in `collection_iterations` or `loop_iterations`; no step under the path prefix is non-final; and every
ChildRoot member admitted under that iteration is final). It needs no certified success, so it is not the scoped leaf
finish of feature 014 (`crates/application/src/leaf_certification.rs`, `crates/infra-postgres/src/scoped_leaf_finish.rs`),
which stays for a depth-1 ordinary leaf; a test per terminal outcome (succeeded, failed, cancelled, compensated, timed
out, partial, compensation failed) and per body shape (leaf, nested collection, loop, child).

**The scoped guard.** Today the guard refuses every claim on a run that has a `SCOPED_PATH` member (`UnsupportedScope`
in `worker_capacity.rs`), and ADR-0075 requires scoped paths to fail closed. With the requirement recorded, both lease
producers (`crates/application/src/worker.rs` and `crates/application/src/worker/scoped_claims.rs`) pass the instance
path to the guard, which grants a lease on a step whose path lies within a scoped-path member's iteration only when
that member holds its group and no yield row exists for the group, keyed by (run, root node, iteration index); a step
of the root run outside every member is not gated, so a nested admission never halts the root run's top-level steps.
The shared predicate of Technical Plan 5 gains the same scoped clause. Without the requirement the refusal stays.
ADR-0154 amends ADR-0075's scoped-path bullet to this rule; race tests cover a holder and a non-holder iteration
claiming at once, and a claim racing `VacatePath`.

**Compensation after retirement.** A scope's compensation is planned only once every iteration is checkpointed or
drained (`plan_scope_compensation`, `crates/scheduler/src/passes.rs`), when the groups of the successful iterations may
already have retired and released their units. A compensation step under the path of a member that is finished, or of
a retired group, is leased outside possession and outside the unit, as a root collection's scope compensation is leased
today after its iterations released their slots. The tenant limit bounds forward work; compensation is bounded by the
steps it compensates. A limit-1 test compensates a successful iteration with a nested body after a later iteration
fails.

### 5. The runnable predicate and the guarded poll

One SQL fragment, `runnable_member_work`, in `crates/infra-postgres`, used by candidate selection and by the sweeper.
It is written from the candidate query of `crates/infra-postgres/src/workers.rs` as that query stands: a stopping run's
non-FORWARD steps (its compensations) are runnable, a paused run's steps are not. The differential test runs the
fragment and the real candidate query (with a session that serves every activity) on a generated corpus that includes
paused, stopping and compensating runs, and asserts the same answer. The sweeper has no worker session, so the
predicate cannot know whether a worker serves an activity, a quota or a breaker allows it; the skip of Technical Plan 3
selection 6 keeps such a member from taking possession back every tick. The guarded poll samples ineligible runs by a
rotating keyset cursor instead of always the four oldest (Gap 724) and counts toward its eligible window only runs whose
claim the guard would accept (Gap 725). A run without membership takes the path of today unchanged (Non-functional
requirement 039-002). Between the guard's unlocked header read and its `SKIP LOCKED` read the holder may change (Gap
726): the second read rechecks the holder, the revision and the yield row, and refuses on any difference; the cost of
the statement's generic plan is measured by Task 039-13.

### 6. The worker's lock sequence, checked

`specs/039-nested-collections/contracts/worker-lock-sequence.json` lists the locks of a guarded claim in the order the
code takes them (`crates/application/src/worker/scoped_claims.rs` and `crates/infra-postgres/src/execution.rs`,
`lock_chain_for_run` and `lock_path_revision`): the chain, the run, the drain advisory lock, the binding advisory lock,
the worker session, the secret advisory locks, the group `SKIP LOCKED` after an unlocked header read, once per instance,
the activity quota locks, the breaker, the database and script locks; each with the function that takes it.
`scripts/check-capacity-activation.py` reads it and checks that the named functions take them in that order (ordered
source markers), and that no `capacity_groups ... FOR` lacks `NOWAIT` or `SKIP LOCKED`. The file also lists the lock
sequences of the creator (Technical Plan 2: parent run, tree-admission lock, new chain and run, group `NOWAIT`), of the
join, and of every capacity command (Technical Plan 4, with the retiring finish's `tenant_iteration_usage` after the
group). The check holds an allow-list of the callers of the commands (the `capacity_sweep` pass only; ADR-0078
"Enforced by"), replacing today's ban of every production call, and the check that ADR-0075 condition (g) names. A
real-PostgreSQL test per writer of a group or member row runs it with the group held on another connection and expects
`Busy` or a skip, never a wait.

### 7. Receipts and retention

A bounded pass deletes, per group, receipts older than 15 minutes that are not among the latest 16, in batches of
1,000. The receipt tables carry `ENABLE ALWAYS` append-only triggers that refuse `DELETE`
(`20261226000000_capacity_transfer_receipts`, `20261210000000_capacity_finish_storage`); the migration replaces them by
triggers that allow a `DELETE` only of a receipt that the retention rule releases (older than 15 minutes and not among
its group's latest 16, checked by the trigger itself, `SECURITY INVOKER`, `search_path` pinned to `pg_catalog, pg_temp`)
or of a receipt of a group whose root run row no longer exists, which `sagat_app` cannot cause since it cannot delete a
run; and grants `sagat_app` `DELETE` on the receipt tables only. The header and member guards
(`capacity_groups_guard`, `capacity_group_members_guard`) are replaced so that a `DELETE` is allowed only of a `RETIRED`
group and its members, and only inside the owner-role definer function `capacity_purge_retired(tenant, namespace,
root_run)` (`search_path` pinned to `pg_catalog, pg_temp`, executable by the role feature 020's purge runs as), which
deletes the retired groups of that root run with their members and receipts; a negative privilege test deletes a kept
receipt and a retired group as `sagat_app` and is refused. The down migration refuses while any group or member exists.

Transfers are not audited one record each: the header gains `transfer_count`, increased by each `Vacate` and
`Acquire`, and a durable row `capacity_transfer_audit (tenant_id, namespace_id, group_id, hour, transfers)` is
upserted by each transfer; the receipt pass writes one audit record `capacity.group.transfers` per closed hour with
its count and deletes the row.

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

**Deactivation.** Two phases (Requirement 053-007). Phase 1 marks the capability draining: `nested_collections_required()`
then answers no to a publication or an activation of a nesting version, while runs of already active nesting versions
still create groups. Phase 2 takes `capacity_groups` and `capacity_group_members` `IN SHARE MODE`, runs the drain check
(no group is not `RETIRED`, and no run of a nesting version is not final), refuses on any, and removes the row; the
descriptor of the capability names fence form (a), the two tables and that check. A binary rollback below the recorded
requirement is unsupported (an older binary cannot parse a stored nested version).

### 10. Telemetry and operability

`capacity_sweep_commands_total{operation, outcome}`, `capacity_group_held_seconds` (a histogram of the age of `HELD`),
`capacity_member_wait_seconds` (ready to acquired), `capacity_member_join_total{outcome}` (`created`, `busy`,
`inactive`), `capacity_sweep_deferred_total{reason}`, `capacity_sweep_backlog` (candidates left after a page),
`capacity_groups_stalled{state}` (groups not `RETIRED` with no transfer, finish or member change for 10 minutes, by
state), `capacity_members_unfinished` (members whose head or iteration is final but which are still `ACTIVE` after 10
ticks), `worker_claim_skipped_total{reason}` (existing, gaining `yield_requested` and `scoped_not_holder`), the refusal
class enumerated with a test (ADR-0075 condition (h)); alerts on a group held over 5 minutes, on joins answered `busy`
for 5 minutes, on `capacity_groups_stalled` above 0 for 10 minutes and on `capacity_members_unfinished` above 0 for 10
minutes; and `docs/runbooks/workflow-primitives.md` gains the nesting section (enable, read the wait, a stuck holder, a
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
| A process without the guard leases a member's step | The engine lists the capability only with guard, creator and sweeper; the serving gate; the fence and the two-phase removal |
| A successor run escapes the guard | `capacity_member` enforced by triggers |
| Transfers fill the audit table | Hourly summary records; receipts kept 15 minutes |
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
| VI. Observability and audit are part of the feature | Pass | Member creation, finish, retirement and hourly transfer summaries audited; metrics with closed labels; HELD-age and stalled-group alerts |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Each transfer writes its outbox hint; a lost hint is recovered by the level-triggered sweeper |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Requirement 039-012's matrix in both orders with barriers and the possession observer; the 100,000-instance workload on a separate host |
| Delivery Constraints | Pass | PostgreSQL as authority, no new component; the latency target measured on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Possession and a sweeper | One tenant unit per root, deadlock-free at limit 1 (Decision 705) | A unit per descendant: deadlock at limit 1 (ADR-0030 §14) |
| A quantum and a yield row | Fairness among members (Gap 718); a busy holder is never quiescent without it; the row needs no header lock the holder's claims keep | The yield on the header: almost always `Busy` under the holder's claims |
| A fan-out bound | The tenant limit bounds groups, not iterations | No bound: a tenant at limit 1 runs thousands of iterations |
| A scoped guard and `FinishPath` | A forEach in a forEach must lease and finish | Leaving scoped paths fail closed: the commonest nesting cannot run |
| Scoped-path commands of their own | Decision 706; the binding differs from a ChildRoot's | One command with a discriminated binding: rewrites the built and tested ChildRoot commands |
| A chain for every member child | ChildRoot membership is bound to a chain | Keying ChildRoot members by run: rewrites the built commands, the unique index and the guard |
| One group per root loop | A loop carries its unit from iteration to iteration | A group per loop iteration: the carried unit is released twice |
| Receipt retention and hourly transfer audit | Bounded storage (Gap 720) | Keep for ever: unbounded growth at a rate the author controls |
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

## Non-Functional Requirements

See Requirements, Non-functional (Non-functional requirements 039-001 to 039-003).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Four slices in order; slices 2 and 3 can run in parallel in two lanes with no crate in common: **lane A**
(`crates/infra-postgres` statements and migrations, `crates/scheduler`) and **lane B** (`crates/domain`,
`crates/infra-ir`, `crates/application` definition service and lease producers, the six SDKs, the scripts). Order
points: Task 039-2's migration (lane A) merges before any lane-B code reads the new columns; Task 039-12 (activation)
closes both.

**Slice 1, rules and checks** (lane B): Tasks 039-1, 039-3, 039-4. **Slice 2, creator, predicate, sweeper** (lane A):
Tasks 039-2, 039-5, 039-6, 039-7, 039-8, 039-9, 039-18, 039-19, 039-20. **Slice 3, the codec and the SDKs** (lane B):
Tasks 039-10, 039-11, 039-21. **Slice 4, proof and activation** (both): Tasks 039-12 to 039-17, 039-22.

- [ ] T039-1 Write this design out, decide the open items of ADR-0078 in ADR-0154 and ADR-0156, accept ADR-0078 and
  record the delegate rows.
  Refs: Requirement 039-004, Requirement 039-005
- [ ] T039-2 Migration: column-scoped `INSERT` grants on groups and members with the insert trigger of Requirement
  039-008 (header `HELD` with counters 1 and `shard` from the root run, member `ACTIVE`, no member of a `RETIRED`
  group); the unique index on ACTIVE ChildRoot memberships per chain and the partial unique index on ACTIVE scoped-path
  memberships per root iteration; `held_since`, `owns_unit` and `transfer_count` on the header, `last_vacated_unleased_at`
  on the member, with the `UPDATE` grant extended to them; `capacity_group_yields`, `capacity_transfer_audit`;
  `capacity_member` on runs (`NOT NULL`, the successor-copy, member-insert and never-cleared triggers of Requirement
  039-015); the `nesting` flag on versions; `INSERT` grants on the scoped columns of the finish receipts; the
  scoped-path receipt table; the retention triggers on the receipt tables and the replaced delete guards of headers and
  members with `capacity_purge_retired`; the partial indexes the sweeper's selections need; the `nested-collections-v1`
  registry value and `nested_collections_required()`; the down migration's refusal while any group or member exists;
  with the migration checklist and negative privilege tests for each grant, trigger and function.
  Refs: Requirement 039-008, Requirement 039-009, Requirement 039-011, Requirement 039-015
  Owner-gate: OD-2026-11-26-141
- [ ] T039-3 Write `worker-lock-sequence.json` in the code's order (the guarded claim, the creator, the join, every
  capacity command with the chain first and the retiring finish's tenant-usage lock last) and extend
  `scripts/check-capacity-activation.py` with the ordered-marker check, the `NOWAIT`/`SKIP LOCKED` rule, the allow-list
  of command callers (the sweeper only) and the check of ADR-0075 condition (g); negative fixtures seen failing.
  Refs: Requirement 039-010
  Owner-gate: OD-2026-11-26-146
- [ ] T039-4 Domain: the quantum and the yield, the oldest-ready choice with the skip, the scoped-path command rules and
  the `FinishPath` proof over the lifecycle, the membership context, the fan-out bound, the sweeper's six selections in
  their order, as pure functions with property tests.
  Refs: Requirement 039-005, Requirement 039-006, Requirement 039-014
  Owner-gate: OD-2026-11-26-142, OD-2026-11-26-143, OD-2026-11-26-145, OD-2026-11-27-443
- [ ] T039-5 The creator: group and first member at a root collection admission or root loop entry of a nesting version
  only, after `nested_collections_required()`; the loop's later members joining its group; ChildRoot join with a chain
  for every member child (one segment without `chainPolicy`, `link_child_chain` at any instance path), the header's
  compare and set and `active_members` under `NOWAIT`; `Busy` and `NESTED_COLLECTIONS_INACTIVE` retried by the next tick
  with no attempt consumed and counted; the descendant admissions of `primitives.rs` and `loop_progress.rs` through the
  membership context behind the run flag; the four decrement sites skipped for grouped iterations and loops and one
  decrement at retirement; audit and metrics; the limit-one tests (a child with and without `chainPolicy`, a child
  containing a collection, a three-iteration `while` containing a collection with `active_iterations` asserted after
  every checkpoint, a cancelled iteration whose children compensate), the policy-reduction test, a chainless child
  inside a body refused a lease while not the holder, and a test that a top-level child of a version published before
  the requirement creates no group.
  Refs: Requirement 039-002, Requirement 039-016, Success criterion 039-001
  Owner-gate: OD-2026-11-26-141, OD-2026-11-27-391, OD-2026-11-27-392, OD-2026-11-27-439, OD-2026-11-27-440, OD-2026-11-27-444
- [ ] T039-6 The shared runnable predicate from the candidate query of `workers.rs` (compensations of stopping runs
  runnable, paused runs not) and its differential test against that query on a corpus with paused, stopping and
  compensating runs; the guarded poll's rotating sample and claimable window (Gaps 715, 724, 725); a limit-1 test that
  cancels a root iteration whose two ChildRoot members both compensate.
  Refs: Requirement 039-007
  Owner-gate: OD-2026-11-26-147
- [ ] T039-7 `VacatePath`, `AcquirePath` and `FinishPath` with their receipts and quiescence over the iteration path,
  the chain first in their lock order, the retiring finish's unit release; the built ChildRoot commands fenced on the
  group's shard and releasing the unit when they retire a group that owns it; the scoped guard in both lease producers
  with the instance path and the yield row, and the scoped clause of the shared predicate; compensation after
  retirement leased outside possession; the commit uncertainty classifier shared; a test per terminal outcome (seven)
  and body shape of `FinishPath`, the race tests of the scoped guard, and the limit-1 test of a compensated successful
  iteration.
  Refs: Requirement 039-003, Requirement 039-004, Requirement 039-005, Requirement 039-016
  Owner-gate: OD-2026-11-26-145, OD-2026-11-27-390, OD-2026-11-27-445
- [ ] T039-8 The `capacity_sweep` pass: six selections in order, finishes first, served by the replica holding the
  group's shard, tenant-weighted pages, the yield budget, deferral counted, operation keys from the whole tuple, no lock
  held across a command; held-lock, restart, lost-hint and single-member no-churn tests; pause-after-yield and
  sibling-times-out-after-yield tests; transfers counted per quantum; a noisy tenant beside a limit-1 tenant; an
  activity no worker serves.
  Refs: Requirement 039-004, Requirement 039-005, Requirement 039-006
  Owner-gate: OD-2026-11-26-142, OD-2026-11-26-143, OD-2026-11-27-443
- [ ] T039-9 Receipt retention pass (15 minutes and the latest 16) and the hourly transfer audit with their tests;
  `capacity_purge_retired` and its tests, called by feature 020's purge when it exists (Gap 039-008).
  Refs: Requirement 039-009
  Owner-gate: OD-2026-11-27-438, OD-2026-11-27-447
- [ ] T039-10 Open the codec to the shapes of Technical Plan 8 under the registry requirement; `nesting_of` and the
  fan-out bound computed from the parsed document; `NESTED_CHILD_DETACH` and `NESTED_FAN_OUT_LIMIT`; a `parse_stored`
  test with the requirement removed; refusals of every unlisted shape.
  Refs: Requirement 039-001, Requirement 039-014
  Owner-gate: OD-2026-11-27-437, OD-2026-11-27-441
- [ ] T039-11 The six SDKs: nested bodies through the existing builders, a compile test that every present call still
  builds, and the scenarios `nested-collections` (build) and `nested-collections-runtime`.
  Refs: Requirement 039-013
- [ ] T039-12 The race matrix of Requirement 039-012 and ADR-0075 condition (e) as the runtime role, with the possession
  observer (Success criterion 039-002), and the limit-one and fairness tests (Success criteria 039-001 and 039-003).
  Refs: Requirement 039-012, Success criterion 039-001, Success criterion 039-002, Success criterion 039-003
- [ ] T039-13 The design-038 budget lines for a run without nesting, with the requirement off and on, for admission,
  claim and completion (Non-functional requirement 039-002), and the generic-plan cost of the guard's second read (Gap
  726).
  Refs: Non-functional requirement 039-002
- [ ] T039-14 The dedicated full-stack nested run on the integrated branch before activation.
  Refs: Requirement 039-011
  Owner-gate: OD-2026-11-26-148
- [ ] T039-15 The 100,000-instance nested workload on a separate database host, twice, with the dated report.
  Refs: Non-functional requirement 039-001, Success criterion 039-004
- [ ] T039-16 Telemetry, the alerts (HELD age, busy joins, stalled groups, unfinished members), the refusal-class
  enumeration test, and the runbook's nesting section.
  Refs: Requirement 039-011
- [ ] T039-17 Activation: the configuration switch and the capability listing of Requirement 039-011 (the engine only
  with guard, creator and sweeper), the fence and the two-phase removal with its descriptor, the registry requirement,
  the gate record of ADR-0075 (a) to (l) with each condition's evidence, `docs/traceability.md`, the roadmap row and
  the release notes; feature 014's moved clauses closed here.
  Refs: Requirement 039-011
  Owner-gate: OD-2026-11-27-393, OD-2026-11-27-442
- [ ] T039-18 The fence function `nested_collections_required()` (`FOR KEY SHARE` on the requirement row, owner-owned,
  `search_path` pinned) called by the creator, publication and activation; phase 2's drain check under `IN SHARE MODE`;
  held-lock tests of a creator racing each phase in both orders; a test in `apps/execution-engine/tests/registry.rs`
  that an engine with the guard off and the passes on does not list the capability; a test that a silent engine's poll
  of a member's step grants nothing.
  Refs: Requirement 039-011, Requirement 039-003
  Owner-gate: OD-2026-11-27-442
- [ ] T039-19 The `capacity_member` triggers of Requirement 039-015, with a continue-as-new successor of a member child
  that is refused a lease while not the holder, and a member insert refused for an unflagged run.
  Refs: Requirement 039-015
  Owner-gate: OD-2026-11-27-446
- [ ] T039-20 The invariant of Requirement 039-016 as a consistency check run by the integration suites after every
  checkpoint, drain and cancellation test of Tasks 039-5 and 039-7.
  Refs: Requirement 039-016
  Owner-gate: OD-2026-11-27-444
- [ ] T039-21 `nesting_of` used by every version-insert path (the ad hoc path included) and the architecture check that
  refuses an `INSERT INTO workflow_versions` that does not bind it, with a negative fixture.
  Refs: Requirement 039-002
  Owner-gate: OD-2026-11-27-446
- [ ] T039-22 The fan-out bound at an active-version child's admission, refused with `NESTED_FAN_OUT_LIMIT`, and the
  wrapped-collection test (a one-item outer `forEach` around a `forEach` of concurrency above the policy value refused
  at publication; one within it runs at most the bound at once).
  Refs: Requirement 039-014
  Owner-gate: OD-2026-11-27-437

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, `scripts/check-capacity-activation.py`, the unit tests of the crates it changes and the suites it touches,
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
