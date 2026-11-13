## What this proposes

This dossier proposes reviewing 12 unchecked task entries across three inventories, identifying their remaining scope, evidence, dependencies and acceptance boundaries. It preserves historical task wording and partial results; it does not authorize implementation, publication, performance acceptance or checkbox changes. The work includes unfinished workflow primitives, Redis integration validation, SDK distribution and genuine human evaluation, with duplicate human-acceptance entries retained for reconciliation.

## How the existing system works, per this document

The linked baseline describes a Rust modular monolith whose durable execution authority is PostgreSQL; state changes, scheduling obligations and applicable outbox records share documented transaction boundaries. Workers execute separately through the canonical protocol, while NATS and optional Redis provide notifications or callback transport without replacing PostgreSQL authority. Selected collection bodies, loops, approvals, declared operations and continuation chains have documented implementation and evidence, while general nesting, capacity-group activation, production ceilings and final release acceptance remain open. Six SDKs share server semantics and common conformance, and the owner-selected distribution channel is GitHub Releases.

## COMP-001 — Major — Registry publication is described as current blocked work despite the recorded distribution decision

**Scope:** FeatureRange, `tasks/reviews/2026-10-30-open-tasks/review_target.md:57`; DetailedRange, `review_target.md:151–155`; **OT-011 / FR-006-008**, originating at `tasks/todo.md:71`.

The overview calls this “public SDK registry publication,” and OT-011 says publication awaits registry identities. However, `docs/owner-decisions.md:49` explicitly selects GitHub Releases only and excludes npm, PyPI, NuGet and Maven publication for now; `docs/owner-decisions.md:26` reiterates that boundary. `docs/runbooks/sdk-packaging.md:41–43` says registry steps are retained for a later decision and are not run. The feature requirement itself, `specs/006-sdk-multilanguage/spec.md:73`, requires versioned packages, a runtime matrix and examples, without requiring those registries.

This is more than an outdated evidence count: it changes what a newcomer understands the remaining action and dependency to be. The preserved task appears to require obtaining registry credentials, while the governing decision defers that publication. The conflict also persists in the requirement-level row at `docs/traceability.md:194`, despite the newer distribution summary at `docs/traceability.md:3`.

**Actionable clarification:** Retain the original entry as historical source text, but annotate it as superseded or deferred by the recorded GitHub-only decision. State separately any remaining verification of approved release artifacts, immutable versions, provenance or consumer installation. Align the overview and requirement-level traceability with that distinction; do not infer either registry authorization or task completion.

## COMP-002 — Major — The human-evaluation documents give contradictory instructions about whether the diagnosis threshold exists

**Scope:** DetailedRange, `tasks/reviews/2026-10-30-open-tasks/review_target.md:149` and `:161`; **OT-010 and OT-012 / SC-008-001, SC-012-001**. Supporting conflict: `docs/validation/human-acceptance-results.template.md:55–56` and `:111`.

OT-010 still requests agreement on the numeric UX target. More consequentially, the linked results template declares `N = PENDING OWNER DECISION` and instructs the observer to use `NOT EVALUABLE — N UNSET`. Yet `docs/owner-decisions.md:40`, `specs/008-observability-operations/spec.md:91` and `docs/validation/human-acceptance.md:11–13` agree that **N = 10**, meaning at most **9 counted interactions**.

A newcomer following the prescribed protocol and copying its prescribed template receives incompatible instructions for judging the same session. The remaining human evidence is understandable, but the dossier incorrectly presents a settled decision as an outstanding dependency. The repeated task identifiers are not themselves a defect: the dossier explicitly preserves both source locations.

**Actionable clarification:** Annotate OT-010 with the existing threshold and align the template’s threshold and comparison fields with it. Preserve `NOT RUN` until genuine sessions occur. Explain that OT-010 and OT-012 share the same acceptance evidence, while keeping the diagnosis criterion distinct from newcomer onboarding: SC-012-001 concerns unaided TypeScript and C# first runs, and its separate timing target is already recorded in `specs/012-developer-experience/spec.md:88`.

## Coverage

