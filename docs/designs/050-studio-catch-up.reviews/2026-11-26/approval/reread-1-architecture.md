# Targeted architecture re-read of design 050 for approval, round 1 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: dabbc4bc
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a re-read of the text changed since the reviewer's full review of 2026-11-25 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 2, Nit 6

1. **Major. `specs/028-developer-tooling/contracts/errors.md`, `spec.md`, `contracts/test-environment-sdk.md`; Technical Plan 3.** Feature 028's contract still declares a closed, unprefixed `enum` of four names (two not covered by Decision 028-008), against Architecture decision 0143 decision 2 and the design's pattern. Fix: refer to design 050's field, prefix all four, list them in Technical Plan 3.
2. **Major. Technical Plan 6, Architecture decision 0144 decision 6.** The publication port is said to return `approvalsRequiredNow` by `PublishingService::effective_policy`, a private application method an adapter cannot call; SQL would copy the policy rule. Fix: raw rows from the port; the use case resolves the policy once per page by a pure function shared with publication's recount.
3. **Nit. Non-functional Requirements.** A stale promise of an SDK ended-run test. Fix: delete it.
4. **Nit. Architecture decision 0144.** "Run states" in the property test, one migration where there are two, and Architecture decision 0018 decision 4 missing from "Refines". Fix: correct all three.
5. **Nit. Architecture decision 0143 decision 4.** It describes a mapping dialog the design no longer builds. Fix: the add-here action and Gap 050-005.
6. **Nit. Technical Plan 3.** The capability list is fixed at start-up while registry-derived names change at runtime. Fix: a source read on each request.
7. **Nit. Technical Plan 6.** Two sources for the caller's principal. Fix: the request's principal.
8. **Nit. Technical Plan 6.** The 2,000-row bound limits rows that pass the predicates, not rows the index walks; the histogram's meaning is unclear. Fix: state what is bounded and counted.
