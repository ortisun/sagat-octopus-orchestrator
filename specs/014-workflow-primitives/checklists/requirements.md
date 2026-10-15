# Specification Quality Checklist: Durable workflow primitives

**Purpose**: Validate design completeness before implementation planning.
**Created**: 2026-10-14. **Feature**: [spec.md](../spec.md).

## Content quality

- [x] User value and delivery boundary are explicit: current examples vs proposed runtime primitives.
- [x] Mandatory user stories, requirements, entities, success criteria and assumptions are complete.
- [x] Specification describes observable behavior; stack, storage, proposed APIs and source structure
  live in plan/data-model/contracts rather than implementation code in the specification.
- [x] Existing condition/static parallel/waits/children/sagas are distinguished from new behavior.

## Requirement completeness

- [x] No unresolved clarification markers; first-increment assumptions and exclusions are recorded.
- [x] All 17 FRs and four NFRs use testable behavior and have task traceability.
- [x] Six SCs specify measurable outcomes and distinguish future release evidence from current checks.
- [x] Seven independently testable user stories include happy, rejection/failure and recovery cases.
- [x] Empty inputs, ordering, limits, stale ownership, races, cancellation, retained state and rollover
  semantics are covered in `spec.md`, `data-model.md` and `contracts/conformance.md`.
- [x] Dependencies use existing features 001/002/003/004/006/008/009/010/011/012; Studio work is separate.
- [x] Checked against accepted ADRs and canonical contracts (2026-10-14): compensation order
  (FR-003-002), value disclosure (ADR-0028), closed `Error.code`, built-in roles (G39),
  `Run.version`, non-buffered signals, tree budget, `rerunOf` and the runtime graph (ADR-0016/0017).

## Feature readiness

- [x] Closed acyclic bodies and stable logical instance identity preserve graph invariants.
- [x] Transactions include state/scheduling/timer/subscription/outbox and applicable receipts/audit/quota.
- [x] Critical external effects remain at-least-once with idempotency/reconciliation and saga drain.
- [x] Tenant/RBAC/secret protection, OTel, typed errors and authorization replay are specified.
- [x] Five-language common fixtures and crash/duplicate/race/compensation/restart coverage are planned.
- [x] Same-version continuation, old-run addressing, explicit chain operations and child semantics agree.
- [x] All proposed engine/SDK/API implementation and release gates remain unchecked in `tasks.md`.

## Open implementation gates

- [ ] Accept and number the proposed ADR and dependency contract changes before engine implementation,
  including the iteration compensation rule (ADR-0005) and the new permissions' role grants.
- [ ] Accept a value-disclosure decision refining ADR-0028 before releasing Query/Update (US06).
- [ ] Confirm the product decisions listed in `research.md` (Product decisions to confirm).
- [ ] Implement and validate versioned canonical schemas, storage and runtime primitives.
- [ ] Execute new fault/security/compatibility/performance suites and all-five primitive conformance.

The checked items validate the written design only. The open gates are intentional future work,
not unanswered semantic questions or evidence that a runtime guarantee already exists. Current
example evidence is recorded separately in [validation-report.md](../validation-report.md).
