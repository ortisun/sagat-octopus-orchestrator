# ADR-0100: Withdrawn: a fleet requirement may require another

**Status:** Withdrawn (2026-11-21), before acceptance; the number is retired so that no other branch takes it.
**Decided by:** agent, delegate (Owner decision 2026-11-20-6), on the design reviewer's round-6 finding (Minor 5) and the architecture reviewer's round-7 confirmation (`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`).
**Refines / supersedes:** none.
**Enforced by:** review only: the record decides nothing.

## Context

Round 5 of the review of Architecture decision 0099 split out a decision of its own: a trigger of the instance-capability registry
that would refuse recording `delivery-acknowledgement-v1` before `lease-withdrawal-v1`, and removing the second while the first is
recorded.

## Decision

None. Once every withdrawal of a marked lease reads both requirements under the locks their removals wait for, and an engine marks
only while it knows both are recorded, no order of recordings and removals can write `step.lease_withdrawn` without its replay floor;
a wrong order costs only marks that are never honoured, and is counted and alerted. The trigger, its static check and its typed
refusals bought nothing more. What remains is considered option 10 of Architecture decision 0099.

## Considered options

1. **Withdraw it (chosen).** Nothing is left for it to enforce.
2. **Keep it.** A trigger, a function body rewritten in every migration that adds a dependency, a check that parses it, and race
   tests, for an ordering the withdrawals already make safe.

## Consequences

The registry is unchanged by Architecture decision 0099 apart from its capability and the amendment of Architecture decision 0082
decision 5 that the build of Architecture decision 0098 carries.

## Evidence

Architecture decision 0099, decision 10 and considered option 10; Decision 006-047 of `specs/006-sdk-multilanguage/research.md`; the
round-5, round-6 and round-7 reviews under `specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`.
