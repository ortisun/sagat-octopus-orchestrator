# Specification Quality Checklist: Internal apps

**Purpose**: Validate specification completeness and quality before planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

Filled by the specification author after self-review; an item is checked only when the criterion
holds, and a note says where it holds only partly. This is not owner review.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — **partly**: as in the house style of
  feature 017, the spec states security contracts that must be testable (CSP directives, sandboxed
  frames, opaque session tokens, route families). Crates, tables and libraries are kept in the plan.
- [x] Focused on user value and business needs (seven stories from author, operator, viewer,
  administrator and developer perspectives).
- [ ] Written for non-technical stakeholders — **partly**: the context, stories and success criteria
  are; the requirements are written for reviewers of a security-sensitive platform feature.
- [x] All mandatory sections completed.

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain; three owner questions (O1360–O1362) carry recommended
  defaults in research.md.
- [x] Requirements are testable and unambiguous (each FR is traced by at least one task and one SC).
- [x] Success criteria are measurable (counts, matrices, zero-leak canaries, a timed authoring task).
- [ ] Success criteria are technology-agnostic — **partly**: SC-027-007/009 name browser behaviour
  and axe-core because the criterion is about the browser boundary and WCAG evidence.
- [x] All acceptance scenarios are defined (Given/When/Then for US01–US07, including failures).
- [x] Edge cases are identified (continuation, contract change, expiry, rate, truncation, popups,
  encrypted fields, restart).
- [x] Scope is clearly bounded (bounded builder; out of scope lists generic designer, delegated or
  anonymous execution, marketplace, resource operations before 021/022).
- [x] Dependencies and assumptions identified (existing 001–017; drafts 020, 021, 022, 026, 028, 029,
  031).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria.
- [x] User scenarios cover primary flows (build/publish, operate, disclose, lifecycle, audience,
  embedding, custom components).
- [x] Feature meets measurable outcomes defined in Success Criteria (SC-027-001..013 map every FR).
- [ ] No implementation details leak into specification — **partly**, see the first item.

## Constitution-specific checks (house rules)

- [x] Transaction boundaries, lock order and failure matrix stated in the plan.
- [x] Tenant boundary, typed errors, OTel with closed labels and audit stated.
- [x] Finite ceilings table with defaults and hard ceilings; zero is not unlimited.
- [x] Rule-9 failure tests planned (crash at commit points, duplicate, timeout vs signal/update race,
  lost broker hint, failed compensation, restart, tenant isolation, quota exhaustion).
- [x] Unproven guarantees marked as gaps (G1361 exfiltration, G1364 issuer prerequisites, G1371 no
  measurements).

## Notes

- The unchecked items are deliberate house-style trade-offs, not missing content; a reviewer may
  move security detail from spec to plan if preferred.
- `/speckit-clarify` is not needed for internal ambiguity; the owner questions are product choices.
