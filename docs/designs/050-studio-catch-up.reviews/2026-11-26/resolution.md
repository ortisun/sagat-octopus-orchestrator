# Resolution of the reviews of design 050 (2026-11-26)

Settled by the author under Owner decision 2026-11-24-4, on the approval rule of Owner decision 2026-11-25-2.

## Security review (`security.md`, at `5f90d452`)

All 10 findings are fixed; a security finding is never accepted (Owner decision 2026-11-25-2).

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: one statement per page with `tenant_id` in every join and no row lock; `an_inbox_page_is_one_statement` and its publication twin | Technical Plan 6, Tasks 050-4, 050-5 |
| 2 | Major | Fixed: every access filter, feature 029's `visibleFolders` included, is a predicate of the scan, so a cursor never names a hidden row; `a_cursor_never_names_a_row_in_a_hidden_folder`; Architecture decision 0144 decision 4 amended | Technical Plan 6, Architecture decision 0144 |
| 3 | Nit | Fixed: the adapter's ended-run guard is deleted, not remapped; `an_ineligible_caller_on_an_ended_run_gets_403_not_409` | Technical Plan 6, Task 050-4 |
| 4 | Nit | Fixed: run admission first, the query read after, `owner` bounded, an unknown owner answers an empty page, `no-store`; two tests | Technical Plan 5, Task 050-9 |
| 5 | Nit | Fixed: `not_built` additions in Task 050-2 and removals in Tasks 050-4, 050-5 and 050-9, with the iteration route's query string | Tasks |
| 6 | Nit | Fixed: the risk row and Architecture decision 0144's context say the inbox adds no disclosure beyond the definition and the journal | Risks, Architecture decision 0144 |
| 7 | Nit | Fixed: one grouped query per run | Technical Plan 5 |
| 8 | Nit | Fixed: the runbook checks `indisvalid` and rebuilds an invalid index | Technical Plan 6 |
| 9 | Nit | Fixed: runtime-role runs in Tasks 050-5 and 050-9; the inbox adapters take no row lock | Technical Plan 6, Tasks |
| 10 | Nit | Fixed: audit filters in component state, downloads labelled, `path()` refuses `.` and `..`, with tests | Technical Plan 7, Task 050-18 |

## egm-review Stage 7, structural lens (`structural_report.md`, at `c0d2adcd`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: five labelled paragraphs (Outcomes, Counts on the owner, The iteration route, The source, Drawing) | Technical Plan 5 |
| 2 | Major | Fixed: labelled paragraphs (Eligibility rule, Port and adapter, Ended runs, Paging and access filters, Per-run list, Run ends, Index, Items), and the publication paragraph the same way; the admission operation and target named | Technical Plan 6 |
| 3 | Nit | Fixed in part: the two long blocks use one device; the other sections keep their run-in labels. Settled: level-4 headings everywhere would renumber nothing but would move every cross-reference's target text; left for the next amendment | Technical Plans 3 to 8 |
| 4 | Nit | Settled: a rewrap of the whole feature range is a mechanical change kept out of the approval commit so that the reviewed diff stays readable; it waits for the next amendment of the design | The feature range |
| 5 | Nit | Fixed: Compensation, Reconciliation, Cross-tenant battery and Runtime role are entries of their own | Terms |
| 6 | Nit | Fixed: options and choice summarised | Decision 050-004 |
| 7 | Nit | Fixed: Task 050-1 changes the link to the relative one when #267 merges; until then the link names the branch the coordinator merges first | Task 050-1 |
| 8 | Nit | Fixed: the directory is linked and both resolutions named; "Architecture decision 0082" is the link text | Overview, Technical Plan 3 |

