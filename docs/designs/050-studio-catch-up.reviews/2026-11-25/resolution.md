# Resolution of the reviews of design 050 (2026-11-25)

The author's disposition of each finding, under Owner decision 2026-11-24-4 (findings settled by the delegate). A
finding marked fixed is answered in the design or its records at the commit that follows the report.

## Architecture review, round 1 (`architecture.md`, at `6a49c922`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: no capability for loops; the publishable subset stated as a table, owned by the codec, shown by a note per kind and by the editor's live validation at the node, never written again in the Studio; the criterion and the journeys rewritten as workflows that publish today | Overview, Terms, The Problem, Requirements 050-003, 050-006, 050-007, Success criterion 050-001, CUJ 1, CUJ 4, Technical Plan 4, Decision 050-009, Architecture decision 0143 decision 5 |
| 2 | Major | Fixed: `PlatformCapability` in `crates/application`; static names at start-up, registry names from the heartbeat's requirement set through one tested mapping; deployment-wide; an absent field means a server before 0.3.0 | Requirement 050-002, Technical Plan 3, Architecture decision 0143 decision 2 |
| 3 | Major | Fixed: every concrete name enumerated, two names for 022, 029's `access.groups` and `access.external-groups` added, 028's names prefixed (Decision 028-008); strings with a pattern and `x-known-values`, never an `enum`; a reader ignores an unknown name | Technical Plan 3; specs/028 research Decision 028-008 |
| 4 | Major | Fixed: `ApprovalRequest::may_decide` extracted from the decision and called by it; the use case filters bounded pages; no SQL predicate; the property test is pure domain; an approval on an ended run is not decidable | Requirement 050-010, Technical Plan 6, Task 050-4, Architecture decision 0144 decision 2 |
| 5 | Major | Fixed: `PublicationRequest::may_decide` with the three clauses in the domain; `PENDING` only | Requirement 050-011, Technical Plan 6, Task 050-5, Architecture decision 0144 decision 6 |
| 6 | Major | Fixed: pending approvals only; the index's `down` and build cost stated | Requirement 050-010, Technical Plan 6, Decision 050-010 |
| 7 | Major | Fixed: a `runId` filter read through the run's index, outside the deadline scan | Requirements 050-009, 050-010, Technical Plans 5 and 6 |
| 8 | Major | Fixed: the order names 022's rebase onto `capability` and `scope`, 041 before the run slice, 026 after the editor slice, 044's Task 044-23 after the run and versions slices with 050 owning the versions page, and the later of 050 and 029 adding `discloses` and the folder filter | Technical Plans 1 and 2 |
| 9 | Major | Fixed: no schema validator in the browser; the server's `422` at its pointer | Requirement 050-014, Technical Plan 5 |
| 10 | Major | Fixed: the download is paced, kept as raw page text and bounded at 50,000 events or 32 MiB | Requirement 050-021, Technical Plan 5, Decision 050-008 |
| 11 | Nit | Fixed: twelve entries; both legacy messages; `VersionsPanel.tsx` grown into the versions page; the schema read by path; `model/runGraph.ts`; the approval refusals in the subset table | The Problem, Terms, Technical Plans 4, 5, 8; Architecture decision 0143 |
| 12 | Nit | Fixed: equality under `canonical()`, the node kept at its index | Requirement 050-004, Technical Plan 4 |
| 13 | Nit | Fixed: keyed on `type` constants, required fields per scope, schema read from the contracts folder | Requirement 050-001, Technical Plan 4 |
| 14 | Nit | Fixed: out-of-range values asked, with the old value shown | Technical Plan 4 |
| 15 | Nit | Fixed: an exhaustive `match` behind `AuditAction::ALL`; the known actions on the OpenAPI `action` parameter; free text still accepted | Requirement 050-013, Technical Plan 7, Decision 050-006 |
| 16 | Nit | Fixed: four rows of `architecture-principles.md`, the baseline, the target `integration_studio_catch_up`, the metric names and labels | Technical Plans 6 and 9, Task 050-2, Verification Plan |
| 17 | Nit | Fixed in part: `summaryAvailable` removed (feature 026 adds its flag). Contested in part: `OD-2026-11-24-700` stays. The identifier scheme is `OD-YYYY-MM-DD-n` with any `n`; on 2026-11-24 the parallel sessions took separate ranges (300 for the Windmill parity readiness, 500 for scale and resilience, 700 for this one) so that their rows cannot collide when the pull requests merge in any order, which a "next free `n`" cannot guarantee across branches | Technical Plan 6 |

