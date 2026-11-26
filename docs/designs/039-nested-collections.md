# Nested collections and the capacity program

*   **Author:** Claude Code, for the owner
*   **Status:** Draft
*   **Date:** 2026-11-26

--------------------------------------------------------------------------------

## Overview

General nesting of feature 014's primitives in release 0.3.0 (Owner decision 2026-11-24-1): a `forEach` whose body
holds another `forEach`, a loop, an approval or a child workflow, and the capacity program that makes nested admission
safe at any tenant limit, including 1. Feature 014 closed on root collections and handed this scope over (Owner decision
2026-11-12-6); its machinery is built and dormant (the capacity group, the native `Vacate`, `Acquire` and `Finish`
commands, bounded holder discovery, the ChildRoot worker-lease guard behind `child_root_capacity_guard`, off). This
design finishes it: it decides the items Architecture decision 0078 left open, adds the parts that are missing (the
creator of groups and members, the sweeper's caller and its coverage, the shared runnable predicate, the scoped-path
commands, fairness, receipt retention), states the activation that the gate of Architecture decision 0075 requires,
and then opens the codec to nested documents.

The rules are [Architecture decision 0078](../adr/ADR-0078-capacity-group-lock-order-and-possession-protocol.md)
(accepted with this design, delegate) and
[Architecture decision 0154](../adr/ADR-0154-nested-capacity-membership-is-created-by-the-engine-and-the-sweeper-serves-every-member.md),
which decides its open items. The registers are in `specs/039-nested-collections/research.md`; feature 014's registers
(`specs/014-workflow-primitives/research.md`, Decisions 701 to 715, Gaps 706 to 726) stay where they are and are cited.

### Terms

- **Capacity group (group).** The unit of tenant capacity that one root admission takes (`retained < limit`, Decision
  704): a header row `capacity_groups` with a possession state (`HELD`, `VACANT`, `RETIRED`), a holder, a revision and
  an epoch.
- **Member.** A row of `capacity_group_members`: a **ChildRoot** member (a child workflow's chain started inside the
  group) or a **scoped-path** member (one iteration of a root collection, by run, root node and iteration index, whose
  body nests work).
- **Possession.** The right of one member, the holder, to be granted worker leases (Owner question 706). Only the
  holder's work runs; the others wait. Nothing else is gated by possession.
- **Join.** A descendant becoming a member of its ancestor's group. A join charges no tenant unit (Decision 705), so a
  suspended ancestor cannot starve its descendants at any limit.
- **Sweeper.** The scheduler-tick caller of the three commands (Decision 714, Owner question 705): level-triggered,
  bounded, one transaction per command.
- **Runnable predicate.** One SQL fragment that says whether a member has work a worker could be leased now, shared by
  the scheduler's candidate selection and by the sweeper (Gap 715).

## The Problem

Feature 014 ships root collections, loops, approval, switch, declared operations and a restricted continue-as-new. A
nested document is refused by the codec (`crates/infra-ir/src/convert.rs`, `published_collection_definition`). The reason
is capacity: each root iteration takes one tenant unit; if an inner iteration or a child workflow started inside it
needed a unit of its own, an outer iteration holding its unit while it waits for inner work would deadlock the tenant at
a limit of 1 (ADR-0030 §14). Feature 014 chose to let descendants join their ancestor's unit (Decision 705) and to let
only one member of a unit run at a time (possession), so that the tenant's limit still bounds what runs.

