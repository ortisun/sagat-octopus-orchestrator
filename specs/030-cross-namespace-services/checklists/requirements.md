# Specification Quality Checklist: Cross-namespace service operations

**Purpose**: Validate specification completeness and quality before planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

Filled by the specification author on 2026-10-26. A tick means the item was checked against the
text; it is not a review by the owner, and nothing here is implementation evidence.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — **partly**: following the house style
  of features 014, 016 and 017, the spec names IR fields, permission strings, failure codes and a few
  existing code paths as evidence for the baseline, because they are contract-level and testable.
  Crate placement, tables and transactions are in `plan.md` and `data-model.md`, not in the spec.
- [x] Focused on user value and business needs (handler and caller teams, operators, security).
- [ ] Written for non-technical stakeholders — **partly**: the user stories are plain; the
  requirements are written for reviewers of a durable-execution platform, as in the other features.
- [x] All mandatory sections completed (user scenarios, requirements, success criteria, assumptions).

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain; three genuine product choices are owner questions
  O1480–O1482 with recommended defaults that the spec applies.
- [x] Requirements are testable and unambiguous (each FR names its observable outcome or code).
- [x] Success criteria are measurable (counts, 100% of injected cases, 100-round races, closed sets).
- [ ] Success criteria are technology-agnostic — **partly**: SC-030-011 and SC-030-014 mention traces,
  metrics and a benchmark report, which are product obligations of constitution VI and the Delivery
  Constraints.
- [x] All acceptance scenarios are defined (US01–US09, Given/When/Then).
- [x] Edge cases are identified (pause, continuation, purge, version drift, revision drift, name reuse,
  size limits, dead messages, reruns, self-calls, encrypted fields).
- [x] Scope is clearly bounded (out-of-scope list; first-increment restrictions are explicit errors).
- [x] Dependencies and assumptions identified (existing features, parallel drafts 018–032 by scope).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (traced to SC-030-001..015 and to
  tasks in `tasks.md`).
- [x] User scenarios cover primary flows (register, call, authorize, fail, cancel, bound, sync,
  observe, compensate).
- [x] Feature meets measurable outcomes defined in Success Criteria (as requirements; no evidence yet).
- [ ] No implementation details leak into specification — see the first item.

## Notes

- Gates before implementation (plan.md, Governance gate): owner acceptance of ADR-0065, answers or
  confirmed defaults for O1480–O1482, and feature 016's acceptance of the `serviceCall` carrier
  (G1493).
- Verified while writing: every cited path, requirement id (FR-002-009, FR-003-006, FR-007-010,
  FR-011-003, FR-014-016/017, FR-016-002/004/016, D36, D37, D283, G21, G39) and ADR (0002, 0004, 0010,
  0012, 0014, 0016, 0017, 0020, 0025, 0027, 0028, 0030, 0031, 0032, 0033, 0036, 0037, 0039) exists in
  the worktree at `70f09435`.
- Revised 2026-10-27 after the independent architecture review and coordinator decisions C1, C4, C6,
  C7, C8, C10 and C13: notification order (D1511), effect classes and outcome lookup (D1512), call
  attempts (D1513), input disclosure (D1514), 031 interplay (D1515, then D1661: encoded values are
  refused, not carried), reset rules (D1516), lock-order
  slot (D1517), 020/028 registrations (D1518), 016/029 amendments (D1519); gaps G1496, G1497. The items
  above were re-checked against the revised text.
