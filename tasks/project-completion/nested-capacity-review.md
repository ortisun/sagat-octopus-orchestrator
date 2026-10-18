# Nested execution capacity — implementation constraints

Status: reviewed proposal requires correction; nesting remains closed. This document neither
introduces a new runtime guarantee nor supersedes ADR-0030. Refs: T014-014–017, T014-042,
ADR-0030 §§13–19, research G427.

## Proposal examined

Keep logical scope slots/lifetime reservations separate from durable tenant execution permits.
A suspended parent could lend a retained permit to a descendant without releasing it into the
global pool, then resume after return. The initial proposal limited lending to exclusively
descendant-blocked parents, rejected new loans above a reduced tenant limit, used one admission
transaction, and returned on borrower terminal status.

## Findings reconciled

Fresh read-only architectural review found five actionable omissions. Each follows from the
accepted contract, so none is accepted as a trade-off:

1. **Reduction before the first loan:** two retained parents reduced from capacity2 to1 cannot
   both be forbidden to forward their retained permits. Distinguish fresh allocation, retained
   ownership and eligibility to execute. Specify a serialized drain/progress path above the new
   limit, without increasing capacity or discarding accepted obligations.
2. **Mixed durable waits:** a parent in `parallel.any` may wait on an event and a descendant;
   the descendant can supply the first successful branch. Lending must account for no executable
   or in-flight parent work, rather than requiring exclusively descendant waits. A signal/timer
   wake must remain durably gated by execution ownership.
3. **Existing child on another shard:** parent authority cannot admit work in a reopened child.
   Use a durable offer/consume/return protocol under each run's own current fence, retaining the
   reservation throughout. Do not acquire two existing run rows or extend parent shard authority.
4. **Child continuation:** ownership belongs to the logical child-chain relationship, including
   outstanding subloans, rather than one segment. `CONTINUED` is not a return condition; handoff
   must preserve ownership until final outcome and drainage.
5. **Cancellation policies:** `PROPAGATE` and `DETACH` must not acquire `REQUEST_CANCEL`'s
   wait-for-child behavior. Define conversion/severance of the return dependency while preserving
   the child's capacity, and independently admit parent drain work where eligible.

These corrections require an implementable state/transaction protocol and tests before code can
enable nesting. Separate permits are allowed by ADR-0030 §14. A design changing child lock order,
current scheduler authority or cancellation semantics would instead need a new ADR and dependent
updates; no such divergence is approved here.

## Required counterexamples and fault evidence

- Capacity1 nesting; multiple descendant branches serialized without abandoning scope slots.
- Capacity2→1 reduction immediately before lending, during a loan and during return.
- Mixed `parallel.any` event/timer/descendant waits; wakeups both before and after loan consumption.
- Different parent/child scheduler shards, lease transfer and stale owners at each protocol step.
- Crashes/duplicate delivery before and after offer, consumption, return and continuation handoff.
- Multi-segment child with outstanding subloans; only FINAL plus drained obligations returns once.
- All three child cancellation policies with spare capacity, exhausted capacity and manual review.
- Actual retained ownership/execution counters versus persisted holders after replay/restart;
  unchanged tenant identity, lifetime ceilings, reservations, outbox and audit boundaries.

The review's first cycle is complete; the revised protocol is still pending. The user selected
the current review after the optional external cross-model review was offered. No external CLI
was authorized or invoked.


## Subsequent bounded reviews

Cycle2 (`nested-capacity-protocol-proposal.md`) found three actionable failures:
terminal-only return strands a parent that wakes while its borrower waits for an event the parent
must produce; reduction2→1 cannot park two independent idle enabled chains; and exclusive transfer
to a detached deepest child strands upstream return entitlements. No finding was accepted as a
trade-off. The capacity-group draft attempted to address these independently of semantic joins.

Cycle3 (`nested-capacity-protocol-v3.md`) still found two substantive actionable issues:

1. **Vacant group recovery is undefined.** In A→B→C, B possesses the group and C parks on a
   durable wait. If B detaches C and reaches FINAL, A later observes B FINAL but neither A nor C
   has a consumable offer. The protocol needs an explicit vacant/released state and fenced CAS
   acquisition, or a durable transferable grant installed at settlement, even with no ready
   recipient. Recovery cannot reopen a terminal source or lock two existing runs.
2. **Run-first prefix conflicts with accepted chain locking.** ADR-0030§18/data-model§6 require
   applicable chain serialization before the owning run, including direct-run commands. The
   draft's literal all-writes run-first prefix can deadlock a chain handoff. A complete existing
   prefix plus group/link ordering for cancellation/handoff is required; no lock-order ADR
   divergence is approved.

Both corrections can preserve ADR-0030; the group quota projection still needs canonical
specification/contracts, actual migration/mixed-version fencing and crash/scale evidence.
The doubt-driven-development three-cycle bound has been reached and escalated to the user.
No fourth whole-proposal review is authorized by that skill. The drafts remain unaccepted and
nesting stays closed while the user chooses decomposition or deferral. Existing independent
blob, SDK and measurement work continues; no task is checked for this design history.

## Owner's decomposition choice — 2026-10-18

The owner chose to split the protocol into smaller implementable/verifiable decisions. The
whole-protocol drafts above remain rejected. Current progress: A0/A1 and checked pure recovery,
B0 source inventory, B1a single-existing-group entry and B1b.1 child-root FINAL authority. Each
scoped review's limits remain explicit; B1/B2/C and runtime activation are incomplete. Continue
from tasks/project-completion/todo.md under standing all-open-tasks authorization.