Much of that is built and dormant: the tables and the three native commands with their receipts (migrations
`20261210000000_capacity_finish_storage`, `20261212000000_scoped_finish_binding`, `20261225000000_capacity_discovery_index`,
`20261231000000_capacity_worker_membership_indexes`; `crates/infra-postgres/src/capacity_*.rs`), bounded holder discovery,
and the worker guard. What is missing is listed by feature 014 (`specs/014-workflow-primitives/tasks.md`, the "Current
frontier" notes; `capacity-*.md`), by the activation gate of ADR-0075 (conditions (a) to (l)), by the "Open" items of
ADR-0078 and by Gaps 715, 716, 718, 719, 720, 722, 724, 725 and 726 of feature 014. In short: nothing creates a group or
a member, nothing calls the commands, the sweeper would not finish non-holders or retire a vacant group whose members
are all final, scoped paths have no transfer commands, the guarded poll can starve a non-holder (Gap 724) or fill its
window with runs the claim refuses (Gap 725), fairness among members is not claimed (Gap 718), receipts grow without
bound (Gap 720), and the worker's complete lock sequence is not written where a check can read it.

Temporal runs child workflows inside loops without a tenant concurrency model; Windmill's for-loops nest without one.
Nesting that holds a tenant limit under a crash at any point is what this platform promises and neither of them does.

## Requirements

### Functional

- **Requirement 039-001** A `forEach` body MAY contain a `forEach`, a loop or a child workflow, to a depth of 4
  collection or loop levels, and a collection MAY stand beside loops, approvals and other nodes at the top level instead
  of only in a chain of collections ending at `end`; an approval inside a body opens only with feature 014's scoped
  compensation of approvals (Decision 295 of feature 014). The definition service MUST accept such a document only when
  the requirement `nested-collections-v1` is recorded in the instance registry (ADR-0082), and the codec MUST still
  refuse every shape this design does not list (Technical Plan 8).
- **Requirement 039-002** A root admission that may nest MUST create a group with its first member in the admission's
  transaction; a descendant MUST join the group of its ancestor and MUST NOT reach the fresh-admission check (Decision
  705). Joining is bounded by the lifetime `maxInstances` budget and by the local scope slots.
- **Requirement 039-003** Only the holder MAY be granted worker leases (ADR-0075 as built, Owner question 706). Reports,
  heartbeats, timers, signals, retries, control nodes and child outcomes MUST NOT be gated by possession.
- **Requirement 039-004** Possession MUST move only by the three fenced commands of ADR-0078 for ChildRoot members and by
  their scoped-path twins (Technical Plan 4), called only by the sweeper, each in its own transaction, holding no other
  lock or transaction of the caller.
- **Requirement 039-005** The sweeper MUST serve every member state that can occur: a quiescent holder with another
  member waiting (vacate), a vacant group with a member that has runnable work (acquire), a holder or non-holder whose
  head is final (finish), and a vacant group whose members are all final (finish the last, which retires the group).
- **Requirement 039-006** No member MAY wait without bound while others run: when another member has runnable work, a
  quiescent holder that has held possession for at least the quantum (1 second by default, 100 milliseconds to 60
  seconds) is vacated, and `Acquire` chooses the member whose oldest runnable step became ready first.
- **Requirement 039-007** Candidate selection and the sweeper MUST use one shared runnable predicate, held equal by a
  differential test (Gap 715); the guarded poll MUST rotate its sample of ineligible runs (Gap 724) and MUST NOT count
  runs the claim refuses toward its eligible window (Gap 725).
- **Requirement 039-008** One ACTIVE ChildRoot membership per chain MUST be enforced by the unique index of ADR-0078
  section 5 and ADR-0075 condition (k), and one ACTIVE scoped-path membership per root iteration by the existing unique
  constraint.
- **Requirement 039-009** Transfer receipts MUST be bounded: per group, the latest 16 and every receipt younger than 24
  hours are kept; older ones are purged in batches of at most 1,000 by a bounded pass. The audit records of each
  transfer stay as the permanent evidence.
- **Requirement 039-010** The worker's complete lock sequence MUST be written in
  `specs/039-nested-collections/contracts/worker-lock-sequence.json` and checked against ordered markers in the code by
  `scripts/check-capacity-activation.py`, which MUST also refuse any `capacity_groups` row lock without `NOWAIT` or
  `SKIP LOCKED` (ADR-0075 condition (a)).
- **Requirement 039-011** Activation MUST follow the gate of ADR-0075: every condition (a) to (l) holds, then the
  operator records the requirement `nested-collections-v1`, which the registry refuses while a live engine or control
  plane lacks the guard-capable capability; the first membership can be created only after it.
- **Requirement 039-012** Crash, duplicate, stale fence, lost broker hint, cancellation, unknown or failed compensation
  and unknown COMMIT MUST be proven for nested scopes, in both race orders, with held locks or barriers.
- **Requirement 039-013** Nested scopes MUST be offered in all six SDKs through the collection builders that exist, with
  conformance scenarios that build, publish and run them.

### Non-functional

- **Non-functional requirement 039-001** The nested 100,000-instance workload (Owner decision 2026-11-04-7) runs on a
  separate database host, with a release build, as the runtime role, twice, and the report states the environment, the
  failures and the latency of a transfer (target: a waiting member acquires within two sweeper ticks of the holder's
  quiescence at the 99th percentile).
- **Non-functional requirement 039-002** A run without nesting MUST cost what it costs today: no added statement in its
  admission, claim or completion, held by the budget suite of design 038.
- **Non-functional requirement 039-003** The sweeper's tick MUST be bounded: at most 256 candidates and 64 commands per
  tick per replica.

### Success criteria

- **Success criterion 039-001** At a tenant limit of 1, a root iteration that waits on an inner collection of 10
  iterations, each of which starts a child workflow, completes; the tenant never has two members of different groups
  running at once.
- **Success criterion 039-002** The race test of ADR-0075 condition (e) passes as the runtime role: holder and non-holder
  pollers race the sweeper's `Vacate` and `Acquire`, an unknown COMMIT is injected on the lease path, no group ever has
  two members with a live leased or running step, and `Acquire` eventually succeeds.
- **Success criterion 039-003** With three members with runnable work, each is granted a lease within three quanta plus
  two ticks (fairness), measured.
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
- A run without nesting pays nothing.

## Non-Goals

- Anything feature 014 already ships.
- Changing the transactional boundary or the lock order beyond what ADR-0078 and ADR-0154 decide.
- Gating scheduler transitions, reports or heartbeats on possession (rejected by the owner, Owner question 706).
- A new tenant counter, a pause of admitted work or a drain lane (Decision 704, Owner question 701).
- Nesting deeper than 4 collection or loop levels in 0.3.0.
- Triggers on a workflow that has a `forEach` (Gap 042-003, taken over as Gap 039-006).

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

## Owner decisions needed

None open. Under Owner decision 2026-11-24-4 the items ADR-0078 left open, the open question of this design's first
draft and the confirmation of the delegated rows are decided as delegate rows (Owner decisions 2026-11-26-140 to
2026-11-26-149):

| Item | Decision | Why |
|---|---|---|
| Accept ADR-0078 (Owner decision 2026-11-26-140) | Accepted as written | Its third architecture and PostgreSQL review found no Blocker or Major; the open items are decided by ADR-0154, not by editing it |
| The creator (Owner decision 2026-11-26-141) | The engine creates groups and members inside the transactions that admit the work; no API | Membership is a consequence of admission, never a user act; an API would add a way to put a chain into a group it does not belong to |
| Sweeper coverage (Owner decision 2026-11-26-142) | Four selections: quiescent holder with a waiting member, vacant group with runnable work, final head of any member, vacant group with only final members | Without the last two a group's unit counts against admission for ever (ADR-0078, Open) |
| Fairness (Owner decision 2026-11-26-143) | A quantum of 1 second and oldest-ready-first `Acquire` | Without it a busy holder starves its siblings (Gap 718); a quantum bounds the wait by the number of members |
| Receipt retention (Owner decision 2026-11-26-144) | Latest 16 per group and 24 hours; audit is the permanent record | Receipts are idempotency keys for a retained command, needed for minutes; the audit trail already holds every transfer |
| Scoped paths (Owner decision 2026-11-26-145) | Commands and a receipt table of their own, as Decision 706 decided, quiescence over the member's iteration path | A forEach in a forEach is a scoped path; without them the commonest nesting cannot run |
| The worker's lock sequence (Owner decision 2026-11-26-146) | Written as a checked contract file | A list in prose drifts; a checked list fails the build when the code changes the order |
| The guarded poll (Owner decision 2026-11-26-147) | Rotate the ineligible sample by a keyset cursor and count only claimable runs | Closes Gaps 724 and 725, which block activation |
| The first full-stack run (Owner decision 2026-11-26-148) | A dedicated full-stack nested run on the integrated branch before activation; the release-candidate run repeats it | The release event must not be the first time the composition runs |
| The delegated rows of 2026-11-04 on nesting (Owner decision 2026-11-26-149) | OD-2026-11-04-3 to OD-2026-11-04-8 confirmed as they stand for release 0.3.0 | They were taken under the earlier delegation and nothing found since contradicts them |

## Critical User Journeys (CUJs)

### CUJ 1: A collection of collections

*   **Given**: a tenant at limit 1 and a workflow whose `forEach` over 3 customers has a body that is a `forEach` over
    each customer's orders (5 each) calling a child workflow per order.
*   **When**: it is published (after the operator recorded `nested-collections-v1`) and run, and the engine is killed
    once while the sweeper's `Acquire` is in flight.
*   **Then**: every inner iteration and every child runs once; at most one member of the one group runs at any instant;
    the killed `Acquire` is either committed or re-sent from the state; the run completes with the same result as on a
    run without the kill.

### CUJ 2: A holder that hogs

*   **Given**: a group whose holder has a long sequence of short steps and two child members with runnable work.
*   **When**: the holder runs.
*   **Then**: between two of its steps, after a quantum, the sweeper vacates it and the oldest waiting member acquires;
    each member is served within three quanta plus two ticks.

### CUJ 3: Nesting not yet enabled

*   **Given**: a deployment where one engine still runs a build without the guard.
*   **When**: the operator records `nested-collections-v1`.
*   **Then**: the registry refuses while that engine is live; publication of a nested document is refused with
    `NESTED_COLLECTIONS_DISABLED`; root collections run as before.

## Technical Plan (Architecture)

### 1. What exists and is reused

The tables, constraints and triggers of the capacity migrations; `Vacate`, `Acquire`, `Finish` for ChildRoot members and
the scoped leaf finish; the receipts and their commit-uncertainty classifier (Decision 713); bounded discovery and holder
discovery (`capacity_discovery.rs`); the worker guard (`crates/application/src/worker.rs`, `child_root_capacity_guard`);
`scripts/check-capacity-activation.py` and the step-status-writer inventory
(`specs/014-workflow-primitives/contracts/step-status-writers.json`).

### 2. The creator

- **Group and first member.** When a root collection admits an iteration whose body may nest (the body contains a
  collection, a loop, an approval or a child), the admission transaction that takes the tenant unit (`retained < limit`)
  also inserts the group (`HELD`, holder the new scoped-path member) and the member. The group row is new, so no lock
  on it is waited for. A root child workflow started outside a collection is a ChildRoot root admission and does the
  same with a ChildRoot member.
- **Join.** A child workflow started inside a group's work is created by the existing parent-first path (the parent run
  `FOR NO KEY UPDATE`, then the tree-admission advisory lock, then the new chain and run rows, ADR-0030 §19 and
  ADR-0037 §6); the same transaction then locks the group `FOR UPDATE NOWAIT` (ADR-0078 rule 2) and inserts the
  ChildRoot member `ACTIVE`. A `Busy` answer rolls the creation back and the step that creates the child is retried by
  the next tick, with no attempt consumed; nothing waits on the group. An inner collection or loop inside a scoped-path
  member's iteration joins nothing: it is work of that member.
