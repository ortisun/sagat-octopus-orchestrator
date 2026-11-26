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
| 4 | Nit | Settled: a rewrap of the whole feature range is a mechanical change kept out of the approval commit so that the reviewed diff stays readable; it is made in a commit of its own after approval, which the body check allows only as whitespace if at all, so it waits for the next amendment of the design | The feature range |
| 5 | Nit | Fixed: Compensation, Reconciliation, Cross-tenant battery and Runtime role are entries of their own | Terms |
| 6 | Nit | Fixed: options and choice summarised | Decision 050-004 |
| 7 | Nit | Fixed: Task 050-1 changes the link to the relative one when #267 merges; until then the link names the branch the coordinator merges first | Task 050-1 |
| 8 | Nit | Fixed: the directory is linked and both resolutions named; "Architecture decision 0082" is the link text | Overview, Technical Plan 3 |
