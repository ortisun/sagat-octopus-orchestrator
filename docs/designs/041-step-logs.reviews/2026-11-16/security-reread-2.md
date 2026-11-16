# Security confirmation check of design 041 (second re-read)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fac00787bcaea68b47252139aadbe20bdfc660a5
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a fresh `security-reviewer`, asked whether the fixes after its re-read introduce a new Blocker or Major; no shell. Recorded condensed.

Checked: the tenant boundary of the marker stream and of `stream`; `may_read` before `stream` (closes the re-read's Nit 1); the listing's `logTruncated` test; the new end rule; the marker's failure path; grants; the ADR Notes.

## Blocker

None.

## Major

None.

## Nit

1. The marker's read of the stored `script_executions.expires_at` does not name the tenant.
2. T041-13's "see no script item" should assert the two `403` answers of the error table.

Counts: Blocker 0, Major 0, Nit 2
