# ADR-0100: A fleet requirement may require another, and the registry refuses either order

**Status:** Proposed
**Decided by:** agent, delegate (Owner decision 2026-11-20-6), split out of Architecture decision 0099 on the architecture reviewer's advice (round 5 of `specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`, Minor 9: one decision per record). Acceptance is the owner's, with Architecture decision 0099.
**Refines / supersedes:** refines Architecture decision 0082 (adds a relation between requirements; its decisions 1, 2 and 7 are kept, and a "Refined by" pointer goes into it at acceptance) and Architecture decision 0098 decision 4 (its generic `capability-activate` command learns the relation). Supersedes none.
**Enforced by:** to be built with Architecture decision 0099 (`specs/006-sdk-multilanguage/tasks.md`, Task 006-044 and Task 006-046): the trigger tests of both orders, also with the hand SQL of `docs/runbooks/instance-registry.md` section 5 and with the race between an activation and a removal held open; the typed refusals of the command; the check in `scripts/check-architecture.py` that the trigger's pairs equal `Capability::requires()`, with its negative test and its row in the fitness-function table of `docs/engineering/architecture-principles.md`.

## Context

The instance-capability registry of Architecture decision 0082 records, per process kind, which capabilities every live process
must list; the operator records a requirement, and a process that lacks a recorded one is refused with a closed gate. Until now
each requirement stood alone. Architecture decision 0099 adds `delivery-acknowledgement-v1`, whose leases are withdrawn with the
event of `lease-withdrawal-v1` (Architecture decision 0098): recording the first without the second would let a binary that cannot
replay the event be admitted, and removing the second while the first is recorded would do the same. Feature 022 will add further
entries to the generic activation command. The registry's rules hold even against the owner's hand SQL: its triggers are
`ENABLE ALWAYS` and recording goes only through its activation functions, which take the epoch row first (Architecture decision
0082 decisions 1 and 2). The closed capability lists in SQL are already rewritten by the migration that adds a capability, and
held equal to `Capability::ALL` by a check (Decision 006-033 of `specs/006-sdk-multilanguage/research.md`, server build).

## Decision

1. A capability MAY require other capabilities, each for named process kinds. The relation is declared once, in the application,
   as `Capability::requires()` beside `Capability::ALL`; the activation entries of the generic command read it and do not repeat it.
2. The command MUST refuse, under the epoch lock, to record a requirement whose dependencies are not recorded for every kind they are
   needed for (the refusal `DependencyNotRecorded`, naming the kinds), and to remove a requirement that another recorded requirement
   needs (`RequiredBy`, naming it).
3. The registry MUST refuse the same in SQL, whatever the caller: a trigger `platform_requirements_guard_dependency` (`BEFORE INSERT
   OR DELETE`, `ENABLE ALWAYS`, an invoker function with schema-qualified names), named so that it fires after
   `platform_requirements_guard`, so that it runs only inside an activation function that already holds the epoch row, and reads
   without locking. Its error text is distinct from those of the guard and of the replay floor. Its pairs are a constant of its
   function.
4. `scripts/check-architecture.py` MUST hold that constant equal to `Capability::requires()`, so the two cannot drift.
5. A dependency is added by the migration that adds its capability, which already rewrites the closed capability lists; that
   migration's `down` restores the previous body of the function. The guard of Architecture decision 0082 is never rewritten for it.

The first pair: `delivery-acknowledgement-v1` requires `lease-withdrawal-v1` for control planes and engines.

## Considered options

1. **A constant of a trigger of its own, held equal to the application's relation (chosen).** One source, checked; no rows an operator
   could edit as data, only a function that changes with the schema; the reviewed guard untouched; it moves with the capability lists, which are rewritten the same way.
2. **A table of dependencies, seeded by migrations.** Needs its own grants, immutability triggers on `UPDATE`, `DELETE` and
   `TRUNCATE`, and a check that the seed equals the code: more to protect for a relation that changes only with a capability.
3. **A new body for `platform_requirements_guard`.** Rewrites a guard that has been reviewed, and restores it in every `down`.
4. **Only the command checks the relation.** The hand SQL of the runbook would bypass it.

## Consequences

Good: no order of activations or removals, by the command or by hand, can record a requirement without what it requires; the
operator gets a typed refusal that names what is missing; the relation has one source. Bad: each new dependency is a function body
in a migration, with its `down`, the same cost as a capability list; the check must parse that body. Compatibility: a requirement
with no dependency behaves exactly as before; a binary that does not know the trigger is unaffected, because recording happens only
through the owner's functions. Rollback: the migration's `down` drops the trigger and its function once no requirement that has a
dependency is recorded.

## Evidence

Architecture decision 0082 and its migration `crates/infra-postgres/migrations/20270113000000_instance_registry.up.sql`; Architecture
decision 0098 (decisions 4 and 8) and its server build's `apps/control-plane/src/capability_activation.rs`; Architecture decision
0099 decision 10; Decision 006-047 of `specs/006-sdk-multilanguage/research.md`; the round-4 and round-5 reviews under
`specs/006-sdk-multilanguage/reviews/2026-11-21-delivery-ack/`.