- **Grants.** The migration grants `sagat_app` column-scoped `INSERT` on both tables (ADR-0075 condition (l)) and adds
  the unique index on ACTIVE ChildRoot memberships per chain (condition (k)).
- **Audit and telemetry.** `capacity.member.created` (a new action with its OCSF mapping in design 040: API Activity,
  Create), and `capacity_members_created_total{kind}`.

### 3. The sweeper

A pass `capacity_sweep` on the scheduler tick, on the shards the replica holds (ADR-0020), with no transaction open while
it calls a command (ADR-0078 section 4). Each tick it pages, with a keyset cursor and at most 256 candidates:

1. `HELD` groups whose holder's head is final → `Finish` the holder;
2. ACTIVE non-holder members whose head is final → `Finish` that member;
3. `HELD` groups whose holder is quiescent (no leased or running step of its work) and either has no runnable work or
   has held possession for at least the quantum while another member has runnable work → `Vacate`;
4. `VACANT` groups with an ACTIVE member that has runnable work (the shared predicate) → `Acquire` for the member whose
   oldest runnable step became ready first;
5. `VACANT` groups whose ACTIVE members all have final heads → `Finish` each, the last one retiring the group.

At most 64 commands per tick. The operation key is derived from the whole tuple each command fingerprints (ADR-0078
section 4); a `FingerprintConflict` means "rediscover". The quantum needs the time possession began: the header gains
`held_since timestamptz` set by `Acquire` (and by the creator), read by the sweeper only.

