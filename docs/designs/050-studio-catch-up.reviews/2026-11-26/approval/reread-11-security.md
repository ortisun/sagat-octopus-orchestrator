# Targeted security re-read of design 050 for approval, round 11 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: e28824eb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-2 changes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged a cursor key in PostgreSQL acceptable for what the cursor protects (every reader of the key can read the rows; a forged cursor only picks a scan position; tenant, namespace and caller bind it) and found no weakened boundary.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 6, Task 050-4.** Which role creates and rotates the key is unstated; the runtime role cannot insert, and random ids make `ON CONFLICT` useless for concurrent first starts. Fix: the first key in `control-plane migrate`, rotation with the schema-owning role, `SELECT` only for the runtime role.
2. **Nit. Technical Plan 6, Architecture decision 0144.** "Bounded by its own rate limit" bounds a rate, not a total. Fix: the arithmetic and a yearly rotation.
3. **Nit. Technical Plan 6, Architecture decision 0144.** The "not a secret" argument needs the bytes never to leave the database. Fix: redacted in `Debug`, never logged or printed, never exported; a test on `rotate`'s output.
