# Specification Quality Checklist: Custom roles, groups, folders and per-operation access lists

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

## Content Quality

- [x] No implementation details (languages, frameworks, APIs) — with one deliberate exception: the spec
  names existing tables, files and ADRs to state the baseline and the compatibility boundary (house style of
  features 016 and 017). Requirements themselves are behavioural.
- [x] Focused on user value and business needs (delegation, team access, hiding work, explainability).
- [ ] Written for non-technical stakeholders — partly: the stories are plain, but FR-029-015/020/021 are
  necessarily precise about evaluation and revocation semantics. Accepted for a security feature.
- [x] All mandatory sections completed.

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain; the three genuine choices are owner questions O1440–O1442
  with recommended defaults, and the spec states the default it assumes.
- [x] Requirements are testable and unambiguous (each FR traced to tasks and to at least one SC).
- [x] Success criteria are measurable (counts, byte equality, time bounds, 100% coverage checks).
- [ ] Success criteria are technology-agnostic — partly: SC-029-004/005 refer to the API contract and
  SC-029-001 to the runtime database role, because the guarantee is about those boundaries.
- [x] All acceptance scenarios are defined (US01–US08, Given/When/Then).
- [x] Edge cases are identified (concurrency, revocation, role change, external source failure, ceilings,
  cross-tenant, crash points).
- [x] Scope is clearly bounded (Out of Scope; ownership boundaries with 013, 014, 021, 027).
- [x] Dependencies and assumptions identified (existing features, new features, G290, 013 exclusions).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria.
- [x] User scenarios cover primary flows (define, assign, group, folder, restrict, delegate, narrow
  operations, explain) and the dependent flows (scripts/connections; identity-provider groups refused).
- [ ] Feature meets measurable outcomes defined in Success Criteria — not applicable yet: nothing is
  implemented; the outcomes are release gates.
- [x] No implementation details leak into the requirement statements beyond the baseline references noted
  above.

## Constitution and house rules

- [x] Fail-closed authorization, opaque `NOT_FOUND`, no existence oracle (with the documented name-conflict
  residual G1441), revocation on the next admitted call, audit in the same transaction.
- [x] Tenant boundary on every row and identifier; runtime-role privileges planned and swept.
- [x] Typed errors reuse the closed `Error.code` set; OTel with closed labels; finite ceilings table.
- [x] Rule-9 failure tests planned (crash points, duplicates, revocation/signal race, restart); lost broker
  delivery and failed compensation stated as not applicable with the reason.
- [x] Five-SDK conformance for client-visible outcomes; no IR or worker-protocol change.
- [x] Unproven guarantees are gated (restricted mode and operation lists per item kind, behind a coverage
  test, an opacity battery, a canary scan and a binary requirement check) rather than claimed; identity-provider
  groups are not built (no approved dependency).
- [x] Independent read-only security and architecture reviews were run on the first draft (2026-10-26); their
  high and medium findings are fixed and recorded in research.md ("Review findings", D1471–D1479).
- [x] An independent architecture re-review (2026-10-27) found B1, B2, S1–S4 and minor issues; all applied and
  recorded in research.md ("Re-review findings") without new identifiers.
- [x] Cross-feature consistency (coordinator decisions C4, C10): additive built-in roles with set-based
  tests, subject-bound schedule and link rechecks, evaluator adoption by 020/021/025/026, ADR-0050 slot 1
  (D1655–D1657).

## Notes

- The unchecked items are honest partial passes, not blockers for planning.
- Built-in role count: the code has seven (G1440); the input said six.
- Owner review is required for ADR-0064 and O1440–O1442 before implementation (T029-001).