## Correctness review, round 1 (`correctness.md`, at `aac7f76e`)

All 19 findings are accepted and fixed; none is contested. Findings 1, 4 and 8 make three small server corrections that
the first text excluded ("nothing else changes in the engine"); the Overview now names them.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the registry member reads the requirement set after registration and on every epoch change and keeps it; red test for a replica started after an activation | Technical Plan 3, Requirement 050-002, Task 050-3 |
| 2 | Major | Fixed: the cursor is the last row scanned, null only when none remains; the Studio follows up to five pages per refresh and restarts each refresh; `decidable=false` defined; the 2,001-plus-one test | Requirements 050-010, 050-012, Technical Plan 6, Tasks 050-4, 050-10, Architecture decision 0144 decision 5 |
| 3 | Major | Fixed: `may_decide(principal, run_ended)` and `decide` take the same input from `RunStatus::is_terminal`; the property restricted to pending requests with fresh keys; the ended-run integration test | Technical Plan 6, Architecture decision 0144 decision 2 |
| 4 | Major | Fixed: `409 RUN_ENDED` instead of a retryable `503`; ended-run rows left out; status re-read after a refused decision; the run-end cancellation test with a fix in the same delivery | Requirements 050-009, 050-010, Technical Plans 5 and 6 |
| 5 | Major | Fixed: open requests still needing an approval under the current policy, `APPROVED` included; the open-state clause in `may_decide` | Requirement 050-011, Technical Plan 6, Architecture decision 0144 decision 6 |
| 6 | Major | Fixed: order by request time, keyset cursor, bound, partial index with its `down`, decisions per page | Technical Plan 6, Task 050-5 |
| 7 | Major | Fixed: a key reused only for the identical body; a new key for a changed body; fields frozen once sent | Requirements 050-009, 050-014, Technical Plan 5 |
| 8 | Major | Fixed: `DefinitionProblem` gains an optional `body`, resolved on the body's canvas; the first-error limitation stated | Technical Plan 4, Requirement 050-003, Task 050-6 |
| 9 | Major | Fixed: iteration status defined from forward instances and the `return`; compensations and reconciliations beside their step; "admitted so far" | Requirement 050-008, Technical Plan 5 |
| 10 | Major | Fixed: the red test uses unreadable and duplicate-id entries, in the root and in bodies | Requirement 050-004, Technical Plan 4, Task 050-6 |
| 11 | Major | Fixed: values carried only when they pass the target schema; absent required fields asked; every dropped property listed; the body subgraph offered a move | Requirement 050-005, Technical Plan 4 |
| 12 | Major | Fixed: without the field, the languages 0.2 builds; `GET /version` re-read at sign-in and after a refused build | Requirement 050-019, Technical Plan 3 |
| 13 | Major | Fixed: pages parsed, one line per item, counted as written, `Retry-After` honoured, "as of the last page read" | Requirement 050-021, Technical Plan 5 |
| 14 to 19 | Nit | Fixed: the run filter's order and cursor bound to it, the namespace check and the rows-scanned test; `CREATE INDEX IF NOT EXISTS` with a `CONCURRENTLY` runbook step; the stale consequence of Architecture decision 0143 corrected; "as of the last successful beat"; `expectedStateRevision` prefilled; `403` after a sent decision treated as "status unknown" | Technical Plans 3, 5, 6; Architecture decisions 0143 and 0144 |

