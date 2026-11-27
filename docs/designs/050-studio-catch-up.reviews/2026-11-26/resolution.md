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

## Architecture re-read, round 1 (`approval/reread-1-architecture.md`, at `dabbc4bc`)

All eight findings are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: 028's contract refers to design 050's field; the four names are `testing.virtual-clock`, `testing.history-replay`, `tooling.git-sync`, `tooling.template-catalog`; Decision 028-008 records the correction; Technical Plan 3 lists all four | `specs/028-developer-tooling/`, Technical Plan 3 |
| 2 | Major | Fixed: the port returns raw rows and the stored policy; the use case resolves the policy once per page by a pure function extracted from `effective_policy` and shared with the recount; Architecture decision 0144 decision 6 says so | Technical Plan 6, Architecture decision 0144 |
| 3 | Nit | Fixed: the clause deleted | Non-Functional Requirements |
| 4 | Nit | Fixed: no run states, two migrations, Architecture decision 0018 decision 4 in "Refines" | Architecture decision 0144 |
| 5 | Nit | Fixed: decision 4 describes the add-here action and cites Gap 050-005 | Architecture decision 0143 |
| 6 | Nit | Fixed: `AppState` keeps a capability source read on each request | Technical Plan 3 |
| 7 | Nit | Fixed: the request's principal; no second read | Technical Plan 6 |
| 8 | Nit | Fixed: the bound counts rows that pass the predicates; the work is bounded by the namespace's pending rows; the histogram's meaning stated | Technical Plan 6 |

## Correctness re-read, round 1 (`approval/reread-1-correctness.md`, at `5d451eb9`)

All five findings are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: a full page's cursor is its last item; otherwise the last row scanned; the publication list has the same `limit` and rule; `a_full_page_resumes_after_its_last_item` with a twin; Architecture decision 0144 decision 5 | Technical Plan 6, Task 050-4 |
| 2 | Major | Fixed: the use case flags `status == PENDING && !run_ended && may_decide`; two tests; Architecture decision 0144 decision 2 | Technical Plan 6, Task 050-4 |
| 3 | Nit | Fixed: the row is seeded | Task 050-4 |
| 4 | Nit | Fixed: named as the one deliberate change, in the requirement and the decision record | Non-functional requirement 050-003, Architecture decision 0144 |
| 5 | Nit | Fixed: the `drain_iteration` rule first; the counts asserted mid-drain | Technical Plan 5, Task 050-9 |

## egm-review re-read, round 1 (`approval/reread-1-egm-review.md`, at `9c893782`)

All ten findings are fixed.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the body loop of `structured_definition_with_policy` is the seventh site; the return problem is the server's; a test with a colliding entrypoint id | Technical Plan 4, Task 050-6 |
| 2 | Major | Fixed: the new journeys are tagged `@catch-up`, run by a second job and excluded from the first with `SAGAT_STUDIO_GREP_INVERT`; the list verified | Task 050-19 |
| 3 | Major | Fixed: the guard reads the status captured before `decide`; `an_eligible_caller_on_an_ended_run_still_gets_503` | Technical Plan 6, Task 050-4 |
| 4 | Major | Fixed (the stronger option): a page examines at most 2,000 rows in an inner query before any predicate; the cursor is sealed by the deployment's `KeyRing` with the caller as associated data; the histogram counts examined rows; two tests and a performance case; Architecture decision 0144 decisions 4 and 5 | Technical Plan 6, Risks, Metrics, Architecture decision 0144 |
| 5 to 10 | Nit | Fixed: the adapter's `effective_policy` routed through the pure function; the report script's second input; the sentence rephrased; the seven run-end paths listed; the codec returns its index map; the name derived from `reset_reapply_enabled()` | Technical Plans 3 to 6, Tasks 050-4, 050-14 |

## Security re-read, round 3 (`approval/reread-3-security.md`, at `2d71be9c`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: a purpose key derived by HKDF-SHA256 from the ring's active key, labels `approval-inbox-cursor` and `publication-inbox-cursor` | Technical Plan 6, Architecture decision 0144 decision 5 |
| 2 | Nit | Fixed: `503` without a ring and never an unsealed cursor; 512 bytes; one `400`; two tests | Technical Plan 6, Task 050-4 |
| 3 | Nit | Fixed: the count of hidden rows per window is stated as the one aggregate disclosed, under Requirement 029-017 | Technical Plan 6, Architecture decision 0144 decision 4, Risks |

## Security re-read, round 4 (`approval/reread-4-security.md`, at `86b9f140`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: AES-256-GCM-SIV (no random-nonce bound) and the tenant in the HKDF `info`, so each tenant's cursors have their own key | Technical Plan 6, Architecture decision 0144 |
| 2 | Nit | Fixed: `KeyRing::derive_purpose_key(label, info)` named, zeroized and redacted; `a_cursor_does_not_open_under_the_secrets_key` | Technical Plan 6, Task 050-4 |
| 3 | Nit | Fixed: the publication twins, `an_approval_cursor_does_not_open_on_the_publication_list` and `an_oversized_cursor_is_refused_before_decoding` | Tasks 050-4, 050-5 |

## Security re-read, round 5 (`approval/reread-5-security.md`, at `10338cd5`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: the publication twins of both tests in Task 050-5 | Task 050-5 |

The security re-read of round 6 (`approval/reread-6-security.md`, at `4dea6eaa`) found nothing. The shared
`KeyRing::derive_purpose_key` gained a key id at `013`'s round 7; design 050's text follows (a cursor under a retired
key gets the same `400` and the Studio starts from the first page).

