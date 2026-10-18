# Capacity transaction entry — B1a candidate

Status: reviewed scoped entry decision, unimplemented; only one existing group per owning transaction. Refs:T014-014..017/040,
ADR-0030, A0/A1 and the reviewed B0 source inventory at c0a8d587. Nested runtime stays closed.

## Artifact

A command concerning one existing tenant-owned group and one exact logical member enters through
its existing applicable chain and own-run guards, then obtains the selected group header with
FOR UPDATE NOWAIT before reading or changing group state. Success retains that guard through the
existing authority/admission suffix, any separately authorized A1 mutation and the actual commit,
including deferred checks, without acquiring another existing run mutation lock. An unavailable
group aborts and consumes the entire owning transaction and produces a distinct internal contention
outcome; a missing or mismatched group fails closed. Retrying requires a fresh transaction and fresh
state, and this entry neither acquires another group nor enumerates the groups belonging to a root.
This decides only entry for a dormant lifecycle adapter; nested execution, transition authorization
and production compatibility remain unaccepted.

## Preconditions

- Resolve and recheck tenant, namespace, own run/path and exact logical member association under
  existing own-run/chain authority. Caller-selected group ID is insufficient. A missing group never
  means Vacant, available capacity or legacy-flat fallback. Discovery reads grant no authority.
- The persisted header row is the one canonical serialization guard. Do not mix an advisory guard
  and a row guard as though they serialize one another. Every group/member/receipt writer follows
  this rule, including uniqueness and FK paths; no implicit reference to another group is allowed.
- Contention consumes/rolls back the entire transaction. No savepoint recovery, in-transaction
  retry, SKIP LOCKED, capacity substitution or unrelated committed effects. No transaction handle
  permitting further writes survives the failure. Retry starts outside the transaction.
- Before COMMIT dispatch, confirmed rollback/server abort establishes no committed effects. Once
  COMMIT may have been dispatched, caller cancellation, timeout or lost acknowledgement gives an
  unknown outcome until reconciled through the durable same-key/same-fingerprint receipt and state
  written atomically with the mutation. Do not infer rollback or issue a new independent mutation.
  Reconciliation starts from fresh entry; this slice must specify its durable receipt before any
  storage operation is accepted. Contention is distinct from commit uncertainty or quota refusal.
- Applicable chain → exact own run → group try → existing suffix preserves ordinary entry. Current
  parent KS/NK and rerun source KS rules remain intact; creation and handoff get no group integration
  in this slice. Existing scheduler/session clock samples stay after their real retained guards.
- One existing group only; no second group and no root-wide sweep. Root cancellation/reconciliation,
  fresh creation/join and continuation integration require their own later contracts.
- NOWAIT addresses the requested row lock. Relation/DDL, indexes, FK checks and later SQL can still
  block. This does not assert global deadlock freedom or remove existing suffix/deferred edges.

## What the first storage slice may establish

Bounded atomic persistence of an accepted A1 transition against an existing header and selected
member in isolated fixtures: validated restoration, current revision/epoch, non-reusable membership,
rollback and guard lifetime. Exact own-member authority is a separate prerequisite for production
Vacate/Acquire/Finish: the adapter must establish current own-path quiescence, eligibility or
settlement, not accept an unchecked fixture-supplied authorization flag. Fixtures are not a live
scheduler/worker integration or a rollout/capability fence.

## Counterexamples and boundaries

| Alternative / schedule | Required conclusion |
| --- | --- |
| Partial blocking group-first adaptation: parent holds own run U waiting G; child holds G waiting parent KS. Mixed chain/G prefixes likewise permit chain→G versus G→chain. | A new blocking prefix requires complete conflicting-writer adaptation, including ancestry and creation; inserting one helper cannot establish it. |
| NOWAIT in that parent schedule. | Parent group contention aborts its whole TX and releases own run. This removes that waiting edge on G; repeated contention may still starve. |
| Successful G try followed by existing suffix locks. | G does not remove baseline suffix cycles. Source-derived candidate: depth-two child continuation retains predecessor active-counter slot K before successor root R FK; independently terminalizing root holds R U then waits same K. Detached descendants make concurrent terminalization relevant. This is an inferred candidate requiring reachability/barrier proof, not an observed deadlock or a B1a repair claim. |
| Mixed old claimant after A vacates and B acquires. | Old claimant can lease A's ready work without G even after all TXs serialized. NOWAIT supplies no old-writer acceptance. |

## Required adapter evidence

1. Native chain-before-run entry, external G barrier, typed contention and full TX end. Independent
   probes acquire its former chain/run guards; no group/journal/receipt/quota/outbox committed effects.
2. Distinct own-run members contend for the same G in both orders. Loser rolls back; fresh retry
   observes winner's committed revision. No retry retaining the previous transaction.
3. Hold actual deferred retained-usage row during a valid commit; prove group and own-run guard
   lifetime, competing attempt abort, and one atomic result after release. Confirmed deferred server
   failure and cancellation before COMMIT dispatch roll back all effects. Cancellation/lost
   acknowledgement after dispatch must reconcile the durable receipt/state for either final
   outcome; never assert guaranteed rollback. Use exact SQLSTATE/barriers, not generic Unavailable.
4. Cross-tenant/member mismatch, absent group, stale revision/epoch and finished identity reuse are
   inert. Same-root unrelated groups remain untouched; no registry hydration or root-wide query.
5. Existing chain/parent/source identity regressions; nested publication still refused. Root cancel
   does no group sweep. Subsequent per-member reconciliation and terminal cleanup remain pending.

Next decisions: schema/RLS/implicit locks and exact own-member authority → one storage transition;
creation/join and handoff → bounded cancel reconciliation/separate terminal cleanup → eligibility,
quota projections, mixed-writer deployment and resource gates. Neither their acceptance nor fairness,
starvation freedom, 100k bounds or a global DAG is inferred from B1a.

Two fresh bounded Astra/xhigh reviews completed. Cycle1 corrected cancellation during COMMIT
from guaranteed rollback to unknown-outcome receipt reconciliation. Cycle2 found no remaining
substantive issues under this limited contract. No SQL adapter, own-member authority, rollout or
new runtime capability is accepted; see evidence/capacity-b1a-entry-20261018/review.md.