### 4. Scoped-path commands

`VacatePath`, `AcquirePath` and the scoped leaf finish serve scoped-path members with a receipt table of their own
(`capacity_path_transfer_receipts`), the same compare-and-set of revision and epoch, the same lock order (the run that
owns the path, then the group `NOWAIT`, then the shard `FOR SHARE`), and quiescence over the member's iteration path: no
leased or running step whose path starts with the member's root node and iteration index.

### 5. The runnable predicate and the guarded poll

One SQL fragment, `runnable_member_work`, in `crates/infra-postgres`, used by candidate selection and by the sweeper,
with a differential test that both answer the same on a generated corpus (Gap 715). The guarded poll samples ineligible
runs by a rotating keyset cursor instead of always the four oldest (Gap 724) and counts toward its eligible window only
runs whose claim the guard would accept (Gap 725). A run without membership takes the path of today unchanged
(Non-functional requirement 039-002).

### 6. The worker's lock sequence, checked

`specs/039-nested-collections/contracts/worker-lock-sequence.json` lists the locks of a guarded claim in order (the drain
advisory lock, the binding advisory lock for a scoped path, the worker session, the secret advisory locks, the chain, the
run, the group `SKIP LOCKED` after an unlocked header read, the activity quota locks, the breaker, the database and
script locks), each with the function that takes it. `scripts/check-capacity-activation.py` reads it and checks that the
named functions take them in that order (ordered source markers), and that no `capacity_groups ... FOR` lacks `NOWAIT` or
`SKIP LOCKED`. A real-PostgreSQL test per writer of a group or member row runs it with the group held on another
connection and expects `Busy` or a skip, never a wait.

