# Targeted re-read 2 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: e56630a20aa88aaa94f8625743bd86bdffb8c881
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read
of the fixes of re-read 1, not a full review. The reviewer read files only and ran nothing.

Re-read 1: Major 1, Major 3, Nits 4 and 6 resolved; Major 2 partly (Major 1 below); Nits 1, 2, 3, 5, 7 partly (Nits 2,
1, 3, 4, 8 below).

## Major

1. The confirmation's `WriteTx` inserts into `server_incarnation`, on which the runtime role has no privilege, so after
   any crash restart or promotion the insert fails with `42501` and no process confirms. Fix: a definer function inserts,
   reads back and returns the token.

## Nit

1. The pre-authentication refusal (`crates/api-http/src/limits.rs`) already shows a reason; adding the new ones there
   would show them to anonymous callers.
2. The lint omits `END` and `ABORT`.
3. Task 048-4 checks that `denyCidrs` cover a namespace, which is not a range.
4. Clean stops delete registry rows, so "no row" also describes a stopped platform; and a command's `after_connect` has
   no confirmed identity. Fix: a durable marker; a command adopts the confirmed identity.
5. The 22 s bound omits a request's statement timeout (5 s) and grows with the commit deadline.
6. `instance_confirm_durability()` takes no credential, so any `sagat_app` session could write another process's
   confirmed identity. Fix: check it against `token_hash`; read the identity inside.
7. (Suspicion.) A cancel that reaches a waiting commit returns success with a `WARNING`, counted as applied; the text
   lists a cancel among the errors.
8. The connection budget names one autoscaler's ceiling where the chart has two, and doubles only the request pool.

Counts: Blocker 0, Major 1, Nit 8

All nine were fixed at `5c2168b9`; for Nit 7 the text is corrected and names the cases (the platform sends no cancel,
Task 048-6 establishes the statement timeout and records the answer, an operator's cancel is the stated residual).
