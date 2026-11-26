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
