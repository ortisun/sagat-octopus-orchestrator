# Security review of the 2026-11-19 revision of design 041

Doc: docs/designs/041-step-logs.md
Reviewed-commit: 4c027adffaa889109a4b776c148d0f11263f9130
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Counts: Blocker 0, Major 2, Nit 6
Note: a fresh `security-reviewer` given the diff and the files, no conclusions; recorded condensed by the author. Checked and found nothing: the carry's tenant boundary, runtime-role privileges, quota bypass, replay after revocation, injection, metric labels, Debug and spans.

## Blocker

None.

## Major

- S-M1. A carry is accepted from the runner and database-runner roles and on managed steps, which the ingest refuses; script output and managed-step bytes could be kept as step logs, against FR-041-016.
- S-M2. The worst carry the contract allowed (500 records) runs under the run lock in the reserved admission class, unmeasured; other tenants' leases compete for the same pool.

## Nit

- S-N1. The carry is stored in the transaction that proved the output leaked a declared secret; scoped refusals not covered. S-N2. The stream's attempt and generation must be the fenced ones, captured before the engine applies the report. S-N3. FR-041-007 (d) and ADR-0085 rule 5 name only the batch request. S-N4. C1 to C3 run before the lease and replay checks. S-N5. C1 should also check the store's proof, and cutting must be in bytes. S-N6. The seam's feature can be unified into workspace test builds.
