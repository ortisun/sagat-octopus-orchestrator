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
