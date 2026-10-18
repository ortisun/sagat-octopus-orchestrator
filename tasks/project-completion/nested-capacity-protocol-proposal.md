> Review status: cycle2 found substantive progress failures. This proposal is retained as
> rejected design history, not an implementation contract. See nested-capacity-review.md.

# Nested capacity protocol — proposal for adversarial review

Status: unimplemented, unaccepted protocol proposal. Nesting remains closed. No guarantee or
new quota exception is advertised by this document. Contract: ADR-0030 §§12–19, feature014
spec/data-model and the existing child cancellation policies.

## Accounting

- Logical scope slots (`active_count`/`slot_held`) remain occupied until their accepted iteration
  obligations finish. Local maxConcurrency, shared lifetime maxInstances/derived reservations,
  maxDepth, payload charges and the1,024-run tree budget remain independent checks.
- A retained **forward capacity reservation** belongs to one admitted lineage. Descendants can
  reuse it; it is never temporarily returned to the global free pool. Fresh independent admission
  requires retained-forward reservations below current tenant capacity.
- An **enabled holder** may execute forward/derived work. Enabling, transferring or resuming one
  requires room under the current execution limit, serialized on the tenant capacity row. Existing
  in-flight leases survive reduction; new enables wait until that overage drains. A disabled
  reservation cannot claim/release external-effect work.
- Retained ownership may exceed a later reduced limit. Reusing a retained reservation does not
  require retained ownership below that limit; it requires enabled capacity. Quiescence/offer/
  return/parking metadata transitions remain discoverable and writable above either limit.
  At limit0 new forward execution intentionally stops; it resumes only after policy permits it.
- For a cancellation drain whose reservation remains with a detached/propagated child, allow a
  separately tagged **mandatory-drain reservation** only for already accepted derived obligations.
  This never grants forward admission or bypasses the enabled execution limit. Its count is
  bounded by accepted derived identities/reservations, not arbitrary new control activations.
- Retained forward reservations, mandatory-drain reservations, enabled holders and logical scope
  slots are separate persisted counters with independent reconciliation. Existing flat-workflow
  quota observations must remain compatible; the precise mapping of legacy `active_iterations`
  is an explicit implementation/contract question before migration, not inferred as already solved.

## Durable states and authority

Each retained reservation has one owner, at most one enabled holder and at most one outstanding
offer. Its owner/borrower identity includes tenant, namespace, run/chain, scope path and generation.

| Transition | Required authority and atomic effects |
| --- | --- |
| Offer | Parent's current run/shard fence. Prove no executable parent work, live leases or unresolved unknown effects; durable event/timer waits may coexist with pending descendants. Disable the parent, persist one addressed offer and park parent effect execution in the same transaction. Keep ownership reserved. |
| Consume | Child's own current run/shard fence (or same-run descendant's exact path fence). Lock tenant capacity then addressed offer in the documented run-first order, CAS an unconsumed offer, enable the borrower if room exists, admit its bounded work and write journal/audit/outbox atomically. Do not lock or mutate an existing parent run. |
| Wake while offered/borrowed | The owning run can record signal/timer outcomes and cancellation/control metadata under its own fence; it cannot release or lease effect work without its enabled holder. Preserve normal first-commit-wins wait semantics. A parent may CAS-withdraw an unconsumed offer; once consumed it must await return or cancellation conversion. |
| Return | Borrower's own fence. Disable its holder, record a once-only return after its obligations/subloans drain, and persist the parent's wake hint. Do not mutate the existing parent run. The parent accepts its return under its own fence and re-enables only if current execution capacity permits. |
| Subloan | A quiescent borrower can offer the same reservation further down its accepted lineage. One leaf holder executes; unwind subloans before an upstream return. Sibling descendants serialize when only one reservation exists. |
| Continue child | Bind borrower lifetime to the logical child-chain link. In the existing child handoff transaction, preserve reservation/subloan ownership and transfer the segment holder to the newly created successor without an enable/release gap. A CONTINUED segment never returns capacity. Reopened successor mutations require its own shard fence. Return only after FINAL and drainage. |
| Cancel REQUEST_CANCEL | Keep the return dependency and normal wait-for-child/drain behavior. Cancellation and offer withdrawal/consumption are serialized by the addressed record CAS. |
| Cancel PROPAGATE/DETACH | Send cancellation only for PROPAGATE, then sever the parent's semantic return dependency. Convert the child's retained reservation into independent ownership, without freeing or duplicating it. A parent with accepted compensation/reconciliation can acquire a mandatory-drain reservation; enabling it still obeys current execution capacity. Parent final metadata requires no child-final wait. |

Parent offer and child consume are distinct transactions, each with its own run-first lock order
and current scheduler authority. An uncommitted new child retains the existing narrowly allowed
creation authority; this does not authorize a reopened child. A newly created child cannot claim
effect work until its loan/own reservation has been enabled. All worker claim forms and planners,
including legacy root fallbacks, must apply that gate.

## Reduction and starvation counterexample

Capacity2, two retained parents waiting on descendants, reduced to1: either parent may write its
quiescent offer/disable transition despite the overage. Once both are disabled, one child can
consume/enable, then return/disable. Existing reservations compete for the single enabled slot
without allocating a third forward reservation. Fair scheduling must discover offers/returns even
when ordinary fresh-admission discovery is blocked by retained capacity.

An exhausted or manual-review child can legitimately occupy scarce execution capacity; this is
a capacity wait rather than a parent semantic join after PROPAGATE/DETACH. With spare execution
capacity, parent drain must proceed independently. No physical progress is promised while unknown
external effects retain every eligible holder or policy explicitly disables forward execution.

## Required implementation proofs before acceptance

- Every invariant and counterexample in `nested-capacity-review.md`, including both race orders.
- No duplicate enabled holder after crashes, offer replay, return replay, lost wake or shard transfer.
- Cancellation conversion versus child continuation/return, with spare/exhausted capacity and review.
- Mixed parallel.any durable waits versus child success and signal/timer wakeups.
- Per-run/tenant reconciliation against actual holders/loans; no whole-history hot-path restoration.
- Additive migration/backfill of active flat runs, mixed-version scheduler/worker compatibility,
  rollback guard for outstanding loans, unchanged old flat quota observations and receipts.
