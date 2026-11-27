# egm-review, Stage 9 (codebase details lens) of design 050 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 19a633cc
Reviewer: egm-review (Stage 9, egm-codebase-details)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: condensed by the author to one paragraph per finding, every severity, location and fix kept. The persona counts any false statement about current behaviour as a Blocker (finding 6 is one, a one-word fix). The reviewer confirmed some forty other code claims, among them the kinds and registry entries, the legacy messages, the drain columns, the snapshot and graph functions, the ended-run guard, the grants, the admission operations, the tenant budget, the reused Studio helpers, the routes, the targets, `not_built` and the npm scripts.
Counts: Blocker 6, Major 5, Nit 9

1. **Blocker. Technical Plan 4, Task 050-6, Requirement 050-003, CUJ 1.** The body scope is not "set by the body conversion path": `approval_in_a_body` (`convert.rs:735-755`) and `operators::check_surfaces` (`operators.rs:55-63`) are raw pre-scans over `bodies.values()` that drop the name; `check_expressions` runs after conversion (`lib.rs:378-379`); body-shape refusals name a whole body (`convert.rs:832-846`, `1028-1042`); `CollectionOperatorsDisabled` is raised in `definitions.rs:1263-1266`; `EscapingBodyEdge` already writes `/bodies/{body}` (`definitions.rs:421-425`). Fix: name each raising site and how it learns the name and index; `/bodies/{name}` for body-level refusals; `resolveNodeId` handles that form.
2. **Blocker. Requirement 050-019, Decision 050-007, Technical Plan 8, Task 050-12.** `LANGUAGE_NOT_BUILT` comes from `ScriptService::create` (`scripts.rs:957-959`), not the build route. Fix: the create-script route, shown in `CreateScriptDialog.tsx`.
3. **Blocker. Technical Plan 6.** `crates/application/src/versioning.rs` does not exist; the resolution is the private `PublishingService::effective_policy` (`publishing.rs:403-429`). Fix: cite it and make it reachable.
4. **Blocker. Technical Plan 3, Decision 050-002, Technical Plan 4.** The taxonomy leaves out the workflow capabilities of `crates/infra-ir/src/capabilities.rs` (Architecture decision 0032 decision 8), whose `SAGAT_RESET_REAPPLY_ENABLED` decides whether publication accepts `reapplyOnReset`, a field the editor edits. Fix: the fourth sense, a second shipped name, the field read-only without it.
5. **Blocker. Task 050-19, Success criterion 050-001, Requirement 050-017, Technical Plan 8.** "No route records one" is false: `PUT /quotas/iteration_concurrency` writes `tenant_collection_policies` (`quotas.rs:969-1003`), and while unset a collection admits nothing; the settings page covers only namespace quotas. Fix: use the route; the tenant quotas on the settings page, or the precondition stated.
6. **Blocker. Non-functional requirement 050-003, Technical Plan 6.** `PublicationRequest` is not a closed schema (`openapi.yaml:7901-7903`); only `ApprovalListItem` is. Fix: one word.
7. **Major. Requirement 050-001, Technical Plan 4, Task 050-15.** The drift test cannot pass as written: `database` has no fields (edited by `DatabaseFields`) though its member requires three, `humanApproval` requires `formSchema`, every member requires `id` and `type`, and the legacy members declare `type` and `required` in `allOf`. Fix: exempt `id` and `type`, let entries declare custom-editor fields, list the legacy fields, resolve `allOf`.
8. **Major. The plan.** Files are not marked created or modified, and existing ones are missing (`AddNode.tsx` with `Palette` and `AddNodeDialog`, `NodeInspector.tsx`, `NodesTable.tsx`, the existing `perf.spec.ts`). Fix: mark each path; add them.
9. **Major. Task 050-14, Decision 050-001.** `perf.spec.ts` already benchmarks a run of about 10,000 nodes under `STUDIO_PERF=1` and reports through `scripts/perf-report.mjs` to `tests/performance/reports/workflow-studio.md`; the verification never runs it, and three report locations differ. Fix: extend it, add its command, one report.
10. **Major. Task 050-19.** The job already declares 14 journeys at 240 seconds (56 minutes) against its 60-minute cap; the new journeys would exceed it. Fix: the real arithmetic; a split job or `SAGAT_STUDIO_GREP`.
11. **Major. Requirement 050-013, Technical Plan 9.** The OpenAPI audit list still appears against Decision 050-006. Fix: name `audit-actions.json`; drop the item.
12. **Nit.** `SAGAT_CONTRACT_BLESS` does not exist; reuse `SAGAT_CONTRACT_BASELINE_BLESS`.
13. **Nit.** A loop's drains name no iteration for a timeout or a cancellation; the Loop row of the subset table omits the refused `chainPolicy`.
14. **Nit.** `entryInstanceId` is an internal UUID nothing reads. Fix: the entry's graph id, or drop it.
15. **Nit.** The drivers route is mounted only with the connections service. Fix: `404 ROUTE_NOT_FOUND` means no Iceberg.
16. **Nit.** The picker would suggest `access.denied.cross_tenant`, which `GET /audit` never returns. Fix: exclude it.
17. **Nit.** The field is `requires` in Terms but `capability` in Technical Plans 1 and 4 and Task 050-15.
18. **Nit.** The versions page does not handle `409 BREAKING_CHANGE` with `acceptBreaking` or `ACTIVE_VERSION_CHANGED`.
19. **Nit.** Reuse `ApiError.retryAfterMs` and `endpoints.journal`.
20. **Nit.** The ESLint boundary rule and the tokens test arrive with pull request #267.
