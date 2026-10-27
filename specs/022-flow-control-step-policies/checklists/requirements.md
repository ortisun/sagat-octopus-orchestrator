# Specification Quality Checklist: Flow control and step policies

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

Filled by the specification author (agent) on 2026-10-26 and revised on 2026-10-27 after the independent
architecture review (022 S1–S6 and minors, 018 B1, 020 S8); a reviewer still owns the final marks. An
unchecked item states why.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs)
  — Not satisfied by design: following the house style of features 014 and 017, requirements name the IR
  fields, REST routes, journal events and existing tables they extend (`runs.result_digest`,
  `activity_quota_limits`), because the contracts are the product surface here. Languages, crates and SQL
  appear only in plan.md and data-model.md.
- [x] Focused on user value and business needs (each story states why and for whom)
- [ ] Written for non-technical stakeholders
  — Partly: user stories and "Why this priority" are plain; the functional requirements target engineers
  and reviewers of a durable-execution platform.
- [x] All mandatory sections completed (user scenarios, edge cases, requirements, key entities, success
  criteria, assumptions; plus dependencies, out of scope, owner questions)

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain (three genuine owner choices are O1160–O1162 with
  recommended defaults in research.md)
- [x] Requirements are testable and unambiguous (each FR names its observable outcome and refusal code;
  each is traced to at least one SC and one task)
- [x] Success criteria are measurable (counts, orderings, rounds, zero-leak and bound tests)
- [ ] Success criteria are technology-agnostic (no implementation details)
  — Partly: several criteria are phrased in platform terms (lease, journal event, tenant battery) because
  the outcome a user observes is exactly that durable evidence.
- [x] All acceptance scenarios are defined (Given/When/Then for US01–US06, happy and failure)
- [x] Edge cases are identified
- [x] Scope is clearly bounded (Out of Scope; ownership boundaries with 018, 019, 020, 021, 026, 028, 031)
- [x] Dependencies and assumptions identified

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (FR → SC mapping in spec.md; FR → task
  mapping in tasks.md)
- [x] User scenarios cover primary flows (cache, concurrency, failure handling, early return, standalone,
  draft test)
- [ ] Feature meets measurable outcomes defined in Success Criteria
  — Cannot be true for a draft: nothing is implemented; every SC is future evidence.
- [ ] No implementation details leak into specification — see the first item.

## Traceability spot check (author)

| FR | SC | Tasks |
|---|---|---|
| FR-022-001..007 | SC-022-001, 002 | T022-004, 006, 015–020 |
| FR-022-008..011 | SC-022-007 | T022-035–038 |
| FR-022-012..016 | SC-022-005, 006 | T022-030–034 |
| FR-022-017..022 | SC-022-003, 004, 010, 015 | T022-004, 006, 021–029, 064 |
| FR-022-023..025 | SC-022-008 | T022-039–041 |
| FR-022-026..030 | SC-022-009, 015 | T022-042–047 |
| FR-022-031..036 | SC-022-010..014 | T022-005, 006, 011–014, 048–062, 065 |
| FR-022-037 | SC-022-016 | T022-016, 063 |
| FR-022-038 | SC-022-015 | T022-008, 064 |

## Notes

- Items marked incomplete above are deliberate deviations or draft-state facts, not open spec defects.
- The plan's Constitution Check marks the design only; it is not approval of an implementation.
