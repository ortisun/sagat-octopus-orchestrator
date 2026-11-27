# Targeted security re-read of design 013 for approval, round 4 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 2d71be9c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the text changed since 66189ab7 by the architecture, correctness and egm-review re-reads (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the signing handle, the revoked-handle check, the two-step attempt key, the lock-strength rule, the code tombstone, the health row, the recompute rows and the activation lag sound, and noted (outside its lens) that the confirmation's lock order would deadlock with a redemption.
Counts: Blocker 0, Major 2, Nit 2

1. **Major. Technical Plans 6 and 13.** A guarded demotion is never retried, so in an administrator handover recomputed in subject order the old administrator keeps `admin` indefinitely. Fix: promotions before demotions; a durable guarded row retried every pass; two tests.
2. **Major. Technical Plans 3 and 4, Requirement 013-026.** With generation rows kept 30 seconds and an opaque token, a reuse after the purge cannot be tied to its session, so a thief who refreshed first keeps the session; step 6 also signs the refresh token with the access-token key against Requirement 013-018. Fix: `<sid>.<generation>.<random>`, any lower generation outside the grace revokes; no signing of the refresh token; two tests.
3. **Nit. Technical Plan 4 "Redeem".** `SESSION_CONFLICT` is answered before PKCE, an oracle to a holder of an intercepted code. Fix: check client, redirect URI and PKCE first, one `invalid_grant`.
4. **Nit. `specs/029-custom-roles-folders/contracts/group-source.md`.** The restart under the exclusive fence is missing from the contract 029's builders read. Fix: add it with the test's name.
