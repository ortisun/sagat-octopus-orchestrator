# Targeted correctness re-read of design 013 for approval, round 2 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 051e5f47
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: a re-read of the text changed since round 1 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found its ten round-1 findings still closed and the lock orders free of cycles.
Counts: Blocker 0, Major 1, Nit 7

1. **Major. Requirement 013-026, Technical Plan 12.** "Kept on a failed read" contradicts "a stale replica mints under a dropped key for at most one interval", and promotion after one interval assumes every replica read; a replica with failing reads refuses new-key tokens and mints under a dropped key indefinitely. Fix: refuse after two intervals without a successful read, one re-read on an unknown key id, promote after two intervals, a resume rule, a test.
2. **Nit. Requirement 013-026, Technical Plan 11.** The rate-limit and empty-ring refusals have no answer on the token endpoint, and an `invalid_grant` would make the client discard the token. Fix: `429 REFRESH_RATE_LIMITED`, a retryable `503`, a CLI test.
3. **Nit. Requirement 013-026.** The limit applies to grace rotations whose window is shorter than the limit's, so an honest retry can be revoked. Fix: exempt grace rotations.
4. **Nit. Requirement 013-026, Technical Plan 3.** Whether a retired generation has a row is unstated, so a concurrent-refresh test is ambiguous. Fix: no row, its use revokes; the test keeps the second token.
5. **Nit. Requirement 013-026.** A refresh verifying the MAC on an unlocked row can promote a token of an old random value past a `revoke`. Fix: verify and mint from the row locked `FOR UPDATE`; a barrier test.
6. **Nit. Technical Plan 6.** A guarded demotion audits and locks on every pass. Fix: audit once, retry when the administrator count changed, upsert the row.
7. **Nit. Technical Plan 13.** The restart after a stale role read has no test. Fix: a barrier test.
8. **Nit. Technical Plan 4 step 4.** The confirmation never consumes the attempt, so a double click links twice. Fix: consume it in the transaction; a test.
