# ADR-0154: Nested capacity membership is created by the engine inside the admitting transaction, the sweeper serves every member state with a quantum, and the worker's lock sequence is a checked file

**Status:** Proposed (2026-11-26). Returned from Accepted the same day: the architecture review of design 039 (round 1) found decisions this record claims but does not yet make (the scoped-path guard, the release of the tenant unit, how the guard is enabled, descendant admission); it is accepted with design 039's approval
**Decided by:** agent proposal with design 039, accepted by the agent preparing design 039 for build under the owner's
delegation (Owner decision 2026-11-24-4); recorded as Owner decision 2026-11-26-150. The owner may overrule it.
**Refines / supersedes:** decides the items ADR-0078 lists under "Open and not decided here" (the creator, the sweeper's
coverage, the worker's complete lock sequence, scoped-path commands, the receipt ceiling, fairness) and conditions (c),
(d) and (f) of ADR-0075's activation gate; refines ADR-0030 §19 and ADR-0037 §6 only by placing the group header after
the locks a child creation already takes; supersedes none.
**Enforced by:** `scripts/check-capacity-activation.py` with the checked lock-sequence file (Task 039-3); the held-group
test per writer, the race matrix and the fairness test (Tasks 039-8, 039-12); the differential test of the runnable
predicate (Task 039-6). Until they exist: review only.

## Context

ADR-0078 fixed the group header as a lock nobody waits for and possession as moving only through three fenced commands,
and left six items open (design 039, The Problem). Nesting cannot be activated until each is decided.

## Decision

1. **The creator is the engine.** A nesting root admission MUST insert the group and its first member in the transaction
   that takes the tenant unit. A child created inside a group's work MUST be joined in its creation transaction, after
   the parent-first creation locks (the parent run, the tree-admission advisory lock, the new rows), by locking the group
   `FOR UPDATE NOWAIT` and inserting the member; `Busy` rolls the creation back for a retry on the next tick, with no
   attempt consumed. No API creates a group or a member.
2. **The sweeper serves every state.** It MUST finish a final holder and a final non-holder whatever their outcome (the
   last finish retiring the group), request a yield of a holder that has held for the quantum while another member has
   runnable work, vacate a quiescent holder only while another member has runnable work, and acquire a vacant group for
   the member whose oldest runnable step became ready first. A group is served by the replica holding the shard of its
   root run, recorded on the header, and every command for it reads that shard's fence.
3. **Fairness by a quantum and a yield.** The quantum is 1 second by default, configurable from 500 milliseconds (the
   sweeper's tick) to 60 seconds; `held_since` on the header records when possession began. `RequestYield` sets
   `yield_requested_at` on the header by compare-and-set of its revision under `NOWAIT`, without moving possession;
   while it is set the guard grants the holder no new lease (leases only, never a report); `Vacate` and `Acquire` clear
   it. The creator's initial `HELD` and this flag are the two header writes outside the three commands of ADR-0078
   section 3.
4. **Scoped paths have their own commands, receipts and guard** (Decision 706 of feature 014): `VacatePath`,
   `AcquirePath` and `FinishPath`, with quiescence over the member's iteration path and the lock order chain, run,
   group `NOWAIT`, shard `FOR SHARE`. `FinishPath` serves every terminal outcome and every body shape from the source
   facts, with no certified success. With the requirement recorded, both lease producers gate a step inside a
   scoped-path member's iteration on that member's possession, keyed by (run, root node, iteration index), and leave
   the root run's other steps ungated; this amends ADR-0075's rule that scoped paths fail closed, which stays true
   while the requirement is not recorded.
4a. **Descendants join; the unit is released at retirement.** An admission inside an ACTIVE member (a scoped-path
   member's iteration, or the run of a ChildRoot member) skips the fresh-admission check and the tenant charge
   (ADR-0075 (c) and (d), ADR-0030 §14); groups are created only for versions published with nesting under the
   requirement; the tenant unit of an iteration that owns a group is released by the `Finish` that retires the group.
5. **Receipts are bounded:** per group the latest 16 and those younger than 24 hours are kept; the audit record of each
   transfer is the permanent evidence.
6. **The worker's lock sequence is a checked file**, `specs/039-nested-collections/contracts/worker-lock-sequence.json`,
   verified against ordered markers in the code, together with the rule that every `capacity_groups` row lock carries
   `NOWAIT` or `SKIP LOCKED`.
7. **Activation is a registry requirement**, `nested-collections-v1`, recorded only after every condition of ADR-0075's
   gate holds; the first membership follows it. A process lists the capability only when its configuration runs the
   guard (control plane) or the creator and the sweeper (engine) (ADR-0075 (f), ADR-0082); the removal `verify` refuses
   while any group is not `RETIRED`; a binary rollback below the requirement is unsupported. Retired groups are deleted
   with their root run's purge, bounding the total.

## Considered options

1. **Engine-created membership, a covering sweeper with a quantum, a checked lock file** — closes every open item with
   no new lock and no wait on the header.
2. **An API that creates groups** — a way to put a chain into a group it does not belong to, for no gain.
3. **No quantum** — a holder with endless short steps starves its siblings.
4. **Receipts kept for ever** — growth of one receipt per quantum per group.

## Consequences

A creation can be retried for a busy group, which costs a tick of latency and never a wait. The sweeper's tick does
more selections; its bounds (256 candidates, 64 commands) cap the cost. Old receipts disappear; the audit trail keeps
the history.

## Evidence

Design 039 (`docs/designs/039-nested-collections.md`), ADR-0075 and ADR-0078, feature 014's
`specs/014-workflow-primitives/capacity-*.md` and research Decisions 701 to 715.