## egm-review Stage 5, Initial Zoom Out (`zoom_out_report.md`, at `035b2bf0`)

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Contested, in writing. The owner's scope names one design for "the Studio catch-up" (Owner decision 2026-11-24-1), and the coordinating session's brief asks this design to cover what else the Studio lacks against Windmill's and Temporal's interfaces; splitting the operator surfaces into a second design would add a second review cycle on the release's path for screens that only call routes that exist. The risk the finding names (a later small change sending the foundation back to draft) is answered instead by finding 13's fix (requirements state observable behaviour; numbers live in the Technical Plan) and by the approval rule itself, which lets follow-up tasks be added after approval. Task 050-1 stays here because accepting ADR-0101 is this design's Decision 050-001; it lands in pull request #267 while that is open | Decision 050-001, Requirements 050-009 to 050-021 |
| 2 | Major | Fixed: design 050 ships two static names; other features add theirs when they ship; the registry derivation and the member change are built by the first feature with a registry-derived name (022, 029 or 013), recorded in Architecture decision 0143 decision 2 and in 029's Task 029-014 and 013's Requirement 013-032 | Requirement 050-002, Technical Plan 3, Task 050-3 |
| 3 | Major | Fixed: the codec's existing pointer becomes `/bodies/{name}/nodes/{index}`; no new field | Technical Plan 4, Requirement 050-003, Task 050-6 |
| 4 | Major | Fixed: `youMayDecide` on the existing per-run and per-workflow lists; the namespace routes lose their filters | Requirements 050-009 to 050-011, Technical Plans 5 and 6, Tasks 050-4, 050-5 |
| 5 | Major | Fixed: a read-only inspector with an "add here" action; the mapping dialog deferred to Gap 050-005 | Requirement 050-005, Technical Plan 4, CUJ 2, Task 050-7 |
| 6 | Major | Fixed: the runtime graph reports iteration statuses and counts from the engine's recorded outcomes; the Studio only groups and draws | Requirement 050-008, Technical Plan 5, Task 050-9 |
| 7 | Nit | Fixed: the drivers list stays the advertisement of lake writes; no `workflow.lake-writes` name | Requirement 050-003, Technical Plans 3 and 4 |
| 8 | Nit | Fixed: Go and Java marked built in the constant; `LANGUAGE_NOT_BUILT` handles another build | Requirement 050-019, Decision 050-007, Technical Plan 8 |
| 9 | Nit | Fixed: the three meanings of "capability" mapped; the type named `AdvertisedCapability`; the wording aligned with Architecture decision 0143 | Technical Plan 3, Architecture decision 0143 decision 2 |
| 10 | Nit | Fixed: one `collectPagesBounded` for the inbox, the audit download and the journal download; no fixed pace | Technical Plans 5 and 6, Requirements 050-012, 050-021 |
| 11 | Nit | Fixed: the existing per-intent key pattern as one hook `useIntentKey`; no key format | Technical Plan 5, Requirements 050-009, 050-014 |
| 12 | Nit | Fixed: `useWindowedRows` and `JsonField` named | Technical Plan 5, Task 050-9 |
| 13 | Nit | Fixed: Requirements 050-009, 050-010, 050-012, 050-014 and 050-021 state observable behaviour; the bounds and mechanics are in Technical Plans 5 and 6 | Requirements |
| 14 | Nit | Fixed: the versions page lives in `features/workflows/` | Requirement 050-016, the plan's introduction |
| 15 | Nit | Fixed: a wrong run-end path becomes a follow-up task of its own scope | Technical Plan 6 |

## egm-review Stage 6, Comprehension Test (`comprehension_report.md`, at `ea803052`)

All 21 findings are accepted and fixed; none is contested.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: the Overview says which primitives publish everywhere and which follow a switch | Overview |
| 2 | Major | Fixed: "Codec" defined in Terms, the same as "the server's validation" | Terms |
| 3 | Major | Fixed: `workflow.step-destinations` gates the `stepDestinations` field, shown read-only without it | Technical Plan 3 |
| 4 | Major | Fixed: the instance registry defined with its link; stated as not part of design 050 | Technical Plan 3 |
| 5 | Major | Fixed: "add here" moves every incoming link, the entrypoint included; a read-only node can be deleted; one wording | Requirement 050-005, Technical Plan 4 |
| 6 | Major | Fixed: Task 050-12 renamed with the requirement's tests | Task 050-12 |
| 7 | Major | Fixed: the server builder owns every `crates/` part; each task's predecessors listed | The plan's introduction |
| 8 to 21 | Nit | Fixed: "four" small things and `youMayDecide` glossed; IR expanded and a list of referenced features; the owner, the delegation, the review files and "an earlier draft" named; gaps in order; only the notes change when the subset widens; field-level capabilities read-only and kept; admitted iterations, drain reasons, compensations and reconciliations defined and the counting rules stated; chain and declared state defined; the test terms glossed; the Studio served from another build explained; CUJ 3 worded; source and recovery activities defined, the combination refusal moved, supersede and recount explained; the two task-identifier forms explained; "derived from the start-up switches"; the cancellation test a probe | Overview, Terms, Technical Plans 3 to 6, CUJ 3, Requirement 050-011, the plan |

