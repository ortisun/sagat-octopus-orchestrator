# ADR-0037: Stable parent identity guards in child-chain handoff

Status: **Accepted scoped clarification** | Date: 2026-10-18. The existing guard is retained;
this decision introduces no new runtime capability.

Refines ADR-0030§18–19. Refs: T014-014..017/040, FR-014-012–015. The owner explicitly chose to
divide the unresolved nested-capacity protocol into smaller implementable/verifiable decisions.
This decision addresses only the existing child-handoff identity guard, not group eligibility.

## Context

ADR-0030 prohibits taking the other existing run/chain row while mutating a child link. Taken
literally as a prohibition on every PostgreSQL row-lock mode, that also prohibits the foreign-key
identity protection needed when a successor references its existing parent/root. The existing
handoff obtains its immediate parent's KEY SHARE guard early. It reads only the parent's identity;
it does not decide or mutate parent business state. Its actual order is chain→parent KEY SHARE→
own head→link, while an older comment/contract incorrectly stated parent→chain→head.

The separate mixed legacy flag UPDATE/link path has been removed. Chained parent transitions
now mutate their own run and link; a missing link alone selects the exact-own-parent legacy
fallback. The remaining early guard is therefore a separate, reviewable decision.

PostgreSQL 16 distinguishes these lock modes: KEY SHARE conflicts with FOR UPDATE but is compatible
with NO KEY UPDATE. Ordinary non-key UPDATE can therefore proceed under an identity guard; it
does not protect mutable parent business state. The local ordering argument below requires the
parent command's explicit FOR UPDATE aggregate lock. These are physical blocking locks that
remain part of the wait-for graph. See the [PostgreSQL 16 row-lock conflict matrix](https://www.postgresql.org/docs/16/explicit-locking.html#LOCKING-ROWS).

## Decision

1. Distinguish an aggregate mutation lock from a stable identity guard. A child handoff may take
   KEY SHARE on its existing immediate parent solely for foreign-key identity protection. It
   cannot use that guard as parent authority, read mutable parent business state to decide a
   transition, update the parent's run/chain, or upgrade the guard to a mutation lock.
   **Ancestry precondition:** the initial segment selected by chain_id, every current/successor
   child segment and the incoming link must name the same parent; all child segments retain the
   same root and depth. The parent/root mapping is immutable for this chain's lifetime. The guard
   resolves the initial segment, while successor insertion copies the locked current head, so
   equality of these identities is necessary for it to protect the intended parent.
2. Keep the existing handoff prefix explicit: own child chain UPDATE→immediate parent KEY SHARE→
   own head UPDATE→scheduler fence→incoming child link UPDATE. Applicable tree admission, active
   quotas, input/state retention and successor INSERT follow their documented current order.
   The successor INSERT also performs its existing parent/root foreign-key identity checks.
3. A parent command takes its own applicable chain lock and explicit run FOR UPDATE before its outgoing
   link. It neither locks the child's chain nor updates the chained child's legacy run flags.
   The early parent guard precedes the child's link acquisition: a parent that already owns its
   run excludes the handoff before that link, and a handoff that owns the guard excludes a
   competing parent mutation until commit. This is a local ordering argument, not a complete DAG.
4. Initial child reservation retains parent NO KEY UPDATE before tree/subordinate admission and
   payload retention. This still excludes parent mutation/reservation peers while admitting
   stable parent/root foreign-key references. Do not add an early root mutation lock or sort
   arbitrary run IDs onto this ancestry path.
5. Preserve the old legacy child-row protocol only when no own-parent child-chain link exists.
   Never combine that fallback's child UPDATE with a matching link mutation. Creation/link
   insertion and the parent's transition remain serialized by the parent aggregate lock.
6. The exception is narrow. Other foreign run-state reads/writes, late guard upgrades, missing
   chain prefixes, quota/session/activity ordering, old-writer fencing and group settlement
   remain separate B0/B/C obligations. A chain lock does not authorize another run's scheduler
   transition. No metadata guard permits holding two existing run mutation locks in this protocol.

The ancestry precondition is currently a writer invariant, not a claim that foreign keys enforce
cross-segment equality or prevent reparenting. Initial creation/commit derives its run ancestry
and ChainParent from the same RunInfo; link_child_chain uses that parent in the same transaction.
continue_evaluated copies the locked head's parent/root/depth into each child successor. No
production writer reparents a chain. Foreign keys prove referenced identities exist; the current
chain/link identity triggers do not establish the stronger invariant. Reparenting, ancestry
migration or another initialization writer requires revisiting this exception and proving the
initial/head/link mapping before using the guard. No migration is introduced by this clarification.

## Alternatives and limits

- Omitting the early guard leaves the successor's parent foreign-key check after link acquisition;
  the ordered parent-cancellation race is the required negative control for that wait-for edge.
- Moving parent business mutation into the handoff or locking the child run in its parent reopens
  the two-aggregate mutation protocol that ADR-0030 rejects.
- Globally weakening run locks or imposing sorted run IDs broadens the change beyond this guard
  and the existing ancestry/creation protocol.

There is no migration, SDK/API change, new scheduler authority or external-effect guarantee.
Existing journal, quota, link, audit and outbox commit boundaries stay unchanged. Retaining this
guard does not accept the complete B0 lock graph, a nested-capacity SQL implementation, compatible
old writers or the 100k resource gate. The nested runtime remains closed.

## Acceptance evidence

1. Remove only the early hold_parent call as a temporary negative control and run the existing
   ordered parent-cancellation/handoff race. Retain its actual result and source diff; do not
   describe a generic public error as an observed PostgreSQL deadlock without that evidence.
2. Restore the original source bytes and rerun the same race successfully. Record source hashes
   and restoration; the omitted-guard source is never a production candidate.
3. Retain the existing depth-two root-FK/new-child and quota/payload barriers, and the new trigger
   guards excluding chained-child legacy flag writes. They prove distinct local edges only.
4. Review this scoped exception independently, update ADR-0030 and dependent strict-lock wording,
   and keep all remaining B0/B/C tasks unchecked. Evidence directory:
   `tasks/project-completion/evidence/parent-fk-guard-20261018/`.

### Executed local control

At 3592444d, removing only the guard call made the existing two-order public race fail 0/1:
the handoff-first branch committed its handoff but parent cancellation returned Unavailable.
That public error is not an observed SQLSTATE/deadlock diagnosis. The parent→link versus
child-link→late-parent-FK cycle is a source-order inference, kept distinct from the raw result.
The original file bytes were restored exactly (matching SHA-256), and the same race passed 1/1
in 3.64s. No executable production change remains from the experiment.

Two independent scoped review cycles completed. The first identified the initial/head/link
ancestry equality precondition; it is explicit above as a current writer invariant, not a database
constraint. Positive assertions now check the initial segment, two successors and incoming link
in the final-notification test (1/1, 3.05s), plus distinct immediate-parent/root ancestry across a
depth-two handoff (1/1, 2.85s). These new assertions are positive coverage, not a claimed RED. The
second review found no remaining concrete precondition, authority, ordering, wording or oracle
defects. Neither reviewer ran tests. ADR-0030 and its dependents now reference this exception;
all other B0/B/C and canonical runtime tasks stay open.

## Related identity guards

ADR-0039 separately covers the unrelated rerun source FK guard before creation admission.
Its current root-rerun writer precondition does not extend this child-handoff exception.
