# egm-review, Stage 5 (Initial Zoom Out) of design 050 (2026-11-25)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 035b2bf0
Reviewer: egm-review (Stage 5, egm-zoom-out)
EGM-hash: 003679ced8d0c6ffa103b14109aa8f8bab83aaf817a3e15479e122fc0f1683f9
Note: the reviewer's output, condensed by the author to one paragraph per finding; every finding, severity, location and fix is kept. The reviewer read the design at the branch tip, whose only later commit changed design 013, and confirmed that a new reader can tell what the design is for and why (context sufficiency passes).

Counts: Blocker 0, Major 6, Nit 9

1. **Major. Overview, Requirements 050-013 to 050-021, Decision 050-001 and Task 050-1, the plan.** One design carries at least five deliveries: the root problem calls for the registry and editor, the run view for loops and switches, and approvals, but the design adds the audit, versions, settings and events pages, the palette, the run family and the download (justified by parity), the acceptance of ADR-0101 with its budget and performance conditions (feature 007's pull request #267), and three server corrections; with 22 requirements, a small later change sends the foundation back to draft. Fix: keep Requirements 050-001 to 050-012 and 050-022; move 050-013 to 050-021 to a second design "Studio operator surfaces"; move Task 050-1 and Decision 050-001's conditions to #267, with 050 depending on it.

2. **Major. Requirement 050-002, Technical Plan 3, Task 050-3.** The capability list is built for features not on `develop`: design 050 uses only static names, yet it pre-registers about 25 names of features 013, 022, 027, 028 and 029, adds registry-derived names with a mapping table, and changes `Member` in `crates/application/src/instances.rs`, whose rules (ADR-0082) were costly to settle; the registry `Capability` enum on `develop` has no `flow-control-*` variant and nothing client-visible reads `access-narrowing-v1`. Fix: ship only the names 050 uses, computed at start-up; keep Decision 050-002 as the rule; let feature 022 or 029, whichever first needs a registry-derived name, add the derivation, the member change and its tests under its own review; no `x-known-values` for unbuilt features.

3. **Major. Technical Plan 4, Requirement 050-003, Task 050-6.** `DefinitionProblem.body` duplicates the JSON pointer problems already carry (`Problem::at_path`); a body name and an id cannot place a node whose id is missing or duplicated. Fix: the codec sets `path` to `/bodies/{name}/nodes/{index}`; extend `NODE_POINTER` and `resolveNodeId` in `validationModel.ts`; no new field.

4. **Major. Requirements 050-009 and 050-010, Technical Plans 5 and 6, Task 050-4.** The new routes add filters existing routes cover: the `runId` filter (own index path, cursor kind, cross-cursor refusal, namespace check) duplicates `GET /runs/{runId}/approvals`; the publication `workflowId` filter duplicates `GET /workflows/{id}/publication-requests`; the approvals `workflowId` filter has no consumer. Fix: add `youMayDecide` (from the same `may_decide`) to the items of the two existing per-run and per-workflow lists (a flag about the caller's own eligibility reveals no policy, initiator or decider); drop the filters, the second cursor kind and their tests; the namespace routes only serve the inbox.

5. **Major. Requirement 050-005, Technical Plan 4, Decision 050-003, Complexity Tracking, Task 050-7.** The guided replacement dialog (schema mapping, out-of-range prompts, a structural move of a legacy body subgraph) has no evidence that such drafts exist, and the simpler option (keep the node read-only and show its old values next to an "add here" action) is not weighed. Fix: keep Requirement 050-004; replace 050-005 with a read-only inspector of the legacy values beside an "Add a For each / Approval here" action; defer the mapping dialog until drafts are counted, recorded as a gap.

6. **Major. Technical Plan 5 (Iterations), Requirement 050-008.** The run view writes the engine's iteration-status rule again in TypeScript, whereas the engine already decides iteration outcomes (`CollectionDrainReason::Failed`, `LoopDrainReason::Failed`, `crates/domain/src/execution/event.rs`, `state.rs`); the argument of Decision 050-009 and ADR-0016 applies; grouping client-side also needs the whole unpaged graph of a 10,000-iteration run. Fix: the runtime graph's owner node reports each iteration's status and counts by status (an additive projection field from recorded outcomes); the Studio groups and draws; otherwise state the client rule as display only, a gap for drift and a test tying it to the engine.

7. **Nit. Technical Plan 3, Decision 050-007.** The drivers list already advertises Iceberg only when `lake.enabled` ("the Studio decides what to offer from this list", `routes/database.rs`), and the codec already refuses collection operators at the node (`COLLECTION_OPERATORS_DISABLED`). Fix: read the drivers list or retire it, not both; avoid two advertisements of one switch.

8. **Nit. Requirement 050-019, Decision 050-007, Technical Plan 8.** `Language::is_built` is a `const fn`, true for all five; feature 023's languages need Studio changes anyway (label, Monaco mode, sample). Fix: set `built: true` for Go and Java and let `LANGUAGE_NOT_BUILT` handle another build; or drop the re-read after a refused build.

9. **Nit. Technical Plan 3, ADR-0143 decision 2.** "Capability" names three things (the registry `Capability`, the workflow switches of `crates/infra-ir/src/capabilities.rs`, the new advertisement), and the design and ADR-0143 disagree on when the requirement set is read. Fix: one sentence mapping the three, perhaps another type name; align the wording; moot if finding 2 is applied.

10. **Nit. Requirements 050-012, 050-013, 050-021, Technical Plans 5 to 7.** Three hand-written page-following loops with their own bounds, and a fixed client pace nobody measured; `collectPages` exists in `api/endpoints.ts` without a bound. Fix: one bounded variant (pages, items or bytes; `Retry-After`; reports an early stop) used in all three; drop the fixed pace.

11. **Nit. Requirements 050-009 and 050-014, Technical Plan 5.** The key rules restate a pattern the Studio already uses in six places (`crypto.randomUUID()` and `options.key`). Fix: use the existing per-intent key pattern, ideally as one hook; drop the key format.

12. **Nit. Technical Plans 1, 4 and 5.** Existing primitives are not named: `useWindowedRows` (`features/runs/useWindow.ts`) already virtualizes 10,000 rows, `JsonField` exists. Fix: name them.

13. **Nit. Requirements 050-009, 050-010, 050-014, 050-021.** Requirements carry mechanics (key reuse, cursor key, scan bounds, pace, byte budgets), so tuning reopens them after approval. Fix: observable behaviour in requirements, numbers in the Technical Plan.

14. **Nit. Technical Plan 8 against the plan.** The versions page has two locations (`features/workflows/VersionsPanel.tsx` and `features/versions/`). Fix: one.

15. **Nit. Technical Plan 6.** "A path found not to cancel its pending approvals is fixed in the same delivery" has no bound. Fix: keep the test, record a wrong path as a follow-up task or name the known paths.
