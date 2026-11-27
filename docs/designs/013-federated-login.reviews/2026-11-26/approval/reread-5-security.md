# Targeted security re-read of design 013 for approval, round 5 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 86b9f140
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fixes of round 4 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged findings 1, 3 and 4 closed and the deadlock fix sound.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Requirement 013-026, Technical Plan 3, Task 013-9.** With an unsigned token and generation rows purged after 30 seconds, the server revokes on a token whose secret it cannot check, so anyone who knows a session's `sid` can end it with `<sid>.0.<any bytes>`, repeatedly, raising false theft alarms; if the grace also skipped the check, a forged lower generation would get a successor. Fix: keep every generation's hash for the session's life (or an HMAC under a purpose key); every path checks the secret first, a failure is `invalid_grant` with no effect and the same answer whether the session exists; two tests.
