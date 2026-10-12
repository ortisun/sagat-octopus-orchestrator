# Tasks: Visual Studio, Monaco and runtime graph

**Input:** `specs/007-workflow-studio/spec.md`, `plan.md`, `research.md`, `data-model.md`, contracts from 001.
**Format:** `[ID] [P?] [US?] Description with exact paths`. Run each phase's gates sequentially.

## Phase 1: Setup / contracts
- [x] T007-001 Review FR-007-001..FR-007-008, SCs and ADRs; record gaps in `specs/007-workflow-studio/research.md`. — gaps G142–G159 and decisions D138–D153 in `research.md`; the decision record is ADR-0016 (`docs/adr/`).
- [x] T007-002 [P] Define fixtures and contract validation in `tests/contract/workflow-studio/fixtures/`. — there is no `fixtures/` directory: the documents are built inline by the tests, and the contract of the new operations on their normal and error paths is `tests/contract/workflow-studio/rest/conformance.rs` (one test, every status and schema of validate, diff, the graph changes and the stream).
- [x] T007-003 [P] Document types, transitions and invariants in `specs/007-workflow-studio/data-model.md`. — `data-model.md` (the document, the run model, the cursor and its invariants, the thresholds of D147).

## Phase 2: Foundation
- [x] T007-004 Create the necessary domain contracts/interfaces in `frontend/apps/studio/src/features/workflows/WorkflowCanvas.tsx` and invariant tests. — the node registry and the document model are `frontend/apps/studio/src/model/{registry,ir,edit,layout,references,advice}.ts` and the canvas is `features/workflows/WorkflowCanvas.tsx` (with `canvasModel.ts`); 845 Vitest tests in 62 files, among them `edit.test.ts`, `references.test.ts`, `canvasModel.test.ts` and `WorkflowEditor.*.test.tsx`.
- [x] T007-005 Implement ports/use cases in the appropriate module based on `specs/007-workflow-studio/plan.md`. — server side: `DefinitionService::{validate, diff}` (`crates/application/src/definitions.rs`, `diff.rs`), `RunService::graph_changes` (`runs.rs`), `RunEvent::changed_nodes` (domain), the comparison in `crates/infra-ir/src/diff.rs`, the routes and streams of `crates/api-http` (`routes/execution.rs`, `streams.rs`, `studio.rs`, `server.rs`); `definition_service.rs` (39 tests), `infra-ir/tests/diff.rs` (18).
- [x] T007-006 Write additive migrations and rollback/forward tests in `crates/infra-postgres/migrations/`. — no migration: the feature adds no table or column (the journal, the steps and the memberships it reads are 001–003's).
- [x] T007-007 Create PostgreSQL integration and tenant isolation tests in `tests/integration/workflow-studio/`. — there is no `tests/integration/workflow-studio/`: the use cases are played against a real PostgreSQL by `tests/e2e/workflow-studio/` (25 tests) and the tenant boundary is asserted in `us01`, `us03`, `fault_workflow_studio` and the `SEC` journey.

## Phase 3: US01 — Create a workflow without programming (P1)
**Independent test:** Given an author in the Studio; When they drag activity/condition/parallel/saga and configure parameters; Then the validator displays errors and saves the IR draft.
- [x] T007-008 [P] [US01] Write a Given/When/Then test for creating a workflow without programming in `tests/e2e/workflow-studio/us01.rs`. — `tests/e2e/workflow-studio/us01.rs` (7 tests: a draft is saved whatever its state, validated with every problem, compared with its version, two people cannot overwrite each other) and the Playwright journey `us01-author.spec.ts` (2).
- [x] T007-009 [US01] Implement the domain path and error handling for creating a workflow without programming in `frontend/apps/studio/src/features/workflows/WorkflowCanvas.tsx`. — the canvas, the list view, the inspector, the palette and the publish dialog of `frontend/apps/studio/src/features/workflows/` (`WorkflowCanvas.tsx`, `WorkflowEditor.tsx`, `NodeInspector.tsx`, `PublishDialog.tsx`).
- [x] T007-010 [US01] Integrate adapters and persistence for scenario US01 in `crates/application/` and `crates/infra-postgres/`. — `crates/application/src/definitions.rs` and the codec `crates/infra-ir`; nothing is persisted that was not already (drafts and versions are 001's).
- [x] T007-011 [P] [US01] Add logs/traces/metrics and tenant/secret protection to scenario US01 in `crates/telemetry/`. — what the platform already gives every route (`protect`: trace id, request metrics, typed errors with the trace id) applies to validate and diff; secrets and payloads are in no answer (`findings_never_repeat_a_value_of_the_document…`). **Open: the browser app sends no telemetry (G159).**
- [x] T007-012 [US01] Run and document duplicate, timeout, worker failure and tenant boundary tests for US01 in `tests/fault/workflow-studio/`. — `us01_two_people_editing_the_same_draft…`, `us01_validating_needs_write_access_and_never_crosses_a_tenant` (404), the hostile document tests of `infra-ir/tests/diff.rs` and `definition_service.rs` (a draft full of triggers or of artifacts is compared in linear time and the answer is bounded), and the journey `US01 two people editing the same draft`; `tests/fault/workflow-studio/` holds the faults of the decisions (US04).

## Phase 4: US02 — Edit code in a node (P1)
**Independent test:** Given a selected Script node; When the user edits TS/Python/C# and tests; Then Monaco shows the result in a sandbox and the version is only published after validation.
- [x] T007-013 [P] [US02] Write a Given/When/Then test for editing code in a node in `tests/e2e/workflow-studio/us02.rs`. — `tests/e2e/workflow-studio/us02.rs` (2 tests: a script node whose artifact was never built is a problem on its node and is refused at publish; a malformed script document is a schema problem) and the journey `us02-script.spec.ts`, which builds, tests in the sandbox and publishes a script from the browser and uses it in a workflow.
- [x] T007-014 [US02] Implement the domain path and error handling for editing code in a node in `frontend/apps/studio/src/features/workflows/NodeInspector.tsx`. — not in `NodeInspector.tsx`: the code is edited on the Scripts page (`features/scripts/`: `ScriptEditorPage`, `MonacoEditor`, `TestRun`, `PublishDialog`) and a script node pins a build through `ScriptPicker.tsx`; there is no link from the node to the editor (G157).
- [x] T007-015 [US02] Integrate adapters and persistence for scenario US02 in `crates/application/` and `crates/infra-postgres/`. — the existing script APIs of 005 and the validation of `UNKNOWN_SCRIPT` in `definitions.rs` (`unknown_script_problems`, linear in the nodes); no persistence change.
- [x] T007-016 [P] [US02] Add logs/traces/metrics and tenant/secret protection to scenario US02 in `crates/telemetry/`. — as T007-011 (G159); the script test run's log is filtered by 005, and the page shows it as text.
- [x] T007-017 [US02] Run and document duplicate, timeout, worker failure and tenant boundary tests for US02 in `tests/fault/workflow-studio/`. — the script faults are 005's (`tests/fault/inline-code-sandbox/`); the Studio's own are `us02_*`, the conflict panel of the script editor (`ConflictPanel`, `ScriptEditorPage.test.tsx`) and the polling of a test run (`polling.test.ts`).

## Phase 5: US03 — Analyze a real execution (P1)
**Independent test:** Given a Run with retries and an untraversed branch; When the operator opens the graph; Then attempts, duration, states, compensations and timeline reflect the journal.
- [x] T007-018 [P] [US03] Write a Given/When/Then test for analyzing a real execution in `tests/e2e/workflow-studio/us03.rs`. — `tests/e2e/workflow-studio/us03.rs` (14 tests: the changes carry the nodes the journal touched; applying every change in order gives the graph the platform would send whole, over a chain and over a saga whose undos wait for each other; a client ahead or far behind is given the whole graph; a stream, its reconnection, its caps, its end) and the journey `us03-analyze.spec.ts`.
- [x] T007-019 [US03] Implement the domain path and error handling for analyzing a real execution in `frontend/apps/studio/src/features/runs/ExecutionGraph.tsx`. — `frontend/apps/studio/src/features/runs/` (`GraphView`, `RunNodeCard`, `NodePanel`, `TableView`, `TimelineView`, `liveRun.ts`, `attempts.ts`), not `ExecutionGraph.tsx`.
- [x] T007-020 [US03] Integrate adapters and persistence for scenario US03 in `crates/application/` and `crates/infra-postgres/`. — `RunService::graph_changes`, `JournalEntry::touched` read from the stored detail by `crates/infra-postgres/src/execution.rs`, `graph_changes_body` and the stream in `crates/api-http`; no persistence change.
- [x] T007-021 [P] [US03] Add logs/traces/metrics and tenant/secret protection to scenario US03 in `crates/telemetry/`. — as T007-011, plus `run_streams_open`, `run_streams_refused_total` and `run_streams_tick_errors_total` (G159).
- [x] T007-022 [US03] Run and document duplicate, timeout, worker failure and tenant boundary tests for US03 in `tests/fault/workflow-studio/`. — `us03_a_client_that_reconnects_with_the_last_event_id_misses_nothing`, `us03_a_stream_never_outlives_the_token…`, `us03_a_stream_ends_for_someone_who_is_no_longer_a_member…`, the three cap tests, `us03_a_stream_of_a_quiet_run_costs_a_look_at_the_run…`, `tests/fault/platform-foundation/server.rs` (a client that stops reading is let go; one that reads slowly is not), `crates/api-http/tests/studio.rs` (the cap and the cut of the file server); the journey `US03` asserts that the graph is loaded once.

## Phase 6: US04 — Investigate and act on a failure (P1)
**Independent test:** Given a failed run and an authorized operator; When they open the trace, adjust the operational decision and perform a retry; Then the action is audited and does not overwrite the history.
- [x] T007-023 [P] [US04] Write a Given/When/Then test for investigating and acting on a failure in `tests/e2e/workflow-studio/us04.rs`. — `tests/e2e/workflow-studio/us04.rs` (2 tests: the decision and its answer both arrive in the stream and the journal keeps what it had; what an operator may do follows the role and a refusal changes nothing) and the journeys `us04-act.spec.ts` (2: an operator retries an undo with a reason, a viewer is shown why not, an administrator skips one after typing the step id).
- [x] T007-024 [US04] Implement the domain path and error handling for investigating and acting on a failure in `frontend/apps/studio/src/features/runs/Timeline.tsx`. — `features/runs/{ops.ts, OperationDialog.tsx, Operations.tsx, NodePanel.tsx, TimelineView.tsx, journal.ts}`, not `Timeline.tsx`.
- [x] T007-025 [US04] Integrate adapters and persistence for scenario US04 in `crates/application/` and `crates/infra-postgres/`. — feature 003's operations route (`review`) and the idempotent `operationId` (`routes/execution.rs`); no persistence change.
- [x] T007-026 [P] [US04] Add logs/traces/metrics and tenant/secret protection to scenario US04 in `crates/telemetry/`. — as T007-011 (G159); every decision is audited with the reason in the platform (003).
- [x] T007-027 [US04] Run and document duplicate, timeout, worker failure and tenant boundary tests for US04 in `tests/fault/workflow-studio/`. — `tests/fault/workflow-studio/` (`fault_workflow_studio`, 5 tests: two operators at once make one decision; a request repeated with its key is the same operation and the same id; a decision on a stale view is `REVISION_CONFLICT` and changes nothing; a compensation that fails again waits for a person again and the journal keeps both decisions; another tenant gets the answer an unknown run gets).

## Phase: Engineering design details (story-linked hardening)

- [x] T007-028 [US01] Implement canvas with IR validation and diff in `frontend/apps/studio/src/features/workflows/`. — `features/workflows/` with `validate` (`useValidation.ts`, `ValidationPanel.tsx`) and the diff (`DiffView.tsx`, `PublishDialog.tsx`); the journeys `US01` and `SC-007-001`.
- [x] T007-029 [US01] Render runtime graph via journal projection cursor in `frontend/apps/studio/src/features/runs/ExecutionGraph.tsx`. — `features/runs/liveRun.ts` and `api/follow.ts` (cursor, deltas, reconnection, polling fallback), `model/runGraph.ts`; `GET /runs/{id}/graph/changes` and `/stream` on the server.
- [x] T007-030 [US01] Add Monaco/test-run and per-type schema form in `frontend/apps/studio/src/features/scripts/`. — Monaco in `features/scripts/` (TypeScript and JSON language services, the script's schemas registered for its test input, a test run in the sandbox) and the per-type forms generated from `model/registry.ts` for the nodes; the workflow's own input and output schemas are JSON text fields checked by the platform (see FR-007-002 in `docs/traceability.md`).
- [x] T007-031 [US01] Implement manual review and compensation UI with double confirmation and RBAC in `frontend/apps/studio/src/features/runs/`. — `features/runs/{OperationDialog.tsx, Operations.tsx, ops.ts}`: the consequence in words, a reason of 8 to 2 000 characters, the step id typed back for the decisions that override or may repeat an effect, buttons that follow `/auth/context` and the state of the step, a refusal shown as it is; tests in `operations.test.tsx` (32) and the journeys `US04`.

## Phase: Visual direction (FR-007-009, FR-007-010, SC-007-005)

Added on 2026-10-10 when the design direction was recorded; numbered after the final phase because it came later, and to be done before
T007-034 (the benchmark must measure the restyled Studio) and T007-035/036. Context: D154–D159, G160–G168, `design/README.md`.

- [x] T007-037 Record the design direction: source, what was measured and what is not known, the mapping of labels, the elements the platform cannot feed, the accessibility deviations. — `specs/007-workflow-studio/design/README.md` and `design/runs-prototype.jpg`; FR-007-009/010, SC-007-005, D154–D159 and G160–G168 in the spec and `research.md`. **No Studio code was changed.**
- [x] T007-038 Replace the measured values by the exact ones from the owner's export and update the record (G160). — `design/README.md` rewritten from `design/source/sagat-octopus.dc.html` (sha-256 in the record): `oklch` tokens, faces, scale, radii, sizes and graph; the contrast of each pair computed from them, which corrected one number of the first version; the deviations of D155 listed with their ratios. The light theme, focus and disabled states are not in the design (G160).
- [x] T007-039 [P] Tokens: the dark, light and forced-colours themes in `frontend/apps/studio/src/styles/base.css` from the record (the light theme and the focus ring derived, G160/G161), with a Vitest test that converts the tokens and computes the contrast of every text, control-border and focus pair per theme against its threshold (the deviations of `design/README.md` are its expected values), and one that fails on a literal colour in any other stylesheet (SC-007-005).
- [x] T007-046 [P] Fonts: Geist and Geist Mono served from the platform's own origin (`font-src 'self'`), the licence read, the packages pinned and at least two weeks old (D138, D159); the system stacks stay as the fallback and the layouts are checked with it (G166). If the packages are not approved, the system stacks stay and G166 is closed as a decision.
- [x] T007-040 [US03] Shell: sidebar (brand, namespace, navigation without links to pages that do not exist, G163) and top bar in `frontend/apps/studio/src/app/Shell.tsx` and `styles/base.css`; the skip link, the focus move on navigation and `nav[aria-label=Main]` keep working; `Shell` tests and the journeys pass.
- [x] T007-041 [US03] Runs: list beside detail on a wide viewport, the URL of a run unchanged, two pages below the reflow width (D158); the strip of facts, the filters and the selected-step strip from what the API gives (FR-007-010, G164) in `features/runs/{RunsPage,RunDetailPage,RunHeader,NodePanel}.tsx` and `styles/runs.css`; `RunsPage` and `RunDetailPage` tests and `us03-analyze.spec.ts` pass.
- [x] T007-042 [US03] Graph: the cards, edges and dotted canvas of the prototype in `features/runs/{RunNodeCard,GraphView}.tsx` and `styles/runs.css`, keeping a taken edge solid, a pending edge dashed and an omitted edge dotted, the visible focus ring and the forced-colours rules that `styles.test.ts` asserts.
- [x] T007-043 [US01] [US02] Derive the workflow editor and the Scripts pages from the same tokens, without a reference design (G161): `styles/{workflows,scripts}.css` and the sign-in; the result is a proposal and is reviewed by the owner before it counts as done.
- [x] T007-044 Verify: axe-core in the journeys in light, dark and forced colours on every page, the US01..US04 and `SC-007-001` journeys unchanged in what they assert, the Runs screen compared with `design/runs-prototype.jpg` at 1470 × 812 by a person (a review, reported as one), and `npm run typecheck`, `lint`, `test` and `build` of the frontend.
- [x] T007-045 Update `docs/traceability.md` and the Studio's README for FR-007-009/010 and SC-007-005 with the evidence of T007-039..044 and T007-046.

Implementation evidence (2026-10-11; owner approval and sandbox CI validation recorded):

- **T007-039:** shared colour/type/spacing/radius/layout tokens, system light/dark preference and forced-colour mappings are in
  `styles/base.css`. `tokens.test.ts` checks contrast, theme parity, fonts/licence, colour literal ownership and resolution of
  custom properties. Dark Monaco contrast was corrected; the final three-theme browser checks passed.
- **T007-046:** Geist and Geist Mono `5.3.0` are pinned and bundled locally; publication 2026-07-19 meets D138's age rule. SIL OFL
  1.1 is distributed in `public/licenses/geist-OFL.txt`. The browser test blocked font files, verified Geist did not load and
  passed 320px reflow using the system fallback (G166 closed).
- **T007-040:** `Shell.tsx` and `Shell.test.tsx` cover the sidebar, namespace, real destinations, session, skip link and navigation
  focus. The final expanded local browser journeys exercised the shell and passed.
- **T007-041:** wide master/detail composition, API-backed facts and selected-step strip are implemented. Regression tests cover
  preserving master-list filters/pagination on sibling navigation and unknown terminal duration. US03 passed in the final partial
  browser suite; no hidden master list is mounted below 74rem.
- **T007-042:** compact cards, curves, dotted canvas and solid/dashed/dotted edge states are implemented. Review fixed undefined
  graph tokens, omitted-edge contrast and the selected-step mobile cascade; `canvas.test.ts`, `styles.test.ts`, shared token
  tests and browser checks passed, as did both restyled benchmarks.
- **T007-043:** editor, Scripts and sign-in styles are derived from shared tokens. **Owner approved on 2026-10-11**; no reference
  design exists for those screens or the light theme.
- **T007-044:** **1 033 Vitest tests in 64 files**, lint/typecheck/build/API/advisory checks and Rust fmt/clippy/architecture passed
  locally. Final browser result: **18 passed, zero failed, two opt-in performance skips**; the skipped benchmarks passed separately
  (T007-034). Light/dark/forced-colour accessibility, 320px reflow and system-font fallback passed. **US02 was excluded** by explicit
  partial mode because Docker's sandbox runtime was unavailable. CI then passed all 19 journeys including US02 with a real sandbox (two opt-in benchmarks skipped), and 1 034 unit tests. The owner approved the visual proposal. Ten PNGs are linked from `design/README.md`; details and CI link: `quickstart.md`.
- **T007-045:** README, runbook, design/research record, quickstart and traceability describe the implementation, final local
  results, owner approval and the successful Studio CI job.

## Final Phase: Hardening / rollout
- [x] T007-032 Run the relevant REST/gRPC/AsyncAPI contract suite in `tests/contract/workflow-studio/`. — REST only: this feature adds no gRPC and no event contract; `tests/contract/workflow-studio/rest/conformance.rs` (the new operations on their normal and error paths against the OpenAPI) and `npm run check:api` (the generated types equal the contract).
- [x] T007-033 Apply lint and acyclic dependency checks in `crates/` and product rules in `docs/architecture/`. — Restyle: Rust fmt, workspace/all-targets clippy and architecture passed locally, as did frontend lint/typecheck/generated API/advisory checks. Final post-Monaco-fix frontend checks passed; prior baseline `cargo audit` evidence is recorded separately in `quickstart.md`.
- [x] T007-034 Run the benchmark defined in `specs/007-workflow-studio/plan.md` and publish the report in `tests/performance/reports/workflow-studio.md`. — Restyled Studio: both `frontend/e2e/tests/perf.spec.ts` benchmarks passed, three cold rounds each; all six unchanged floors held. The 1 000-node editor opened in median 687 ms; the 9 991-node run in 1 470 ms, drawing 245 cards and at most 29 table rows, peak recorded heap 307 MB. Report regenerated. One machine and Chromium; partial no-sandbox mode does not affect these scenarios.
- [x] T007-035 Review security, redaction, RBAC, README and runbook; record evidence in `specs/007-workflow-studio/quickstart.md`. — The original two review rounds remain in the baseline record. A fresh read-only restyle review checked correctness, security, accessibility and performance; its confirmed graph, navigation and duration findings were fixed. README/runbook and `quickstart.md` record the review fixes and passing technical checks; the owner approved the visual proposal on 2026-10-11.
- [x] T007-036 Update FR/SC traceability and mark proven criteria in `docs/traceability.md`. — The evidence table records the built visual changes and final passing local checks, with the excluded US02 journey and recorded owner acceptance explicitly distinguished.

## Dependencies & independent delivery

- This feature requires: 001, 002, 003, 004.
- US01 creates the smallest vertical slice; US02..US04 can evolve after Foundation, with stable contracts.
- `[P]` tasks are parallel only if they do not change the same file/contract.
- No `[ ]` may become `[x]` without a verifiable test/artifact.

## Incremental implementation strategy

Contract → first failing test → domain → adapter → happy path → failure/recovery → observability → security gate → performance → release.

First T007-039 increment (2026-10-11): shared palette and contrast regression tests
implemented on `feature/007-design-tokens`; see `design/README.md` for measured
failures, deviations, validation and the coverage that was still open at that increment.

The later restyle, owner approval and real-sandbox CI journey complete T007-039..046.

Owner acceptance, 2026-10-11: Marcelo reviewed the open local Studio and explicitly answered
"Aprovo a proposta visual" for Runs, the light theme, workflow editor, Scripts and sign-in. This closes
the owner-review portion of T007-043/044 and SC-007-005. The real-sandbox US02 journey also passed in the Studio CI job on commit `ace4060`, completing T007-044. No second visual approval is required for this proposal.

## Remaining V1 schema editing

- [x] T007-047 Provide schema-aware workflow input/output editors (FR-007-002), using bundled Monaco JSON validation/autocomplete with independent models and an accessible plain-text fallback. Preserve incomplete edits across blur and editor-mode changes. Evidence: 31 focused component tests, typecheck/lint/build, and all three real Chromium US01 journeys passed on 2026-10-12; the new journey checks completions, schema diagnostics and lossless fallback.

## Visual defect regression (FR-007-008/009, SC-007-005)

- [x] T007-050 Fix HTML/asset mismatch after rebuilding a running Studio and replace the router's
  developer error screen with accessible recovery. Prove fresh root/deep-link HTML, safe missing
  index handling and real-browser recovery from a missing CSS chunk; update serving/UI contracts
  and the runbook. Preserve the configured security and resource boundaries. Evidence on 2026-10-12:
  13 HTTP serving tests, 1 051 frontend tests and 24 real Chromium journeys passed (two opt-in
  benchmarks skipped; US02 excluded). New regressions failed before the fix. Lint, app/E2E types,
  frontend/control-plane builds, targeted Rust clippy and formatting passed. The live local server
  was replaced and its current HTML/sign-in assets checked in Chromium without browser errors.

- [x] T007-049 Correct unreadable initial graph zoom, intermediate-width editor layout, selected
  forced-colour controls and the run header/canvas geometry. Add real-browser regressions in
  `frontend/e2e/tests/visual-layout.spec.ts`, retain API and interaction coverage, and record actual
  tests/captures in the design record and quickstart. No backend or wire-contract change. Evidence
  on 2026-10-12: 1 049 Vitest tests in 67 files; lint, app/E2E types, production build and generated
  API checks passed. Real Chromium/API/engine suite: 23 passed, zero failed, two opt-in benchmarks
  skipped; US02 explicitly excluded in no-sandbox mode. Eight before/after captures are in
  `design/review-2026-10-12/`. UX and frontend agent review completed; no new human acceptance claimed.

## Remaining V1 payload inspection

- [x] T007-048 [US03] [US04] Implement the authorized, audited `structure-only-v1` payload inspection in ADR-0028 (FR-007-005, FR-007-010, FR-008-006, FR-009-004): input or immutable journal-sequence selectors, new `run:payload:inspect` permission, tenant-scoped payload read plus committed bounded audit, server-side removal of every value and key with strict projection caps, verified external metadata and honest no-payload states. Evidence on 2026-10-12: the real PostgreSQL/API inspection suite passed 2/2; application projector and identity suites passed 6/6, 4/4 and 27/27; 70 focused Studio tests plus lint/typecheck/build passed; and one real Chromium journey against the real API/engine passed with key/value canaries absent. Tests cover role and tenant boundaries, audit failure/collapse, historical selection, forged descriptors, external metadata, size/depth/UTF-8 caps and stale session/query cancellation. Customer classification, retention decisions and raw-value disclosure remain pending under G42/G39.
