# Specification Quality Checklist: Additional script languages

**Purpose**: Validate completeness and quality of the specification before planning and implementation
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)
**Filled by**: the specification author (agent), 2026-10-26. Items are the author's assessment; the owner
review in T023-001 may reopen any of them. `[x]` means the requirement-quality criterion is met, not that
anything is implemented.

**Settled 2026-11-24**: every item left open was accepted or rejected under the owner's delegation,
delegate (OD-2026-11-24-4), by the agent preparing the feature for build. An item accepted as a stated
deviation is ticked and says "accepted as a stated deviation": the criterion is not met, and the owner's
delegate has judged the deviation right for this feature. An item rejected was fixed in the specification
and is ticked because it is now met. The owner may reopen any of them.

## Content Quality

- [x] No implementation details (languages, frameworks, APIs) — **deliberately not met.** As in features
  005 and 017, the requirements name interpreter flags, artifact formats, image boundaries and contract
  fields, because the security boundary (constitution V) and compatibility (IV) are testable only at that
  level. User stories and success criteria remain readable outcomes. Accepted as a stated deviation,
  delegate (OD-2026-11-24-4): removing the details would leave principle V untestable (a reviewer cannot
  check "isolated" without the flags, mounts and formats that make it so), and the plain-language summary
  added for the next item gives non-technical readers the outcome without them.
- [x] Focused on user value and business needs (authors, operators, security administrators, auditors).
- [x] Written for non-technical stakeholders — partly: user stories are; requirements are for reviewers of
  the sandbox and the contracts. Rejected as it stood, delegate (OD-2026-11-24-4), and fixed: the
  specification now opens with "In short", a plain-language summary of what the feature gives and what it
  does not change, as the rule "documentation first, written for people" (Owner decision 2026-11-20-8)
  asks; the requirements stay precise for the reviewers of the sandbox.
- [x] All mandatory sections completed (stories, edge cases, FR, NFR, key entities, SC, assumptions,
  dependencies, out of scope, owner questions).

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain; the three genuine owner choices are O1200–O1202 with
  recommended defaults the spec already assumes.
- [x] Requirements are testable and unambiguous (each FR-023 is traced by at least one task and one SC).
- [x] Success criteria are measurable (pass rates, refusal rates, presence/absence of canaries, verified
  envelopes, published reports with environment).
- [x] Success criteria are technology-agnostic — not fully: SC-023-004/005/006 name ELF, CycloneDX, DSSE and
  CI architectures, which are the evidence the constitution asks for. Accepted as a stated deviation,
  delegate (OD-2026-11-24-4): principle VIII asks for verified contracts, and a criterion that said
  "artifacts are well formed" instead of naming the ELF, CycloneDX and DSSE checks could not be measured;
  the business outcome of each criterion is stated in its first words.
- [x] All acceptance scenarios are defined (US01–US07, Given/When/Then, failure scenarios).
- [x] Edge cases are identified (14, including pool misconfiguration, moved tags, catalog change, policy
  races, encrypted and secret fields).
- [x] Scope is clearly bounded (out of scope; ownership boundaries with 019, 022, 021, 031, 028).
- [x] Dependencies and assumptions identified (existing 001, 002, 005, 007, 008, 009, 016; new 019, 031, 028).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (SC table maps FR ranges).
- [x] User scenarios cover primary flows (author, operator, security, governance, evaluation).
- [x] Feature meets measurable outcomes defined in Success Criteria (as a design; no evidence exists yet).
- [x] No implementation details leak into specification — see the first item. Accepted as a stated
  deviation with the first item, delegate (OD-2026-11-24-4), for the same reason.

## Constitution and house-style checks

- [x] Tenant boundary, RBAC (`script:policy:manage` admin-only), secret redaction and egress specified.
- [x] Versioning, additive contracts, migration with refusing downgrade, and compatibility with runs in
  flight defined.
