# Targeted re-read 2 of design 043: architecture (2026-11-16)

Doc: docs/designs/043-run-input-validation.md
Reviewed-commit: d890b9263cd8b84a7156edf66c3f08224b0d1617
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read of
the changes since a763d9d0 (OD-2026-11-16-3), not a full review. The reviewer read files only and ran nothing.

Re-read 1 Majors 1 to 4: resolved. No new layering, ADR or compatibility problem.

## Major

1. (Re-read 1 Major 5, open.) No gate run is recorded at the reviewed commit. Recording it on the final tree after this
   re-read is acceptable if the record names its commit (and, if it is not this one, that the diff since touches records
   only), covers the targets this diff changes as the owner and as the runtime role (`integration_gitops_versioning`,
   `integration_database_connections`, `integration_schedules_bulk_operations`, `integration_workflow_primitives`,
   `integration_run_input_cache`, `integration_claim_transition_efficiency`, the unit tests), fmt, clippy and the python
   checks, and the "seen failing first" evidence; no task is ticked before it. A code change after this re-read goes back
   to a reviewer.

## Nit

2. A rerun or reset of an old ended run can start a version the inventory calls superseded.
3. `MAX_MULTIPLE_OF` decides what v1 admits and is not pinned or in the contract.
4. Technical Plan 5 still says the recheck counts only a refusal.
5. The inventory answers a panic "run it again", and its module doc omits the schedule pin.
6. ADR-0092's options are out of order (1, 2, 3, 4, 6, 7, 5).

Counts: Blocker 0, Major 1, Nit 5