## egm-review Stage 7, Design Critic (`critic_report.md`, at `ea803052`)

All 14 findings are accepted; none is contested. Findings 11, 13 and 14 had already been fixed by the settlement of
Stage 6 (`ca5ef2a0`), which the critic did not see; finding 6 reverses Stage 6 finding 3, which had kept the name.

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Major | Fixed: outcomes `SUCCEEDED`, `FAILED` (the one named by the drain), `DRAINED`, `RUNNING`, and not admitted for a collection, from the iteration events; CUJ 4 rewritten as one failure that stops the collection (1,200 + 1 + 15 + 784 = 2,000); citation corrected | CUJ 4, Requirement 050-008, Technical Plan 5 |
| 2 | Major | Fixed: counts on the owner only, whose every iteration event names it; no field on scoped instances; `RETRYING` dropped (a retried step shows on its own node); the `graph/changes` versus fresh graph test added (Decision 050-012) | Technical Plan 5, Requirement 050-008, Task 050-9 |
| 3 | Major | Fixed in part: per-iteration outcomes moved to the paged route `GET /runs/{runId}/graph/iterations`; the existing graph route is not paged by this design; payload size and parse time are measured and a graph read by scope is the follow-up if they exceed the budget | Gap 050-006, Non-functional requirement 050-002, Risks, Task 050-14 |
| 4 | Major | Fixed: only the tab in view, only while visible; interval doubling to four minutes while unchanged; the load estimated (300 inboxes: at most 50 requests per second, about 6.3 when unchanged, against 100) and measured | Technical Plan 6, Requirement 050-012, Risks, Task 050-14 |
| 5 | Major | Fixed: the inbox is per namespace (Decision 050-011), a Non-Goal, the goal and user story reworded | Goals, Non-Goals, User story 050-02, Decisions |
| 6 | Major | Fixed: only `workflow.collection-operators` ships; feature 035 adds `workflow.step-destinations` with its switch; the field stays read-only until then | Requirement 050-002, Technical Plan 3, Task 050-3 |
| 7 | Major | Fixed: Architecture decision 0144 decision 1 has no filters; decision 2 adds the per-run flag | Architecture decision 0144 |
| 8 | Major | Fixed: a reduced publication inbox item without decisions or the last editor; the requester named and why; the contract test `the_publication_inbox_item_has_exactly_its_fields`; decision 6 reworded | Technical Plan 6, Requirement 050-011, Architecture decision 0144, Task 050-5 |
| 9 | Nit | Fixed: the ended-run answer named as a deliberate change with the conformance scenario `an_approval_decision_on_an_ended_run_is_not_retried` in six languages; the closed schemas gain the property; the SDK descriptions corrected | Non-functional requirement 050-003, Overview, Tasks 050-2 and 050-4, Architecture decision 0144 |
| 10 | Nit | Fixed: the three answers of an older replica and what the Studio does with each | Gap 050-001, Task 050-10 |
| 11 | Nit | Already fixed at `ca5ef2a0`; the test renamed `add_here_moves_every_incoming_link_to_the_new_node` | Requirement 050-005, Technical Plan 4, Task 050-7 |
| 12 | Nit | Fixed: option 3 rejected only for kinds and fields a deployment switches off; shape refusals left to live validation | Architecture decision 0143 |
| 13 | Nit | Already fixed at `ca5ef2a0` | Gaps |
| 14 | Nit | Already fixed at `ca5ef2a0` | Task 050-12, Verification Plan |

## egm-review Stage 9, Implementation Readiness (`readiness_report.md`, at `dba19bef`)