## egm-review style lens (`style_report.md`, at `4ac4065d`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the canvas shows a problem until the body has one `return`, and the draft still saves; the same words in all four places | Requirement 050-007, CUJ 1 |
| 2 | Major | Fixed: a MUST NOT list and a MUST list | Requirement 050-022 |
| 3 | Major | Fixed: "Intent" in Terms; "a changed body uses a new key, so it is never refused as a reused key"; the test renamed `a_resend_replays_and_a_changed_decision_gets_a_new_key`; "only the person resends" | Terms, Requirements 050-009, 050-014, Technical Plan 5 |
| 4 | Major | Fixed: the registry field is `requires: { capability } \| { driver: 'iceberg' } \| null`; "capability" means only a name of `GET /version` | Terms, Requirement 050-003, Technical Plan 4 |
| 5 | Major | Fixed: "inbox tab" and "browser tab", with the two tests named | Requirement 050-012, Technical Plan 6 |
| 6 | Major | Fixed in part: the two longest blocks (Technical Plans 5 and 6) are split into labelled paragraphs by the structural settlement. Settled: the requirements keep one paragraph each, because the repository's requirement form is one bullet with its acceptance, and every rule in them is restated, one per paragraph, in the Technical Plan they cite | Requirements 050-008, 050-010, 050-011 |
| 7 | Major | Fixed: the closed gate glossed, "Design 050 changes nothing in the registry", "the registry member" named, the repeated sentence deleted | Technical Plan 3 |
| 8 | Major | Fixed: "Owner node" in Terms; the validation delay is `useValidation`'s quiet delay; the frame budget is the browser's 50-millisecond long-task threshold; "at most five pages per list per refresh"; "Stage N review" defined in Open Questions | Terms, Requirements 050-003, 050-012, Non-functional requirement 050-002, Open Questions, Gap 050-006 |
| 9 | Nit | Settled: the first bullet already says the three rules in one sentence that the Technical Plan expands; left as is | Overview |
| 10 | Nit | Fixed: Open Questions rewritten with one form for the owner decisions | Open Questions |
| 11 | Nit | Fixed in part: the decision's adapter named by file. Settled: the run-end probe sentence names its test already | Technical Plan 6 |
| 12 | Nit | Fixed: the audit sentence rewritten, the loop events listed, "a fifth of these figures" | Technical Plans 6 and 7, Decision 050-012 |
| 13 | Nit | Fixed: "small" dropped, "the two products this design is compared with", the closing clause of Decision 050-001 removed. Settled: the assessment of 2026-11-11 is the coordinating session's comparison, kept as a dated reference | Overview, The Problem, Decision 050-001 |
| 14 | Nit | Fixed: `documentIO.ts` named | Technical Plan 4 |

While fixing finding 1 the author also corrected the acceptance of Requirement 050-006, which still read "validates it against the test server and publishes it" after the Stage 9 settlement moved publication to the journeys.

## egm-review Stage 8, Intermediate Zoom Out (`intermediate_zoom_out_report.md`, at `1b5b03cc`)

All five findings are accepted and fixed. Findings 1 and 2 remove work: no migration on the progress tables and no
change of any existing answer. The design is again read-only on the engine's tables, as Constitution Check I says.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the migration and the drain writes are removed. The outcomes map from `drain_kind`, `drain_iteration` and `drain_failure_code`, the runbook caveat is dropped, and Task 050-9 is read-only. Stage 9 Blocker 1's premise that no failed index was stored was wrong; its settlement is corrected here | Technical Plan 5, Task 050-9 |
| 2 | Major | Fixed: the probe is Task 050-4's first red test. The scan has a run-status predicate (a data filter, not the eligibility rule), and `may_decide` and `decide` keep their inputs. The adapter's ended-run guard moves after authorization and keeps its answer. No SDK change, no `RUN_ENDED`, no Gap 050-001 case. Security Nit 3 stays fixed by the move: an ineligible caller is refused for eligibility and never learns the run ended | Overview, Requirement 050-010, Technical Plan 6, Non-functional requirement 050-003, Gap 050-001, Task 050-4, Architecture decision 0144 |
| 3 | Nit | Fixed: one list, the Studio's JSON, held to `ALL`, with `AuditAction` built from one table as `failures!` does | Decision 050-006, Technical Plans 7 and 9, Tasks 050-2, 050-11 |
| 4 | Nit | Fixed: the refresh is built on `refetchInterval` with a function, as `BatchPage.tsx` uses. Task 050-14 measures latency at the bounded worst case (50 requests per second) and does not copy the schedule | Technical Plan 6, Task 050-14 |
| 5 | Nit | Fixed: the kind-level variant is marked as feature 022's, and `without_the_operators_capability_the_stage_list_is_read_only_and_kept` is added | Technical Plan 4, Task 050-15 |

