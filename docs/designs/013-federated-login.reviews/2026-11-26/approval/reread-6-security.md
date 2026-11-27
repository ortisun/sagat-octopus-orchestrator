# Targeted security re-read of design 013 for approval, round 6 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 10338cd5
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fix of round 5 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged round 5's finding closed (no forged token revokes or gets a successor) and timing not a finding.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Technical Plan 3, Requirement 013-026.** Keeping every generation's hash for the session's life, with no limit on how often a session refreshes, lets any CLI session add rows without bound to a table all tenants share. Fix: a ceiling and a rate limit, or derive each generation's secret as an HMAC under a purpose key so no per-generation row is needed.
