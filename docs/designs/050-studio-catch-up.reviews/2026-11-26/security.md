# Security review of design 050 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 5f90d452
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: the first security round of design 050; condensed by the author to one paragraph per finding, every severity, location and fix kept. The reviewer read files only and ran nothing.
Counts: Blocker 0, Major 2, Nit 8

1. **Major. Technical Plan 6; `crates/infra-postgres/src/approval_progress.rs` (`list_for_run`, `load_request`).** The inbox reuses "the same row type" as `list_for_run`, an N+1 adapter (one statement per request), so a 2,000-row page is up to 2,000 statements, for any viewer, multiplied by the per-subject and per-tenant rate limits, against the shared database. Fix: one statement per page joining runs, workflows and the business identifier with `tenant_id` in every join; a statement-count test.
2. **Major. Technical Plan 6; Architecture decision 0144 decisions 4 and 5; Technical Plan 1 item 7.** The cursor names the last row scanned, and cursors are plain base64; if feature 029's `visibleFolders` filters after the scan, a hidden approval's identifier and deadline leak through `nextCursor` (and the same for the publication inbox). Fix: folder visibility as a SQL predicate of the scan, or a sealed cursor; a test that a hidden last row is never in a cursor.
3. **Nit. Technical Plan 6; `approval_progress.rs`.** Today the ended-run guard runs before the principal is read; remapping it to `409` would tell an ineligible caller that the run ended. Fix: delete the guard; test an ineligible caller on an ended run gets `403`.
4. **Nit. Technical Plan 5, the iteration route.** A typed query refuses before run admission, breaking the codebase's refusal order and the battery; `owner` is unbounded. Fix: read the query after admission, bound `owner`, an unknown owner answers without detail, `Cache-Control: no-store`.
5. **Nit. Tasks 050-2, 050-4, 050-5, 050-9; `battery.rs`.** The `not_built` bookkeeping and the iteration route's query string are missing. Fix: name the edits per task.
6. **Nit. Architecture decision 0144; the Risks row.** The policy is in the definition and the initiator and deciders are journal actors, so withholding them is not a confidentiality boundary. Fix: word it as "no disclosure beyond the definition and the journal"; keep the field-list test.
7. **Nit. Technical Plan 5.** One grouped query per owner on every `graph/changes` poll grows with design 039's owners. Fix: one grouped query per run.
8. **Nit. Technical Plan 6, the migration.** A failed concurrent build leaves an invalid index that `IF NOT EXISTS` skips. Fix: the runbook checks `indisvalid` and rebuilds.
9. **Nit. Tasks 050-5, 050-9.** Runtime-role runs cover only Task 050-4; the inbox adapters must take no row lock. Fix: add the runs; no row lock.
10. **Nit. Technical Plan 7; `src/api/client.ts`.** Audit filters in the address would put subjects in history and links; downloads hold personal data; `path()` lets `.` and `..` through. Fix: filters in component state, label the downloads, refuse those segments.

Looked at with no finding: `GET /version` (authenticated, deployment-wide, advice only), the namespace routes' tenant boundary (`404` for another tenant's namespace, audited), replay after revocation, the ended-run classification, the publication item's disclosure, `youMayDecide` as an oracle, metric labels, the referrer policy, browser storage.
