# Specification Quality Checklist: Developer tooling (028)

**Purpose**: Validate specification completeness and quality before planning and owner review
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — **partly.** Following the house style of features 014
  and 017, the spec names contracts the requirements depend on (`/v1/testing/`, the `INTERCEPTOR_FAILED` code, GitHub
  and GitLab signature headers, RFC 8785, Ed25519, `SAGAT_ENV=dev`). Crate placement, SQL and data structures are kept
  in plan.md and data-model.md.
- [x] Focused on user value and business needs (each story states who benefits and why its priority).
- [ ] Written for non-technical stakeholders — **no.** The readers are platform engineers and SDK authors; the stories
  are plain, the requirements are technical.
- [x] All mandatory sections completed (scenarios, edge cases, requirements, key entities, success criteria,
  assumptions, dependencies, out of scope, owner questions).

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain; the three genuine owner choices are O1400–O1402 with recommended
  defaults.
- [x] Requirements are testable and unambiguous: each FR names an observable behaviour and is traced from a task
  (tasks.md) and a success criterion.
- [x] Success criteria are measurable (counts, equalities, zero findings, refusal before work); the one duration
  (SC-028-001) is a benchmark criterion to verify on a stated environment, not a promise.
- [ ] Success criteria are technology-agnostic — **partly**: they name SDKs, providers and the editor because those are
  the subject of the feature.
- [x] All acceptance scenarios are defined (Given/When/Then, happy and failure paths per story).
- [x] Edge cases are identified (clock, interceptors, replay, Git, catalog, editor).
- [x] Scope is clearly bounded (out-of-scope list; ownership of 020, 022 and 031 respected; no marketplace).
- [x] Dependencies and assumptions identified (existing features, new features, provider versions).

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (scenarios plus SC mapping).
- [x] User scenarios cover primary flows (test, intercept, replay, edit, sync, import).
- [ ] Feature meets measurable outcomes defined in Success Criteria — **not applicable yet**: nothing is implemented.
- [ ] No implementation details leak into the specification — see the first item.

## Constitution and house rules

- [x] Tenant boundary, typed errors, OTel, audit and finite ceilings stated (FR-028-043/044, plan tables).
- [x] Five-SDK conformance for every SDK-visible behaviour (FR-028-009, 018, 024).
- [x] "Never in production" for virtual time is a requirement with automated checks (FR-028-002, SC-028-002).
- [x] Credentials as OpenBao-backed references, never in logs, responses or commits (FR-028-037, NFR-028-SEC).
- [x] Publication semantics of ADR-0018 kept, with the stated weakening for imported drafts and its compensation
  (policy + 1 approvals, verified branch review; FR-028-034, D1432); imports create drafts (D1424).
- [x] One payload-transforming SDK stage, owned by feature 031; 028 interceptors only observe and decorate (FR-028-021,
  decision C5).
- [x] Time classes of every draft published in one catalog (contracts/time-classes.md, decision C13).
- [x] The constitution's exclusion of a connector marketplace addressed (FR-028-039, D1434, O1402).
- [x] Rule-9 failure tests planned: crash points, duplicates, timeout/signal race, lost delivery (lost webhook), failed
  compensation (shifted-clock saga suites), restart, tenant isolation, ceiling exhaustion.

## Notes

- Review round of 2026-10-27 (architecture and security reviews, coordinator decisions C3, C5, C13, C16, C20, C21)
  applied; see the report of that round for each finding.

- US03 is blocked on feature 020's history export format (G1404); its requirements are complete but cannot be
  implemented before that format is accepted.
- The time-class refactor touches every feature that stores an instant; ADR-0063 is proposed for owner acceptance.
