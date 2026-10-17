# Specification Quality Checklist: Per-field payload classification

**Purpose**: Validate design completeness before implementation planning.
**Created**: 2026-10-16. **Feature**: [spec.md](../spec.md).

## Content quality

- [x] User value and delivery boundary are explicit: what is built, what stays as ADR-0028 and ADR-0031 decided, and the one existing surface (the event inbox) that is not brought under classification.
- [x] Mandatory user stories, requirements, entities, success criteria, assumptions and out-of-scope are complete.
- [x] The specification describes observable behaviour; storage, adapters and source structure live in plan, data-model and contracts.
- [x] Existing behaviour is distinguished from new behaviour with evidence from the code (research baseline), not from memory.

## Requirement completeness

- [x] No unresolved clarification markers. The open product questions are listed with options and a recommendation (`research.md`, P1-P4), and none is hidden as an assumption.
- [x] All 25 functional and 6 non-functional requirements are testable and have task traceability.
- [x] Nine success criteria are measurable and name the suite that proves them.
- [x] Seven independently testable stories include the happy path, the refusal and the failure case.
- [x] Edge cases cover: size and complexity bounds, branch composition, undeclared data, empty values, Unicode and bidirectional text, concurrent reveals, grant and membership changes during a request, reruns, old documents read by old engines.
- [x] Dependencies are named: features 001, 002, 006, 007, 008, 009, 011, 012; ADR-0014, 0023, 0027, 0028, 0033; the unmerged G21 change and feature 014 are recorded as branch dependencies without links to files absent from this branch.
- [x] Checked against the code (2026-10-16): carriers that exist, carriers that do not, the permission catalog and `Permission::ALL` equal to Admin, one role per membership, no membership management surface, `getInboxEvent` and dead letters returning values, `payloadRef` as a plain digest, the SDK builders' `outputSchema` coverage, the closed `Error.code`, `DefinitionProblem` codes.

## Feature readiness

- [x] The grammar of the annotation, its inheritance, its composition rules and its refusals are exact enough to test (contracts section 1).
- [x] Every disclosure path has an audience, an audit rule, a bound and a refusal (contracts section 2, audience matrix).
- [x] Transaction boundaries, failure outcomes and the tenant boundary are specified (data-model sections 4-6).
- [x] The classification is pinned to the published version, including the interpretation (`classification_rules`).
- [x] Existing versions keep today's behaviour by construction (rules version 0), and the migration is additive.
- [x] OTel, audit, typed errors (reasons only, closed `Error.code`), tenant boundary and performance bounds are specified for the feature.
- [x] Five-language SDK scope is decided (setters and one golden, no helper) and justified.
- [x] Tasks are dependency-ordered, tests precede implementation, every task has file paths and Refs, and nothing is ticked.

## Open implementation gates

- [ ] The owner answers P1 (grant mechanism), P2 (event inbox), P3 (two readings of ADR-0033) and P4 (self-grant); T016-002.
- [ ] Add the schema and OpenAPI changes and prove them in CI (T016-005 to T016-008).
- [ ] Implement and prove the domain, the publication check, pinning, disclosure, reveal and grants (Phases 2 to 5).
- [ ] Prove the canary suite across every boundary, the fault suite, tenant isolation, compatibility and the five-SDK golden (T016-042, T016-049 to T016-052).
- [ ] Measure the bounds on a stated environment (T016-053) and write the runbook (T016-055).
- [ ] Merge the G21 run-result change and then deliver `source: "result"` (T016-031).

The checked items validate the written design only. The open gates are intentional future work and
state no result.
