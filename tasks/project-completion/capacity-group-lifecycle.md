# Capacity group lifecycle — isolated model contract A0

Status: abstract transitions reviewed with no substantive findings under their proof preconditions;
bounded command representation implemented as the isolated A1 pure model. This is one pure component of the decomposed design, not a
replacement whole-protocol proposal. Refs:T014-014..017, ADR-0030§§13–19. Runtime nesting stays closed.

## Artifact

A tenant-owned group retains one capacity reservation while any accepted logical member remains.
It has an immutable group/tenant identity, a revision for optimistic compare-and-swap, a possession
epoch, a non-reusable registry of member identities (active or finished), and possession `Vacant`,
`Held(member)` or `Retired`. Joining/finishing a member changes the revision; changing possession
also advances its epoch so stale ownership cannot authorize another acquisition. Finishing the
holder atomically makes possession Vacant even if no recipient is ready; any remaining member
may later acquire that vacancy under its own authority. The last active member's settlement
retires the group exactly once, but intermediate/final parent cleanup never removes another
member's retained membership.

## Transitions

| Command | Preconditions | Mutation |
| --- | --- | --- |
| Create(initialMember) | Fresh unique group/member; allocation authorized outside this model | Active initial member, Held(initialMember), initial revision/epoch |
| Join(member) | Group not Retired; never-used member identity; admission authorized outside model | Add active member; advance revision; existing possession unchanged |
| Vacate(holder,expectedEpoch) | Current holder+epoch; caller proves its own path is safely quiescent | Vacant; advance epoch/revision; retain every active member |
| Acquire(member,expectedEpoch) | Vacant at that epoch; active member; eligibility authorized outside model | Held(member); advance epoch/revision |
| Finish(member) | Active member; caller proves its own logical work has settled | Mark finished; advance revision; if holder, also vacate/advance epoch; if no active members, Retired |

Commands use the current expected group revision. A stale revision/epoch fails without mutation.
An exact duplicate command must not allocate another membership/holder/reservation; the application
may return its durable receipt, or recompute against the current state. A finished member identity
is never rejoined. Group Retired is absorbing. Integer overflow fails without partial mutation.
All non-retired groups require at least one active member; a Held member must be active.

## Contract and explicit boundaries

- At most one possession holder. Possession alone grants **no** ability to release/claim/execute an
  activity; current run/worker/namespace authority and tenant execution eligibility belong to B.
- No transfer steals from an active/in-flight/unknown holder. Quiescence/settlement facts must be
  established from that caller's own current execution state by B, inside the same durable mutation.
- A→B→C may share a retained group. B FINAL/DETACH settles only B. If B held possession, A and C
  survive with a vacancy; either can later acquire without reopening B. If C held it, finishing B
  leaves C's possession unchanged. Semantic child joins/cancellation policies are untouched.
- Child CONTINUED does not finish its logical chain membership. Binding the membership to the
  successor's own run authority is an adapter/chain transaction obligation in B/C, not a new
  unrelated member admission.
- Model tests prove these transitions only. Tenant retained/enabled/drain counter projections,
  fairness, directed offers/discovery, link/run/chain locking, migration and mixed-version fencing
  are unimplemented B/C dependencies. No quota-policy or progress guarantee is inferred from A.

## Required model evidence

1. Exhaust small command histories: one holder, active-member validity, monotonic revision/epoch,
   stale operations inert, no rejoining finished identities and Retired absorbing.
2. Reproduce B FINAL with A/C retained both while B holds and while C holds; vacancy acquisition
   has no dependency on the terminal source and never releases retained capacity early.
3. Vacate/reacquire races and duplicate finish leave one holder at most and one eventual retirement;
   finishing one non-holder cannot invalidate a current holder's possession epoch.

A0 abstract/representation/source review completed three component cycles with no substantive
findings under the stated preconditions. A1 model tests passed6/6, including1024 histories with
fixed seed3714 against an independent full registry. Counter overflow tests passed2/2. Domain
all-target strict Clippy, architecture24 crates and formatting passed. See
`evidence/capacity-a1-20261018/manifest.json`. B/C integration evidence remains pending.


## Bounded representation for A1

The registry above is a logical persisted registry, **not** a requirement to load every member into
one Rust object on every command. Production pure transitions use a bounded group header
(identity/tenant, revision/epoch, active-member count, possession) plus the selected member's exact
snapshot `(groupId, memberId, NeverSeen|Active|Finished)`. B must read that snapshot consistently
under the current group revision/lock and persist the returned header/member mutation atomically;
caller-invented or stale membership facts are outside the input contract, and a foreign group ID
is rejected by the domain. A1 uses a full-map reference model only in property/exhaustive tests,
checking the bounded transition result against the true registry after every operation. This
keeps ordinary command work independent of historical finished-member count; no SQL-bound or
migration guarantee is inferred until B/C actually provide that port.

Header restoration, before a persistence adapter is added, must validate nonzero revisions/epochs,
active count versus Retired and holder membership consistency. A1 need not invent a public
unchecked restoration API. Retired group IDs and finished member IDs remain non-reusable through
the actual persisted registry/uniqueness/receipt protocol in B/C; reconstructing a fresh object with
an already-used ID is not a second authorized allocation.
