# Constitution 3.0.0 (B01) — Sync Impact Report

This record accompanies the dedicated PR adopting the B01 amendment. It was prepared as 2.0.0 on
top of 1.1.0. On 2026-11-01 the owner split it out of feature 037, which adopts 2.0.0 (Principle III
only, compensation safety), so B01 is renumbered **3.0.0**. #75 merged first (`a76a3ac3`,
2026-11-02) and this text is rebased onto 2.0.0. **Status: PROPOSED.** The owner has not yet approved this text
(`docs/owner-decisions.md`, 2026-11-01 item 2).
The adopting PR's normal approval and merge process is the adoption evidence; a prepared diff
or passing check is not ratification. Owner approval is required for this reset by the owner's
task instructions, not introduced as a new project-wide ratification authority by B01. The
amendment does not certify the current implementation or a release.

## Scope and why MAJOR

Three ambiguities prevented readers and agents from following the existing accepted contracts:
the absolute ban on secret API responses contradicts capability-authorized executor delivery;
tenant metric labels contradict the cardinality rule; and a NATS-only baseline contradicts the
accepted Redis alternative (ADR-0069). This amendment explicitly relaxes the first and third
obligations and redefines the conflicting metrics obligation. It is not merely editorial.

The broader reviewed Constitution candidate is not adopted. Its C2 identifiers, SDK support
exceptions, compatibility changes, testing applicability, authority hierarchy and plugin rules
do not become active through this amendment. That proposal must be rebased before later use.

## Complete clause impact

The stable 1.1.0 assessment units below identify all changed units; every other unit is retained
verbatim by this bounded patch. They are assessment identifiers, not new runtime authorities.

| Old unit | Action | Result |
|---|---|---|
| V-3 tenant in credentials/SQL/messages/metrics/audit | REWRITE — obligation redefined | Tenant scope retained on authority-bearing surfaces; tenant/namespace metric identifiers forbidden; diagnosis through authorized logs/spans/durable queries |
| V-4 execution-time reference/capability and no secret API response | RELAX and strengthen guards | Two bounded sets (task-declared secrets; pinned managed credential), respective publisher/binding/pool or dedicated-executor rights; shared token/identity/lease fence and post-provider revalidation, managed-only session ownership; audit before return; no general read or classified-secret payload exception |
| VII-3 NATS-only hints | RELAX | NATS and accepted ADR-0069 Redis alternative, neither grants execution authority |
| D-1 technology baseline | RELAX in transport only | Redis alternative acknowledged; other technology, topology and NATS-support obligations unchanged |
| G-6 version metadata | UPDATE | 2.0.0 → 3.0.0 (prepared as 1.1.0 → 2.0.0); original ratification date retained, actual amendment date recorded on adoption |
| All remaining units | KEEP verbatim | Durable transactions, fencing, effect ambiguity, Principle III as amended by 2.0.0 (compensation safety), run compatibility, SDK parity, isolation, failure gates, history and governance procedure unchanged |

Preserved in particular: no exactly-once external-effect claim; critical effects require
idempotency or lookup; unknown effects must be looked up before repetition; Principle III as amended by 2.0.0
(compensation safety) is untouched; no privileged inline code; no blanket relaxation
of the database-access restriction; no adoption of the owner’s future plugin model by implication.

## Dependents updated together

- `.specify/memory/constitution.md`: only the clauses and metadata above.
- `AGENTS.md`: the opening transport summary, without changing numbered rules.
- `docs/engineering/architecture-principles.md`: transport summary.
- `docs/architecture/capacity-and-topology.md`: recognize only the approved transport baseline.
- `docs/architecture/security-and-sandbox.md`: metric attribution and executor disclosure wording.
- `docs/architecture/overview.md`: transport-neutral hint invariant and diagram.
- `README.md`: describe the configured alternative for this source checkout.
- `docs/runbooks/execution-engine.md`: selected dispatcher, broker inventory and backlog diagnosis.
- `docs/runbooks/scaling.md`: operator-owned broker and selected-backend egress/NetworkPolicy.

Existing ADR-0002/0017/0069 and feature 009/015/033 contracts are evidence, not rewritten history.
No new ADR number is allocated: the Redis decision already has ADR-0069, metric cardinality has
ADR-0017, and this is the explicit constitutional amendment for the executor exception.
Existing feature plans record their original checks; they are not retroactively marked compliant.
Any future change or release re-evaluates its Constitution Check against the active revision.
Spec Kit templates refer to the active Constitution without copying these three clauses; no
template rewrite or new implementation task is needed for this documentary amendment.

## Risks and validation

The secret-response exception is narrower than general read permission but broader than the old
literal ban. Authorize it only with the explicit fences above; policy or audit failure must never
fall through to disclosure. Replacing per-tenant metric labels must not remove tenant diagnosis.
Ordinary resolution treats the matching lease token as a bearer capability when the authenticated
caller also passes current membership/binding/pool and publisher checks. It does not prove that
the caller is the original lease-owning worker; session ownership is additional in managed database
resolution. Protecting tokens is essential. Stronger ordinary session binding would be a separate
behavior/specification/test change and is not claimed by this amendment.
Redis selection is not proof of fsync, failover, deduplication or safe activation; ADR-0069 retains
its own delivery/rollout gates. No runtime behavior, schema, broker selection or deployed secret
provider is changed by this amendment.

Review the exact diff and cross-references, run the existing agent-configuration validator on the
candidate tree, and verify application/reversal in a disposable copy. Those static checks do not
prove runtime, security, performance or deployment guarantees. Feature suites remain the evidence
for those guarantees when the respective behavior or release is tested.

## Rollback and remaining decisions

Before merge, discard only the B01 patch. After adoption, reverse it through a reviewed amendment
preserving the version history and the recorded reason; do not reset unrelated work or rewrite Git
history. Runtime has not changed, so restoring the old prose does not remove current capabilities
and would restore the three known inconsistencies.

The two compensation decisions once listed here (terminal `fail`, defensive compensation of an
unknown outcome) belong to feature 037 and Constitution 2.0.0; this amendment does not touch them. Fresh-install intent is already recorded as owner input and is not re-asked;
this amendment establishes no schema/protocol cutover. Product/plugin architecture and the
remaining engineering-reset work stay under their own approval batches.
