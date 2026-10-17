# Specification Quality Checklist: Per-field payload classification

**Purpose**: Validate design completeness before implementation planning.
**Created**: 2026-10-16. **Feature**: [spec.md](../spec.md).

## Content quality

- [x] User value and delivery boundary are explicit: what is built, what stays as ADR-0028 and ADR-0031 decided, and the one existing surface that changes (the event inbox and dead letters: values restricted now, classification later), with the compatibility impact stated.
- [x] Mandatory user stories, requirements, entities, success criteria, assumptions and out-of-scope are complete.
- [x] The specification describes observable behaviour; storage, adapters and source structure live in plan, data-model and contracts.
- [x] Existing behaviour is distinguished from new behaviour with evidence from the code (research baseline), not from memory.

## Requirement completeness

- [x] No unresolved clarification markers. The four product questions were answered by the owner on 2026-10-17 and are recorded with their options and D-ids (`research.md`, "Product decisions answered 2026-10-17"); what remains undecided is listed at the end of `research.md`.
- [x] All 29 functional and 6 non-functional requirements are testable and have task traceability.
- [x] Ten success criteria are measurable and name the suite that proves them.
- [x] Eight independently testable stories include the happy path, the refusal and the failure case.
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
- [x] Tasks are dependency-ordered, tests precede implementation, every task has file paths and Refs; implementation ticks cite executed evidence in the canonical task list.

## Open implementation gates

- [x] The owner answers P1 (grant mechanism), P2 (event inbox), P3 (two readings of ADR-0033) and P4 (self-grant); T016-002. _Answered 2026-10-17; recorded in `research.md`, `docs/owner-decisions.md` and the ADR-0033 addendum._
- [x] Canonical schema/OpenAPI and generated Studio types are implemented and locally verified (T016-005..008; 18 contract tests and generated-client/reference checks). Remote CI is not claimed.
- [x] Domain, publication, pinning, disclosure, reveal and grants have executed unit/real-PG/REST/fault evidence (Phases 2..5; see tasks and integration notes).
- [x] Event values are restricted and the response break documented (US08): original event gates 2 contract / 11 e2e / 111 integration tests, plus reveal/audit/200-letter/tenant-boundary cases.
- [x] Canary boundaries, actual CLI/Chromium, transaction/process faults, tenant isolation, frozen-server byte compatibility and five-SDK builder/released-client evidence passed (T016-042, 049..052; manifests and limits in integration notes).
- [x] Registered benchmark passed with environment, response sizes, allocations/cache policy and limits recorded; runbook delivered (T016-053, 055).
- [x] G21 result/child carriers are integrated and actual worker/script/child/rerun/retry/blob/result scenarios passed (T016-031).
- [ ] Complete the integrated workspace/architecture/Clippy/contracts/advisory/Compose release gates (T016-054 and shared feature-014 gates).

Content/readiness items above validate the written design. Implementation marks cite actual local
execution in tasks.md and the integration notes; the remaining release gate is still open. Nothing
here claims remote CI, deployment or human visual acceptance.
