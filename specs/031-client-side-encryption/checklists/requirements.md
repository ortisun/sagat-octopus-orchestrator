# Specification Quality Checklist: Client-side payload encryption

**Purpose**: Validate specification completeness and quality before proceeding to planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

Reviewed by the specification author on 2026-10-26; an owner review is still required (T031-001). `[x]` means the
criterion is met by the text, not that anything is implemented.

## Content Quality

- [x] No implementation details (languages, frameworks, APIs) — **Partially.** The user stories and success criteria
  are written as behaviour, but the functional requirements name wire shapes (annotation key, envelope members, REST
  paths, protocol fields) because they are contracts between six SDKs, a customer service and the platform. Crate and
  file placement is kept in `plan.md`. Accepted, as in features 016 and 017. Accepted, delegate (OD-2026-11-24-4): the
  wire shapes are the interoperability contract of six SDKs and customer codec servers, so leaving them out would make
  the requirements untestable; placement stays in the plan.
- [x] Focused on user value and business needs (regulated values unreadable by the platform operator).
- [x] Written for non-technical stakeholders — **Partially.** Context, threat model and stories are; the requirements
  target engineers by necessity. Accepted, delegate (OD-2026-11-24-4): the context, the threat model and the stories
  give a non-technical reader what is protected and what is not; the requirements are read by implementers and
  reviewers, who need the exact codes and bounds.
- [x] All mandatory sections completed.

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain (three genuine choices are owner questions O1520–O1522 with defaults).
- [x] Requirements are testable and unambiguous (each FR names its code, path or bound).
- [x] Success criteria are measurable.
- [x] Success criteria are technology-agnostic — **Partially.** SC-031-001..013 are outcomes, but name stores and the
  browser where the evidence is collected (PostgreSQL rows, Chromium), as the house style does. Accepted, delegate
  (OD-2026-11-24-4): a canary criterion is only provable against the named stores it is searched in; naming them makes
  the release-blocking criteria of NFR-031-SEC checkable, and the outcomes themselves stay technology-free.
- [x] All acceptance scenarios are defined (Given/When/Then for US01–US08).
- [x] Edge cases are identified.
- [x] Scope is clearly bounded (Out of Scope, threat model "not protected").
- [x] Dependencies and assumptions identified (existing features, new 018–032 features, the unmerged 015 draft).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (traced to SC-031-* and tasks).
- [x] User scenarios cover primary flows (declare, encrypt, rotate/fail, decode in Studio, classification, compression,
  CLI, inventory).
- [x] Feature meets measurable outcomes defined in Success Criteria (each FR maps to at least one SC and one task).
- [x] No implementation details leak into specification — see the first item; accepted deviation. Accepted, delegate
  (OD-2026-11-24-4): the only detail named in the specification is the wire contract, for the reason of the first item;
  re-checked on 2026-11-24 after the readiness corrections, which added requirements (FR-031-037, FR-031-038) in the
  same style and no crate placement.

## Traceability spot check

| FR | SC | Tasks |
|---|---|---|
| FR-031-001..005 | SC-031-002, SC-031-014 | T031-003, T031-016, T031-017, T031-066 |
| FR-031-006 (registry, C7) | SC-031-002 | T031-003, T031-020 |
| FR-031-007, 008 | SC-031-002 | T031-018, T031-019 |
| FR-031-009..013 | SC-031-001, 006, 014 | T031-015, T031-022..029, T031-055, T031-066, T031-069 |
| FR-031-014 | SC-031-001 | T031-015, T031-059 |
| FR-031-015 | SC-031-013 | T031-028, T031-030 |
| FR-031-035 | SC-031-001 | T031-067 |
| FR-031-036 | SC-031-013 | T031-068 |
| FR-031-016..022 | SC-031-003, 004, 005 | T031-033..039, T031-042, T031-056 |
| FR-031-023..027 | SC-031-007, 008, 011 | T031-047..054, T031-057 |
| FR-031-028 | SC-031-008 | T031-045, T031-046 |
| FR-031-029 | SC-031-006 | T031-058 |
| FR-031-030..032 | SC-031-009, 010 | T031-031, T031-032 |
| FR-031-033, 034 | SC-031-011, 012 | T031-005..009, T031-061..065 |
| FR-031-037 (043, added 2026-11-24) | SC-031-015 | T031-071 |
| FR-031-038 (041, added 2026-11-24) | SC-031-004 | T031-072, T031-073 |
| FR-031-016..022 for Rust and the worker host (2026-11-24) | SC-031-003, 004, 005 | T031-070, T031-073 |

## Notes

- The three unchecked items are deliberate house-style deviations, not defects to fix before planning.
  Corrected 2026-11-24: the four items that were unchecked are accepted under the delegation of Owner decision
  2026-11-24-4, which satisfies the checklist gate of Owner decision 2026-11-03-33; each item says why.
- ADR-0066 is Proposed; implementation waits for the owner (T031-001). Corrected 2026-11-24: ADR-0066 is revised and
  Accepted and O1520 to O1522 are decided, delegate (OD-2026-11-24-4).
- Revised on 2026-10-27 after the independent architecture and security reviews: customer-authenticated codec access
  (O1521 default changed), envelope context binding, the location rule across documents, decode-plan gating, fleet
  activation, side channels and the cross-draft registry. The quality items above were re-checked; no new
  [NEEDS CLARIFICATION] marker was introduced.