## egm-review Stage 9, codebase details lens (`codebase_details_report.md`, at `19a633cc`)

All 20 findings are accepted and fixed; every Blocker is fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: six raising sites are named, each with how it learns the body's name and the node's index from the raw document. Body-level refusals use `/bodies/{name}`, and `resolveNodeId` handles both forms | Technical Plan 4 |
| 2 | Blocker | Fixed: the create-script route's `LANGUAGE_NOT_BUILT`, shown at the picker of `CreateScriptDialog.tsx`, and the test renamed | Requirement 050-019, Decision 050-007, Technical Plan 8, Task 050-12 |
| 3 | Blocker | Fixed: `PublishingService::effective_policy` in `publishing.rs`, made `pub(crate)` | Technical Plan 6 |
| 4 | Blocker | Fixed: the workflow capabilities of `infra-ir` added to the taxonomy, and a second shipped name `workflow.reset-reapply`. `reapplyOnReset` is read-only without it | Technical Plan 3, Requirement 050-002, Task 050-3 |
| 5 | Blocker | Fixed: the harness uses `PUT /quotas/iteration_concurrency`. The settings page shows the tenant quotas, and Success criterion 050-001 starts by setting the capacity | Task 050-19, Technical Plan 8, Success criterion 050-001 |
| 6 | Blocker | Fixed: only `ApprovalListItem` is closed | Non-functional requirement 050-003, Technical Plan 6 |
| 7 | Major | Fixed: `id` and `type` exempt, custom-editor fields declared, legacy fields listed, `allOf` resolved | Technical Plan 4 |
| 8 | Major | Fixed in part: Task 050-15 names the files it modifies and creates (`AddNode.tsx`, `NodeInspector.tsx`, `NodesTable.tsx`, `src/api/capabilities.ts`), and Task 050-14 extends the existing `perf.spec.ts`. Settled: the other tasks name new folders (`features/inbox/`, `features/audit/`, `features/settings/`, `features/events/`) or the files they change by name already, so no create-or-modify marker is added to each | Tasks 050-14, 050-15 |
| 9 | Major | Fixed: `perf.spec.ts` is extended, and its `STUDIO_PERF=1` command joins the verification. One report, `workflow-studio.md`, which Decision 050-001's condition also reads | Task 050-14 |
| 10 | Major | Fixed: the real arithmetic. The new journeys run as a second job selected with `SAGAT_STUDIO_GREP` | Task 050-19 |
| 11 | Major | Fixed: `audit-actions.json` is named, and the OpenAPI item is dropped | Requirement 050-013, Technical Plan 9 |
| 12 to 20 | Nit | Fixed: `SAGAT_CONTRACT_BASELINE_BLESS`; the loop's drain reasons; `entryNodeId`, the entry's graph id, replaces the internal UUID; a missing drivers route means no Iceberg; `access.denied.cross_tenant` is excluded from the picker; `requires` everywhere; the activation refusals handled; `retryAfterMs` and `endpoints.journal` reused; the lint rule and the tokens test said to arrive with #267. `chainPolicy` added to the refusals of the Loop row | Technical Plans 3 to 8, Tasks |

## Security re-read, round 1 (`approval/reread-1-security.md`, at `db1bea08`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: the sentence says reads collapse per caller and filter per five minutes | Technical Plan 7 |
