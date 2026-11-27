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
  **Accepted as a recorded deviation on 2026-11-24, delegate (OD-2026-11-24-4).** Options weighed: (a)
  accept; (b) strip operation, field and rpc names from the requirements into the contracts only. (b) would
  leave two texts that say the same thing in different words, which is how a requirement and its contract
  drift apart, against the rule that the document leads and the code follows (Owner decision 2026-11-20-8);
  every requirement here is checked by a contract test that needs those names. Crates, frameworks and
  languages stay out of the requirements (they are in `plan.md`), which is what the item protects. The
  item stays unticked because it is not met; it no longer blocks implementation.
- [x] Focused on user value and business needs (eight user stories with their reason for priority).
- [x] Written for non-technical stakeholders — *partly* when first reviewed: user stories and success
  criteria were; the functional requirements are written for reviewers of the contracts. **Rejected on
  2026-11-24 and fixed, delegate (OD-2026-11-24-4)**: options weighed were (a) accept the partial state,
  (b) rewrite the requirements in plain language, (c) add a plain-language section that a non-technical
  reader can stop at. (b) loses the precision the contract tests need; (a) leaves the owner and buyers
  without a readable entry. (c) was done: `spec.md`, "In plain words", says what the feature does and never
  does without a contract term; the item is met.
- [x] All mandatory sections completed (scenarios, requirements, entities, success criteria,
  assumptions, dependencies, out of scope, owner questions).

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain (three owner questions O1320–O1322 carry recommended
  defaults instead).
- [x] Requirements are testable and unambiguous (each FR names its observable behaviour, refusal or
  ceiling; ceilings are in plan.md).
- [x] Success criteria are measurable (counts, 100 % agreement, zero duplicates, named refusals).
- [ ] Success criteria are technology-agnostic — *partly*: several name HTTP statuses and SDK languages,
  which are the contract under test. **Accepted as a recorded deviation on 2026-11-24, delegate
  (OD-2026-11-24-4).** Options weighed: (a) accept; (b) restate each criterion without statuses and
  languages. The statuses are the observable outcome a caller depends on (`200` against `202` is the whole
  point of User story 026-02) and the six languages are the promise of constitution principle IV; a
  criterion without them could pass while the product fails. The item stays unticked because it is not
  met; it no longer blocks implementation.
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
- [ ] No implementation details leak into specification — *see the first item*. **Accepted as a recorded
  deviation on 2026-11-24, delegate (OD-2026-11-24-4)**, for the reasons of the first item; the
  re-baseline of 2026-11-24 kept crate and table names in `plan.md` and `data-model.md`, and the few that the
  requirements now cite (`runs.public_link_id`, `event_deliveries`, the storage class) are named because a
  contract test reads them. It no longer blocks implementation.

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
- 2026-11-24, delegate (OD-2026-11-24-4): of the four items unticked until then, three are accepted as recorded
  deviations (they stay unticked and no longer block implementation, the gate of Owner decision 2026-11-03-33) and one
  was rejected and fixed by the new section "In plain words" of `spec.md` (ticked). Architecture decisions 0061 and
  0068 were accepted with amendments and the three owner questions answered under the same delegation (research.md);
  the coordinator records the rows in `docs/owner-decisions.md`. Re-read after the re-baseline: every requirement,
  the two new ones included (051 by Task 026-078 and Success criterion 026-020, 052 by Tasks 026-069 and
  026-075 and Success criterion 026-021), is traced by a task and a success criterion (78 tasks, 21 success
  criteria); the Constitution and house rules items still hold, with the transaction table grown by T4b.
- Re-reviewed by the author on 2026-10-26 after the independent architecture and security reviews and
  the coordinator decisions C1–C22: every FR is still traced by a task and an SC (68 tasks, 19 SCs).