### 7. Receipts and retention

A bounded pass deletes, per group, receipts older than 24 hours that are not among the latest 16, in batches of 1,000,
under a grant of `DELETE` on the receipt tables only to the pass's function. The down migration's refusal while a
receipt exists stays.

### 8. The codec opening

Today `published_collection_definition` (`crates/infra-ir/src/convert.rs`) accepts only an optional prefix of plain
source activities, then a chain of `forEach` nodes ending at `end`, each body one to eight activities, conditions,
switches, timers or event waits with local routes and one return; it refuses `chainPolicy`, `triggers`, an approval in a
body (pending scoped compensation, Decision 295 of feature 014), and a loop, approval, continue-as-new or return beside a
collection. This design opens, under the registry requirement:

| Shape | Today | After this design |
|---|---|---|
| A `forEach` inside a body | refused | accepted to a depth of 4 collection or loop levels; a scoped-path member per root iteration |
| A loop (`while`, `repeatUntil`) inside a body | refused | accepted within the same depth |
| A child workflow inside a body | refused (not a body node kind) | accepted; a ChildRoot member that joins the group |
| A collection beside loops, approvals and other nodes at the top level | refused | accepted; each root collection iteration that may nest takes one group |
| An approval inside a body | refused, pending scoped compensation | accepted only once Decision 295's dependency is met; Task 039-10 checks it and otherwise keeps the refusal (Gap 039-007) |
| `chainPolicy` with a collection | refused | still refused (continuation of a collection is not in this design) |
| `triggers` on a workflow with a collection | refused | still refused (Gap 039-006) |
| Source activities with `onError`, compensation, an effect policy or a database operation | refused | still refused |

The definition service refuses a nested document while `nested-collections-v1` is not recorded
(`NESTED_COLLECTIONS_DISABLED`). The codec reads no switch: a stored nested document stays readable with the requirement
removed (the switch is read at publication only, as design 042 does).

### 9. Activation

The order: every condition of ADR-0075 (a) to (l) proven by its task; every control plane and engine of the release
lists `nested-collections-v1`; the operator records the requirement; the first membership is created by the first
nested admission after that. Removing the requirement stops new nested publications and admissions of new groups;
existing groups drain through the sweeper.

### 10. Telemetry and operability

`capacity_sweep_commands_total{operation, outcome}`, `capacity_group_held_seconds` (a histogram of the age of `HELD`),
`capacity_member_wait_seconds` (ready to acquired), `worker_claim_skipped_total{reason}` (existing), the refusal class
enumerated with a test (ADR-0075 condition (h)), an alert on a group held over 5 minutes, and
`docs/runbooks/workflow-primitives.md` gains the nesting section (enable, read the wait, a stuck holder).

## Risks and Mitigations

| Risk | Mitigation |
|---|---|
| A lock cycle through the group header | Nothing waits on the header (ADR-0078 rule 1); the checked lock sequence; a held-group test per writer |
| Two members run at once | Possession gates every lease grant; Success criterion 039-002 races it with an unknown COMMIT |
| A member starves | The quantum and oldest-ready-first acquire; Success criterion 039-003 |
| A group's unit never frees | The sweeper's selections 1, 2 and 5 |
| Receipts grow without bound | Retention of Requirement 039-009 |
| A run without nesting pays for it | No membership, no change to its path; the design-038 budget suite |
| An old engine leases without the guard | The registry requirement before the first membership |
| A zombie worker or a blocking activity holds possession | Gap 039-001; the HELD-age alert |

