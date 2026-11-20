# Specification Quality Checklist: Invocation and human-interaction surfaces

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

**Review ownership**: filled by the specification author (an agent) on 2026-10-26 as a self-review; it
is not the owner's review. `[x]` means the author judged the criterion met; notes explain each
judgement that is arguable.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — *not met by design*: house style
  (features 014, 016, 017) names the REST operations, IR fields and worker rpc in the requirements
  because the contracts are the product here; framework and crate placement are kept in plan.md.
- [x] Focused on user value and business needs (eight user stories with their reason for priority).
- [ ] Written for non-technical stakeholders — *partly*: user stories and success criteria are; the
  functional requirements are written for reviewers of the contracts.
- [x] All mandatory sections completed (scenarios, requirements, entities, success criteria,
  assumptions, dependencies, out of scope, owner questions).

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain (three owner questions O1320–O1322 carry recommended
  defaults instead).
- [x] Requirements are testable and unambiguous (each FR names its observable behaviour, refusal or
  ceiling; ceilings are in plan.md).
- [x] Success criteria are measurable (counts, 100 % agreement, zero duplicates, named refusals).
- [ ] Success criteria are technology-agnostic — *partly*: several name HTTP statuses and SDK languages,
  which are the contract under test.
- [x] All acceptance scenarios are defined (Given/When/Then per story, including failure paths).
- [x] Edge cases are identified (races, restarts, lost hints, forwarded links, erasure boundary,
  encrypted fields).
- [x] Scope is clearly bounded (out-of-scope list; ownership boundaries with 021, 022, 025, 027, 031).
- [x] Dependencies and assumptions identified (existing features, parallel drafts, Teams retirement).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (every FR is traced by a task and an
  SC; see tasks.md "Traces" and SC-026-001..019).
- [x] User scenarios cover primary flows (start, call-and-wait, follow, share, decide, notify, describe,
  decide in chat).
- [x] Feature meets measurable outcomes defined in Success Criteria (as a design; no evidence exists).
- [ ] No implementation details leak into specification — *see the first item*.

## Constitution and house rules (added by the author)

- [x] Transaction boundaries stated (plan T1–T11); terminal runs never reopen; first commit wins.
- [x] Anonymous and link-carried access fails closed, is rate-limited, audited and under quota (ADR-0061).
- [x] Approvals keep feature-014 semantics; links never authorize.
- [x] Outbound sends use the ADR-0025 pipeline; at-least-once and duplicates stated.
- [x] Disclosure declared and classified everywhere a value is shown (feature 016, ADR-0061 row).
- [x] Tenant boundary, typed errors, OTel, audit, finite ceilings, five-SDK parity, Studio rules,
  rule-9 failure tests planned.
- [x] Nothing presented as implemented; placeholders and dependency hooks marked (G1327, G1328).

## Notes

- The three unchecked items are deliberate house-style deviations, recorded rather than hidden.
- Owner review of ADR-0061, ADR-0068 and O1320–O1322 (T026-001) is required before implementation.
- Re-reviewed by the author on 2026-10-26 after the independent architecture and security reviews and
  the coordinator decisions C1–C22: every FR is still traced by a task and an SC (68 tasks, 19 SCs).
