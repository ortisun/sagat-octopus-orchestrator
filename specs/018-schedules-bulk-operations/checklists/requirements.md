# Specification Quality Checklist: Schedules and bulk run operations

**Purpose**: Validate specification completeness and quality before planning
**Created**: 2026-10-26
**Feature**: [spec.md](../spec.md)

**Review Ownership**: drafted by the specifying agent; `[x]` records the author's self-review only.
Owner review is still required (O1000–O1002, ADR-0050 status Proposed). `[x]` means the
requirements-quality criterion was checked, not that anything is implemented.

## Content Quality

- [ ] No implementation details (languages, frameworks, APIs) — **Partly not met, deliberately.** As in
  features 014 and 017, the spec names existing contracts (idempotency scopes, journal events, error
  reasons, run fields) because compatibility with them is the requirement. Crate and table placement
  live in `plan.md` and `data-model.md`, not in the FRs.
- [x] Focused on user value and business needs (each story states who acts and why).
- [ ] Written for non-technical stakeholders — **Not met, deliberately.** The audience is platform
  operators and implementers; the stories are readable on their own, the FRs are technical.
- [x] All mandatory sections completed (stories, edge cases, FR, NFR, key entities, SC, assumptions,
  dependencies, out of scope, owner questions).

## Requirement Completeness

- [x] No [NEEDS CLARIFICATION] markers remain; three genuine choices are owner questions with
  recommended defaults that the plan implements.
- [x] Requirements are testable and unambiguous (each FR names its observable outcome and reason code).
- [x] Success criteria are measurable (each SC is an automated proof with a pass condition).
- [ ] Success criteria are technology-agnostic — **Partly not met**: SC-018-002/009/013 name PostgreSQL,
  the lock monitor and contract baselines, because those are the evidence the constitution requires.
- [x] All acceptance scenarios are defined (Given/When/Then for US01–US07, including failure cases).
- [x] Edge cases are identified (DST, jitter, races, crashes, quotas, stale reads).
- [x] Scope is clearly bounded (out-of-scope list; ownership boundaries with 019, 020, 022, 026, 028, 029).
- [x] Cross-feature contracts with drafts 020 (`RunSelector`, search attributes, purge, erasure) and 022
  (run permits through T10, test runs) stated as dependencies (FR-018-050/051), after the 2026-10-27 review.
- [x] Dependencies and assumptions identified.

## Feature Readiness

- [x] All functional requirements have clear acceptance criteria (traced to SC and tasks; see the
  traceability check below).
- [x] User scenarios cover primary flows (schedules, operation, business ids, with-start, batch, reset,
  trigger migration).
- [x] Feature meets measurable outcomes defined in Success Criteria (every FR is in at least one SC row).
- [ ] No implementation details leak into specification — see the first item.

## Architecture conformance (house rules)

- [x] One-transaction rule stated per operation (plan transaction table; FR-018-005, -021, -030, -037).
- [x] Terminal runs never return to running; reset and rerun open new runs (FR-018-037, -038; ADR-0050).
- [x] No buffered signal (FR-018-024, -039; ADR-0030 decision 17 kept).
- [x] Shard fence and database clock for pass-driven work (FR-018-005, -023; plan).
- [x] Tenant boundary, opaque `NOT_FOUND`, fail-closed authority (FR-018-015, -030, -041).
- [x] Typed errors with existing codes only (FR-018-043; contracts/errors.md).
- [x] OTel, fixed-label metrics, audit in the decision's transaction (FR-018-044; plan).
- [x] Finite ceilings with defaults and hard limits (FR-018-045; plan table) — values are design
  values pending the benchmark (G1005).
- [x] Additive contracts; worker protocol unchanged; one capability-gated IR field (FR-018-046;
  contracts/journal-and-events.md, contracts/workflow-ir.fragment.schema.json).
- [x] Five-SDK conformance scenarios (FR-018-048; contracts/sdk-conformance.md).
- [x] Rule-9 failure tests planned (plan failure matrix → tasks).
- [x] Cross-draft registries owned here are populated: reset-rule registry (C1,
  `contracts/journal-and-events.md`) and the merged admission lock slots (C4, ADR-0050 decision 7);
  known deviations of other drafts recorded as G1605 (2026-10-27, round 2); the instance-capability
  registry used by 029 and 031 is specified here (FR-018-052).

## Traceability check (performed 2026-10-26)

Every FR-018-001..052 appears in at least one task (`tasks.md`) and in at least one SC row (re-run
2026-10-27 after the architecture review). Every SC-018-001..017
is named by at least one task's evidence.

## Notes

- Items left unchecked are deliberate deviations shared with the house style of 014/017, not defects
  to fix before planning.
- Nothing in this feature is implemented; no test has been run.