- [x] Durable semantics: no new state machine; build completion transaction named; at-least-once stated.
- [x] Finite ceilings table with defaults and hard ceilings (plan.md); zero is not unlimited.
- [x] Failure tests of AGENTS.md rule 9 planned: crash at critical commit points, duplicate delivery, lost
  broker hint, timeout/cancellation race, restart, tenant isolation, quota exhaustion. Failed compensation
  is not applicable (the feature performs no compensable effect of its own; scripts' effects remain the
  author's, at least once).
- [x] Unproven guarantees marked pending (G1206, G1207, G1216, G1218; no SLSA level; no benchmark figure).
- [x] Five-SDK impact and conformance scenario planned (T023-050).
- [x] Studio: tokens, WCAG AA, no drawing of unavailable data (FR-007-010) planned.

## Gates before implementation (open)

- [x] Owner acceptance of ADR-0057 and answers to O1200–O1202 (T023-001). Settled 2026-11-24, delegate
  (OD-2026-11-24-4): Architecture decision 0057 accepted with an amendment (its status block), Owner
  questions 1200 to 1202 answered (research section 6). The row of `docs/owner-decisions.md` is added by
  the coordinating session.
- [x] Coordinator integration into `docs/adr/README.md`, `docs/roadmap.md`, `SPECIFICATION-INDEX.md` and
  `docs/traceability.md` (not done by this author by rule). Accepted 2026-11-24, delegate
  (OD-2026-11-24-4), as done to the extent it can be before the build: `docs/adr/README.md` (row 0057),
  `docs/roadmap.md` (row 23) and `SPECIFICATION-INDEX.md` (row 023 and the host-call registry) list the
  feature on `develop` at `2c94064f`; the coordinator changes "Proposed" to "Accepted" in the first two;
  `docs/traceability.md` gets its rows with the evidence (Task 023-060), because a traceability row
  without a test would be a claim.
- [x] Migration version allocated against the other parallel features (G1213). Accepted 2026-11-24,
  delegate (OD-2026-11-24-4): the migrations of every local and remote branch were listed on 2026-11-24 (newest on `develop` `20270119000000`,
  newest on any branch `20270209000100`) and `20270323000000` and `20270323000100` are proposed (research
  Gap 1213); the coordinator confirms the reservation and the builder checks again before slice A1
  merges.

## Notes

- Independent architecture review (2026-10-27, no blocking findings): S1 (proc-macro wording), S2
  (provenance per build key), S3 (language policy at lease, coordinator decision), both minor findings
  (five policy points; binary-only rollback) and 024's S3 (`python-data` mechanism, coordinator decision)
  applied as D1226–D1230, G1219–G1221, FR-023-027, SC-023-014, T023-061..063.
- Cross-feature review, coordinator decision C17 (2026-10-27): reserved host calls `checkpoint` and
  `stream.append` applied as FR-023-028, SC-023-015, D1231–D1233, G1222–G1224, T023-064..066 and ADR-0057
  decision 9. C21 does not apply (no new app). The overflow range D1625–D1629/G1625–G1629 was not needed.

- Currency check, 2026-11-24, delegate (OD-2026-11-24-4): the drafts were re-read against `develop` at
  `2c94064f`; stale items and new decisions are research section 1a, Decisions 1234 to 1239, 1625 to
  1627, Gaps 1225 to 1230, Requirements 023-029 and 023-030, Success criterion 023-016, Tasks 023-067 to
  023-071 and the build slices of `plan.md`. The overflow range is now partly used (Decisions 1625 to
  1627); Decisions 1628 and 1629 and Gaps 1625 to 1629 remain free for this feature.
- Cited paths, requirement ids (FR-005-006, FR-005-010, FR-007-010, FR-014-016, FR-016-002), 005 gaps
  (G97–G119 subset) and ADRs (0002, 0004, 0006, 0013, 0017, 0021, 0024, 0027, 0032, 0033) were checked
  in the worktree on 2026-10-26.
