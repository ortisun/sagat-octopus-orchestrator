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
2. **The sweeper serves every state.** It MUST finish a final holder, finish a final non-holder, vacate a quiescent
   holder that has no runnable work or has held for the quantum while another member has runnable work, acquire a vacant
   group for the member whose oldest runnable step became ready first, and finish the members of a vacant group whose
   members are all final.
3. **Fairness by a quantum.** The quantum is 1 second by default, configurable from 100 milliseconds to 60 seconds;
   `held_since` on the header records when possession began.
4. **Scoped paths have their own commands and receipts** (Decision 706 of feature 014), with quiescence over the
   member's iteration path.
5. **Receipts are bounded:** per group the latest 16 and those younger than 24 hours are kept; the audit record of each
   transfer is the permanent evidence.
6. **The worker's lock sequence is a checked file**, `specs/039-nested-collections/contracts/worker-lock-sequence.json`,
   verified against ordered markers in the code, together with the rule that every `capacity_groups` row lock carries
   `NOWAIT` or `SKIP LOCKED`.
7. **Activation is a registry requirement**, `nested-collections-v1`, recorded only after every condition of ADR-0075's
   gate holds; the first membership follows it.

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