The security re-read of round 7 (`approval/reread-7-security.md`, at `9a19411f`) found nothing.

The security re-read of round 8 (`approval/reread-8-security.md`, at `dcfd91b1`) found nothing.

The security re-read at `db9e8a50` found nothing in design 050.

## Security re-read, round 9 (`approval/reread-9-security.md`, at `0aa18b84`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: Task 013-8 named as the other owner | Technical Plan 6 |

The security re-read of round 10 (`approval/reread-10-security.md`, at `9a760bd7`) found nothing.

## Architecture re-read, round 2 (`approval/reread-2-architecture.md`, at `239577a1`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the port `InboxCursorSealer` in the application, its adapter in `infra-secrets` with `aes-gcm-siv`, wired in the control plane and the harness; no new dependency edge | Technical Plan 6, Task 050-4 |
| 2 | Major | Fixed: Requirement 050-010 is conditional on the key ring; `KEY_RING_NOT_CONFIGURED` with its remediation; the Studio shows the inbox unavailable and stops refreshing; the runbook line; the harness configured with a ring; Architecture decision 0144's Consequences record the dependency and why a dedicated ring was rejected; a test | Requirement 050-010, Technical Plans 6 and 9, Task 050-4, Architecture decision 0144 |
| 3 | Nit | Fixed: Task 050-4 lists the derivation and a closed enumeration of purpose labels, unless Task 013-8 landed them | Task 050-4 |
| 4 | Nit | Fixed: four tests in "Enforced by" | Architecture decision 0144 |
| 5 | Nit | Fixed: `inbox_rows_scanned{list}` | Technical Plan 6 |

## Correctness re-read, round 2 (`approval/reread-2-correctness.md`, at `051e5f47`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: the three paging twins in Task 050-5 | Task 050-5 |
| 2 | Nit | Fixed: `retryable: false`; the inbox reads `details.reason` before `retryable` and stops the refresh and the focus refetch; two tests | Technical Plan 6, Task 050-10 |
| 3 | Nit | Fixed: only the two inbox labels; `cli-refresh` stays in design 013's `IdentityKeyRing` | Task 050-4 |
| 4 | Nit | Fixed: 2,000 seeded ended-run rows ahead of a live row | Task 050-4 |

## egm-review re-read, round 2 (`approval/reread-2-egm-review.md`, at `e1537935`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed (the reviewer's second option, chosen for the architectural advantage): the cursor key is a deployment-wide key kept in PostgreSQL (`inbox_cursor_keys`), created at the control plane's first start, rotated by `control-plane inbox-cursor-key rotate`; it hides only row keys, which whoever reads the table can read anyway, so the inbox needs neither OpenBao nor `SAGAT_SECRETS_KEYS` and works on every deployment; `KEY_RING_NOT_CONFIGURED` is gone; Architecture decision 0144's Consequences say so | Requirement 050-010, Technical Plans 6 and 9, Task 050-4, Architecture decision 0144 |
| 2 | Major | Fixed by removal: design 050 no longer uses `KeyRing` or `derive_purpose_key`; design 013 owns the function alone | Technical Plan 6, Task 050-4 |
| 3 | Nit | Fixed: `<key id>.<nonce>.<ciphertext>` in base64url; `open` derives from the cursor's own key id | Technical Plan 6 |
| 4 | Nit | Fixed by removal: there is no ring check | Technical Plan 6 |
| 5 | Nit | Fixed: AES-256-GCM (`aes-gcm`) and HKDF-SHA256 (`hkdf`), both already in the lock file, so no new crate; a Complexity Tracking row for the sealed cursor | Technical Plan 6, Complexity Tracking |
| 6 | Nit | Fixed: the requirement's sentence restored; a `400` ends the refresh, shows what it collected, and the next starts from the first page; the cursor text split into "Cursor sealing" and "Cursor refusals" | Requirement 050-010, Technical Plan 6 |

## Security re-read, round 11 (`approval/reread-11-security.md`, at `e28824eb`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Nit | Fixed: `control-plane migrate` creates the first key and `inbox-cursor-key rotate` replaces it, both with the schema-owning role; the runtime role has `SELECT` only | Technical Plan 6, Task 050-4, Architecture decision 0144 |
| 2 | Nit | Fixed: the arithmetic (2^32 cursors at 100 a second is about 1.36 years) and a yearly rotation in the runbook | Technical Plans 6 and 9 |
| 3 | Nit | Fixed: the bytes are redacted in `Debug`, never logged or printed (`rotate` prints only the id), the table never exported; the test renamed `the_cursor_key_is_created_by_migrate_and_never_printed` | Technical Plan 6, Task 050-4 |

## Architecture re-read, round 3 (`approval/reread-3-architecture.md`, at `d8a16e9a`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: a fallible port (`SealerError::Unavailable` answers `503`); the adapter loads at start, re-reads every minute and once on an unknown id, outside any transaction, keeping cached keys on a failed read; an empty table answers `503`; a cross-replica test | Technical Plan 6, Task 050-4 |
| 2 to 5 | Nit | Fixed: the first key created by `sagat_infra_postgres::migrate` under a singleton guard; `rotate --operator` recorded in the operator history; `aes-gcm` in the workspace dependencies and `hkdf` 0.13 in `infra-postgres`; Architecture decision 0144 option 5 and two tests in "Enforced by"; the test renamed for a dropped key; the Risks row reworded | Technical Plan 6, Task 050-4, Risks, Architecture decision 0144 |
