# Targeted re-read 3 of design 043: architecture (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: 2606e1d59960a5b72e35726a7b6db4e9f05fa74a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A third targeted re-read of
the changes since d890b926 (OD-2026-11-16-3), not a full review. The reviewer read files only and ran nothing.

No new layering, ADR or compatibility problem; the Nits of re-read 2 are closed. The gate record (re-read 1 Major 5) is
not counted open: nothing but the record itself is missing. It is resolved when `validation-report.md` names the commit
(and, if later, a diff of records or test-only lines), the environment, the static gates, the unit tests of the changed
packages, `integration_gitops_versioning`, `_database_connections`, `_schedules_bulk_operations`, `_workflow_primitives`,
`_run_input_cache` and `_claim_transition_efficiency` as the owner and as the runtime role with the named tests, the
"seen failing first" evidence, every failure with its cause, and no tick before it.

## Nit

2. The new `interrupted` path of the inventory has no test.
3. The catalog probe names `public.` while the statement uses the search path.
4. The runbook and FR-043-018 list fewer causes of exit 2 than the code has.

Counts: Blocker 0, Major 0, Nit 3