- **OT-001:** No finding under comprehension. The task, clause-level evidence and remaining nested/low-quota obligations are understandable through the linked specification and research.
- **OT-002:** No finding under comprehension. Logical charging, terminal drain and ordered aggregation have defined context.
- **OT-003:** No finding under comprehension. Captured membership, blob provenance, scoped identity and publication boundaries are explained by the linked documents.
- **OT-004:** No finding under comprehension. The successive compatibility increments are distinguishable from complete fault, compensation and release acceptance.
- **OT-005:** No finding under comprehension. Historical fault results are explicitly evidence for a checkpoint, not current-tree acceptance.
- **OT-006:** No finding under comprehension. The linked performance report distinguishes earlier defects, subsequent measurements and still-unaccepted production ceilings.
- **OT-007:** No finding under comprehension. The old CI checkpoint does not establish the final integrated release gate.
- **OT-008:** No finding under comprehension. Requirement-to-evidence maintenance and the restriction to proven public shapes are understandable.
- **OT-009:** No finding under comprehension. The Redis transport, released-process cutover evidence and remaining final release-tree validation are distinguished.
- **OT-010:** COMP-002.
- **OT-011:** COMP-001.
- **OT-012:** COMP-002, through the results template required by its existing protocol.

## Read evidence

Fully read the following review instructions and inputs:

- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/agents/comprehension.md`
- `/Users/marcelo/.codex/plugins/cache/egm/egm/0.1.0/docs/codex.md`
- `tasks/reviews/2026-10-30-open-tasks/review_target.md`, all 165 lines, covering both assigned ranges.
- `tasks/reviews/2026-10-30-open-tasks/execution_manifest.json`.

Fully read all 39 files in the mandatory baseline reading list:

1. `tasks/reviews/2026-10-30-open-tasks/inventory.json`
2. `.specify/memory/constitution.md`
3. `AGENTS.md`
4. `specs/AGENTS.md`
5. `SPECIFICATION-INDEX.md`
6. `docs/traceability.md`
7. `docs/owner-decisions.md`
8. `docs/test-strategy.md`
9. `docs/sdk-compatibility.md`
10. `docs/runbooks/sdk-packaging.md`
11. `specs/014-workflow-primitives/spec.md`
12. `specs/014-workflow-primitives/plan.md`
13. `specs/014-workflow-primitives/tasks.md`
14. `specs/014-workflow-primitives/research.md`
15. `specs/014-workflow-primitives/validation-report.md`
16. `specs/033-redis-streams/spec.md`
17. `specs/033-redis-streams/plan.md`
18. `specs/033-redis-streams/tasks.md`
19. `specs/033-redis-streams/research.md`
20. `specs/033-redis-streams/validation-report.md`
21. `specs/006-sdk-multilanguage/spec.md`
22. `specs/006-sdk-multilanguage/plan.md`
23. `specs/006-sdk-multilanguage/tasks.md`
24. `specs/006-sdk-multilanguage/research.md`
25. `specs/008-observability-operations/spec.md`
26. `specs/008-observability-operations/plan.md`
27. `specs/008-observability-operations/tasks.md`
28. `specs/008-observability-operations/research.md`
29. `specs/012-developer-experience/spec.md`
30. `specs/012-developer-experience/plan.md`
31. `specs/012-developer-experience/tasks.md`
32. `specs/012-developer-experience/research.md`
33. `docs/validation/human-acceptance-results.template.md`
34. `docs/validation/human-acceptance.md`
35. `specs/014-workflow-primitives/evidence/released-v02-20261027/README.md`
36. `specs/014-workflow-primitives/evidence/released-v02-inflight-20261027/README.md`
37. `specs/014-workflow-primitives/evidence/released-v02-applied-20261027/README.md`
38. `specs/033-redis-streams/released-callback-cutover.md`
39. `tasks/todo.md`

Additional reference reading:

- Fully read `docs/releases/0.2.0.md`.
- Read the opening implementation plan and initial checkpoints, plus the final combined integration and CI-diagnosis sections of `specs/014-workflow-primitives/implementation-ledger.md`—lines 1–220 and 1590–1678.
- Read the conclusions and environment, then the scheduler corrections and subsequent diagnostic/runtime-role measurements in `tests/performance/reports/workflow-primitives.md`—lines 1–85 and 1128–1438; also inspected its heading and status index.

## Missing reads and limitations

No mandatory baseline file was missing or inaccessible, and no mandatory full-file read remains incomplete. The additional implementation ledger and performance report were read selectively, as identified above.

Historical raw logs, external CI runs, sibling worktrees, executable implementation/test files and every transitive reference were not inspected. In particular, the validation report describes historical `tasks/project-completion/` evidence as preserved outside this checkout; its underlying artifacts were not independently verified. All recorded test counts remain documentary claims in this review.

This was an independent comprehension review only. No implementation document standard was applied, no prior stage report was consulted, and no files, Git state or external services were changed. No builds or tests were run.
