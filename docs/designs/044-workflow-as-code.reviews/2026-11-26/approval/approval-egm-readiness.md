# Targeted re-read of design 044 for approval: egm implementation readiness, confirmation (2026-11-26)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 14513c8533b0a0a475d77fe21f3886c4e07c52cc
Reviewer: egm-implementation-readiness, implementation-readiness standard, the text changed since f7ddcbbe
Note: condensed by the author from the reviewer's output; every finding and severity is kept. All three Majors of the
first pass (`reread-1-readiness.md`) are closed; Nits 4 to 8 closed and 9 partly closed. This lens report is
summarized in `approval-egm-review.md`.

## Major

1. A run at its 32-blob ceiling has no specified way out: the ownership arm and the commit refuse an earlier lease's
   blob, a new lease cannot learn its id, and the 24-hour sweep comes after the review.
2. The item's outcome after a verifier session is no longer live is not stated, so the red test cannot assert it.

## Nit

3. Thirteen new tests sit in Task 044-24 while their behaviour is built in Tasks 044-6, 044-9 and 044-13.
4. Task 044-9's `blobs.rs` note and the files of the application's blob module and the `WriteBlob` handler.
5. The `retry-after-ms` of the two ceiling refusals.
6. Precedence when two refusal reasons hold at once.
7. The setting should end in `_SECS` and be listed in `alerts.md`.
8. The `WorkflowTaskLease` table row is split across two lines.
9. The union classification of a child's output has no implementing task.

Counts: Blocker 0, Major 2, Nit 7
