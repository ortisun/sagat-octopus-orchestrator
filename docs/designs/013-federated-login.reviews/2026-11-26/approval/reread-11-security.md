# Targeted security re-read of design 013 for approval, round 11 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 0aa18b84
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fixes of round 10 (Owner decision 2026-11-25-2, step 4); condensed by the author. The reviewer judged all three findings of round 10 closed (module privacy enforces the constructor, the owner and commands are named).
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Requirement 013-026, Technical Plan 12, Task 013-8.** "Cancels any unfinished rotation" says not how: a `rotate` waiting between its two steps could overwrite revoke's ring and restore the leaked key. Fix: check-and-set writes on the ring's version; the second step promotes only from its own first step's version, otherwise stops audited; a barrier test.