## Constitution Check

| Principle | Result | Evidence |
|---|---|---|
| I. State and retry semantics are explicit and durable | Pass | Membership commits with the admission; each command compares and sets revision and epoch in one transaction; commit uncertainty re-sends the retained command (Decision 713) |
| II. Modular domain, ports and adapters | Pass | Possession rules in `crates/domain/src/execution/capacity.rs`; statements in `crates/infra-postgres`; the sweeper in `crates/scheduler` |
| III. External effects are at-least-once; sagas compensate | Pass | Nesting adds no external effect; compensation inside a nested scope keeps feature 037's rules |
| IV. Compatibility is versioned and pinned | Pass with conditions | Additive schema; the codec opens only under a registry requirement; stored documents stay readable. Condition: the schema is not rolled back once a receipt exists (ADR-0078 section 6) |
| V. Code, tenant boundary and secrets are isolated | Pass | Every row carries tenant and namespace; the runtime role gets column-scoped inserts only; no tenant label |
| VI. Observability and audit are part of the feature | Pass | Member creation and every transfer audited; metrics with closed labels; a HELD-age alert |
| VII. Events are redeliverable; inbox and outbox are the contract | Pass | Each transfer writes its outbox hint; a lost hint is recovered by the level-triggered sweeper |
| VIII. Failure is tested and contracts are verified, honestly | Pass | Requirement 039-012's matrix in both orders with barriers; the 100,000-instance workload on a separate host |
| Delivery Constraints | Pass | PostgreSQL as authority, no new component; the latency target measured on a stated environment |

## Complexity Tracking

| Addition | Why it is needed | Simpler alternative rejected |
|---|---|---|
| Possession and a sweeper | One tenant unit per root, deadlock-free at limit 1 (Decision 705) | A unit per descendant: deadlock at limit 1 (ADR-0030 §14) |
| A quantum | Fairness among members (Gap 718) | None: a holder with endless short steps starves its siblings |
| Scoped-path commands of their own | Decision 706; the binding differs from a ChildRoot's | One command with a discriminated binding: rewrites the built and tested ChildRoot commands |
| Receipt retention | Bounded storage (Gap 720) | Keep for ever: unbounded growth at one receipt per quantum per group |
| A checked lock-sequence file | ADR-0078 Open; prose drifts | Prose in the ADR |

## Alternatives Considered

- **Keeping nesting in feature 014** with its original criteria: rejected by Owner decision 2026-11-12-6.
- **Gating scheduler transitions on possession**: rejected by the owner (Owner question 706).
- **Inline `Vacate` in the worker or step transaction**: rejected by the owner (Owner question 705).
- **Counting each descendant against the tenant limit**: deadlocks at a limit of 1.

## Non-Functional Requirements

See Requirements, Non-functional (Non-functional requirements 039-001 to 039-003).

--------------------------------------------------------------------------------

## Detailed Implementation Plan

Four slices in order; slices 2 and 3 can run in parallel in two lanes with no crate in common: **lane A**
(`crates/infra-postgres` statements and migrations, `crates/scheduler`) and **lane B** (`crates/domain`,
`crates/infra-ir`, `crates/application` definition service, the six SDKs, the scripts). Order points: Task 039-2's
migration (lane A) merges before any lane-B code reads the new columns; Task 039-12 (activation) closes both.

**Slice 1, rules and checks** (lane B): Tasks 039-1, 039-3, 039-4. **Slice 2, creator, predicate, sweeper** (lane A):
Tasks 039-2, 039-5, 039-6, 039-7, 039-8, 039-9. **Slice 3, the codec and the SDKs** (lane B): Tasks 039-10, 039-11.
**Slice 4, proof and activation** (both): Tasks 039-12 to 039-17.

- [ ] T039-1 Write this design out, decide the open items of ADR-0078 in ADR-0154, accept ADR-0078 and record the
  delegate rows.
  Refs: Requirement 039-004, Requirement 039-005
- [ ] T039-2 Migration: column-scoped `INSERT` grants on groups and members, the unique index on ACTIVE ChildRoot
  memberships per chain, `held_since` on the group header, the scoped-path receipt table, the partial indexes the
  sweeper's five selections need, the `nested-collections-v1` registry value, the down migration's refusals; with the
  migration checklist and the runtime-privilege test.
  Refs: Requirement 039-008, Requirement 039-009, Requirement 039-011
  Owner-gate: OD-2026-11-26-141
