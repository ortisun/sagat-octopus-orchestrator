# Security review of design 044, round 11 (2026-11-25)

Doc: docs/designs/044-workflow-as-code.md
Reviewed-commit: 9bdb707d84f3fef3c190979cde128256f2b8159a
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Round 10's Major 1 is
closed and 2 partly closed; Nits 3 to 8 closed. No finding on the tenant boundary, secrets (outside Major 4),
authorization (outside Major 1), injection, the runtime role and definer functions, or the races.

## Major

1. Chain-addressed updates and signals of a code run lose their chain-scoped idempotency: a retry after
   continue-as-new admits a second update (a second charge): key code updates by the address used; keep
   `chain.signal:{chain}`; a chain receipt read; tests.
2. The blob-validation semaphore has no capacity or full-semaphore outcome, so other tenants' load becomes this
   tenant's reviews or counted failures: a capacity, a deadline, `UNAVAILABLE` not counted; tests.
3. A gated activity's input or an approval's subject can be a blob, which approvers cannot see: always inline; the SDK
   learns gated entries at registration; the completion refuses a blob there; a test.
4. No rule names which version's carrier classifies a moved or reset run's stored signals and input, so a move can
   declassify a `secret` field: classify by the accepting version's carrier; a test.

## Nit

5. Ownership and the `secret` refusal should be checked before reading a blob's bytes.
6. Blob references in the export should carry the size only.
7. The secret placeholder for query and replay-check tokens.
8. Literal flow-control keys can fill the namespace's live-keys ceiling.
9. Grants for `code_workflow_types`.
10. The reconciler's fairness between tenants.
11. A verdict from a session whose binding was revoked.
12. A child's `secret` output field reaches the parent's function unless its entry repeats the annotation.

Counts: Blocker 0, Major 4, Nit 8
