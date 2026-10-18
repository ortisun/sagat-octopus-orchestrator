> Review status: cycle3 found two substantive unresolved issues: possessor finality can
> strand the group, and the literal run-first prefix conflicts with chain serialization.
> This draft is NOT accepted. Do not implement or enable nesting from it.

# Nested execution capacity — third proposal

Status: unimplemented and unaccepted. The second proposal's three review findings are actionable;
it is superseded as a candidate, not retroactively accepted. Nesting remains closed.
Contract: ADR-0030 §§12–19 and feature014's quotas, child/chain cancellation and transactional rules.

## Capacity group and logical membership

A durable tenant-owned capacity group reserves one unit. It is independent of any one run's
lifetime and contains typed memberships for accepted execution paths `(tenant,namespace,run,path)`.
Its one enabled holder is an execution **path**, not one worker task: existing parallel activities
inside that path retain their ordinary task/worker quotas. Different descendant paths cannot execute
under that same unit at the same time. Additional groups can provide real parallelism when capacity
allows. Group membership grants no namespace, worker, publisher or secret authority.

Local scope slots and lifetime/derived-instance reservations remain distinct. A nested path/child
can inherit an existing group; its local slot and every activated instance are still charged.
Fresh independent iteration admission or a new group requires room under the tenant's current
retained-capacity bound. Enabling a parked group additionally requires room under its current
enabled-capacity bound. Existing in-flight holders survive reductions; no new enable above the
reduced bound is allowed. Limit0 stops new forward execution until policy permits it.

Candidate projection mapping: legacy `tenant_iteration_usage.active_iterations` counts retained
groups, preserving its flat-workflow value (one group per admitted flat iteration). Add a separate
enabled-group counter; local logical slots continue to be counted by each scope's active_count.
No logical slot is released merely to reuse capacity. Document/test these distinct counts before
exposing nesting; do not claim nested logical-slot totals equal tenant retained groups.

A group retires its capacity only when every membership, enabled holder and outstanding transfer
is finished. Its tombstone/receipt identity may remain for retention/replay without holding capacity.
Parent finality/cleanup cannot delete a group still referenced by an independent child.

## Transfers, parking and recall

All writes start with the current owning run's lock/fence, then the documented tenant capacity/group
order. No operation locks or mutates a second existing run. Every group transfer carries a monotonic
epoch and addressed recipient membership; duplicates/CAS failures cannot create another holder.

1. **Offer:** the current holder's own fence proves it has no executable work, live lease or unknown
   outcome requiring reconciliation/review under existing effect policies. Durable waits may coexist.
   Disable the holder, persist an offer to an eligible group member and its wake hint atomically;
   retain the group reservation. Claim/release paths are gated by the current holder and epoch.
2. **Consume:** the recipient's own run/shard fence rechecks membership, offer epoch, tenant capacity
   and all ordinary admission limits. CAS the offer, enable one holder and schedule its bounded work
   in its own transaction. Parent state is already parked and cannot release work during the gap.
3. **Park:** a holder with no live/in-flight/unknown obligation may disable itself under its own fence
   even without a descendant offer. Parking, yield/return metadata and their discovery are permitted
   above reduced limits; fresh work is not. Ready local work can remain durable but unclaimable while
   parked. A path suspended only on durable waits must park even without an offer. After a reduction,
   a safe idle holder above the enabled bound must park before taking more forward work, allowing
   eligible groups to re-enable within the reduced bound. Do not require spare capacity merely to park.
4. **Recall/yield:** a woken ancestor or another eligible group member requests possession through
   the shared record; the current holder handles that request under its own fence. A safely parked
   borrower can yield possession while its child/chain/iteration is still unfinished. Keep its
   logical membership and outcome obligations; child FINAL is never required for this execution
   transfer. The requester consumes only a recorded yield offer, never steals an active holder.
5. **Fairness:** persist both group possession requests and the tenant queue of groups waiting to
   enable. A safe holder must yield/park before taking more forward work if an older eligible
   request is queued, within its group or at the tenant enable queue. Record yield-pending while
   existing parallel leases finish and prevent replacement/new forward claims during that drain;
   otherwise an overlapping lease pipeline could prevent quiescence indefinitely. Lease completion
   and durable-wait parking are safe yield points. Discovery of parking/recall/return is independent
   of fresh-admission filters.
   Signals/timers retain first-commit-wins state updates, but an unenabled path releases no effect
   work. Critical unknown outcomes remain reconciled/reviewed, not declared safely parked by fiat.

Thus a capacity1 parent can lend to a child waiting forY, wake onX, recall from the safely parked
child, execute the activity producingY, then yield back. The child remains logically unfinished
throughout. A currently executing/non-preemptible effect may cause a real capacity wait; this does
not add a physical exactly-once or forced-preemption promise.

## Cancellation and continuing children

Execution possession is separate from semantic parent-child joins. REQUEST_CANCEL retains its
existing wait-for-child outcome rule. PROPAGATE requests child cancellation; DETACH does not.
Both sever only the semantic wait required by their policy; they do not transfer sole ownership
of the capacity unit to the deepest borrower.

For A→B→C, if B detaches C and reaches FINAL, A and C retain their independent group memberships;
B's membership/obsolete offer is settled by its own fenced transition. A's semantic link observes
B's FINAL through the existing child-link protocol. A can request possession without waiting for
C's FINAL, and C can continue under its own authority. Shared records retain/reconcile all member
and transfer references; stale upstream return hints cannot consume a newer epoch. There is no
upstream promise that B must return exclusive ownership it no longer possesses.

With spare capacity, an eligible existing path can obtain an additional group under its own fence
and current capacity checks, leaving the other member's group/holder unchanged. With exhausted
capacity, safe yield permits progress; an active/manual-review holder may impose a capacity wait.
Already accepted compensation/reconciliation remains representable and competes for eligible
capacity without inventing new forward-instance budget or silently waiting for a detached child
to become FINAL.

Child-chain membership spans segments. Handoff transfers retained membership in the existing
child transaction; CONTINUED is not logical completion. Holder transfer must be separated from
enabling successor effects: a reduced/disabled group can remain parked across handoff. New forward
admissions recheck all current policies, lifetime budgets and execution eligibility; metadata/retained
ownership transfer is not permission to grandfather new forward execution above a reduced limit.
Only the child's FINAL/drain settles its logical membership, independently of possession yield.

## Required acceptance evidence

All prior review counterexamples, both wake/consume orders, safe recall versus a simultaneous
claim/heartbeat, two idle enabled chains reduced2→1, cancellation at every subloan depth, manual
review with spare/exhausted capacity, continuation while parked and stale return epochs must pass.
Reconcile counters against actual groups/holders/members and local slots after crash/restart.
Migration/backfill, old active flat runs, mixed-version reader/worker fencing, rollback guards,
bounded indexed discovery and 100k-instance measurement remain mandatory implementation gates.
No old engine/control-plane process may execute new group-gated work without the new gate; the
rollout/capability fence requires actual old-process evidence, not an assumed compatible parser.