- [ ] T039-3 Write `worker-lock-sequence.json` and extend `scripts/check-capacity-activation.py` with the ordered-marker
  check and the `NOWAIT`/`SKIP LOCKED` rule; negative fixtures seen failing.
  Refs: Requirement 039-010
  Owner-gate: OD-2026-11-26-146
- [ ] T039-4 Domain: the quantum, the oldest-ready choice, the scoped-path command rules, the sweeper's five selections
  as pure functions with property tests.
  Refs: Requirement 039-005, Requirement 039-006
  Owner-gate: OD-2026-11-26-142, OD-2026-11-26-143, OD-2026-11-26-145
- [ ] T039-5 The creator: group and first member at a nesting root admission; ChildRoot join after the parent-first
  creation locks with the group `NOWAIT`; `Busy` retried by the next tick with no attempt consumed; audit and metrics.
  Refs: Requirement 039-002
  Owner-gate: OD-2026-11-26-141
- [ ] T039-6 The shared runnable predicate and its differential test; the guarded poll's rotating sample and claimable
  window (Gaps 715, 724, 725).
  Refs: Requirement 039-007
  Owner-gate: OD-2026-11-26-147
- [ ] T039-7 `VacatePath` and `AcquirePath` with their receipts and quiescence over the iteration path; the commit
  uncertainty classifier shared.
  Refs: Requirement 039-004
  Owner-gate: OD-2026-11-26-145
- [ ] T039-8 The `capacity_sweep` pass: five selections, bounded pages, operation keys from the whole tuple, no lock held
  across a command; held-lock, restart and lost-hint tests.
  Refs: Requirement 039-004, Requirement 039-005, Requirement 039-006
  Owner-gate: OD-2026-11-26-142, OD-2026-11-26-143
- [ ] T039-9 Receipt retention pass and its test.
  Refs: Requirement 039-009
  Owner-gate: OD-2026-11-26-144
- [ ] T039-10 Open the codec to the shapes of Technical Plan 8 under the registry requirement; a `parse_stored` test with
  the requirement removed; refusals of every unlisted shape.
  Refs: Requirement 039-001
- [ ] T039-11 The six SDKs: nested bodies through the existing builders, a compile test that every present call still
  builds, and the scenarios `nested-collections` (build) and `nested-collections-runtime`.
  Refs: Requirement 039-013
- [ ] T039-12 The race matrix of Requirement 039-012 and ADR-0075 condition (e) as the runtime role (Success criterion
  039-002), and the limit-one and fairness tests (Success criteria 039-001 and 039-003).
  Refs: Requirement 039-012, Success criterion 039-001, Success criterion 039-002, Success criterion 039-003
- [ ] T039-13 The design-038 budget lines for a run without nesting (Non-functional requirement 039-002).
  Refs: Non-functional requirement 039-002
- [ ] T039-14 The dedicated full-stack nested run on the integrated branch before activation.
  Refs: Requirement 039-011
  Owner-gate: OD-2026-11-26-148
- [ ] T039-15 The 100,000-instance nested workload on a separate database host, twice, with the dated report.
  Refs: Non-functional requirement 039-001, Success criterion 039-004
- [ ] T039-16 Telemetry, the alert, the refusal-class enumeration test, and the runbook's nesting section.
  Refs: Requirement 039-011
- [ ] T039-17 Activation: the registry requirement, the gate record of ADR-0075 (a) to (l) with each condition's evidence,
  `docs/traceability.md`, the roadmap row and the release notes; feature 014's moved clauses closed here.
  Refs: Requirement 039-011

#### Follow-up tasks

None yet.

### Verification Plan

Every pull request runs `cargo fmt --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`, the three
checkers, `scripts/check-capacity-activation.py`, the unit tests of the crates it changes and the suites it touches,
also with `SAGAT_TEST_AS_RUNTIME_ROLE=1`. A test is seen failing for the right reason first; races use held locks or
barriers, never sleeps. `integration_workflow_primitives` and `fault_workflow_primitives` run unmodified and with the
nested tests by name; the budget suite of design 038 runs for every change of a claim or a completion. A hand mutation
per rule (take the group with a plain `FOR UPDATE`, skip the quantum, let a non-holder lease) is caught by a named test.