All 22 findings are accepted and fixed; none is contested. The plan is rewritten with one builder per task, explicit
external gates, a file and a command per task, and the server and Studio parts split (new Tasks 050-15 to 050-19).

| Finding | Severity | Disposition | Where |
|---|---|---|---|
| 1 | Blocker | Fixed: the progress tables are the source. A migration adds `failed_iteration` and `failure_code` to `iteration_scopes` and `loop_scopes`, written at a `Failed` drain. The mapping from lifecycles to outcomes is stated. The counts go in `GraphSnapshot` through `RunStore::graph` and are drawn by `runtime_graph`. The page is `RunStore::iteration_page` with `owner` (the owner's `RuntimeNode.id`), `after` and the filter applied in the query. Runs that began their drain before the migration are stated | Technical Plan 5, Task 050-9 |
| 2 | Major | Fixed: `RunEnded` comes only after authorization, replay and the settled check, and `may_decide` mirrors that order. `a_replay_on_an_ended_run_still_answers_its_receipt` covers it | Technical Plan 6, Architecture decision 0144, Task 050-4 |
| 3 | Major | Fixed: the Rust integration test seeds the inconsistent row. Each of the five SDKs that call the decision route gets a unit test of the `409` classification | Non-functional requirement 050-003, Task 050-4, Architecture decision 0144 |
| 4 | Major | Fixed: `x-planned` first, switched by each route's task. The two digests are named with their renewal, and the redocly lint is the verification | Technical Plan 9, Task 050-2 |
| 5 | Major | Fixed: `audit-actions.json` is imported by the Studio, and one contract test in `contract_rest_workflow_studio` holds both lists to `AuditAction::ALL` | Decision 050-006, Technical Plan 7, Task 050-11 |
| 6 | Major | Fixed: the hook fills `decisionKey` or `updateKey`. A resend is the person's, and the client retries nothing; Gap 050-001 is corrected to match | Technical Plan 5, Gap 050-001 |
| 7 | Major | Fixed: the variants that gain a body scope are listed, `nodeId` is kept, and `resolveNodeId` reads the body pointer first. The server tests live in `crates/application` | Technical Plan 4, Tasks 050-6, 050-15 |
| 8 | Major | Fixed: a nested `HandleSpec` and five edge kinds; the body-document adapter `model/bodies.ts` as Task 050-16 before the canvas; the green test marked as a guard; `useCapabilities` with its query key | Technical Plan 4, Tasks 050-15, 050-16, 050-8 |
| 9 | Major | Fixed: component tests with a mocked `validate`, and publication of each kind as a journey case | Task 050-7, Task 050-13 |
| 10 | Major | Fixed: `advertised_capabilities` is a pure function used by the control plane and the harness. The `AppState` field and both routers are named, the switch citation is corrected, and `every_advertised_capability_is_a_known_value` is added | Technical Plan 3, Task 050-3 |
| 11 | Major | Fixed: owners for every path, with the external gates as predecessors on the tasks | The plan's introduction, every task |
| 12 | Major | Fixed: a file and a command per task. The integration target is created in Task 050-4, and the decision records' test paths now point at `rest/` files compiled by `contract_rest_workflow_studio` | Tasks, Architecture decisions 0143 and 0144 |
| 13 | Major | Fixed: Task 050-19 enables the switch, seeds the policy row, and sets a budget of three minutes per journey | Task 050-19 |
| 14 | Major | Fixed: `perf_studio_catch_up`, `perf.spec.ts`, the seeding method, the 300-inbox driver and the report path | Task 050-14, Verification Plan |
| 15 | Major | Fixed: `ApprovalInboxStore::pending_page` with its row type, the role read by `IdentityStore::find_membership`, `list_for_run` returning the same rows, and `PublicationInboxStore::open_page` reading `approvalsRequiredNow` by publication's policy resolution | Technical Plan 6 |
| 16 to 22 | Nit | Fixed: `inputSchema` is kept; Go and Java samples are named; the publication property test is added; the position `match` pattern is described; `openedAt` is `created_at` and the limit is 1 to 200; the server and Studio parts are split; each Studio test has its file | Technical Plans 4, 6 and 7, Tasks |
