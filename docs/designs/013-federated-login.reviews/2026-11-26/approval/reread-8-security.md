# Targeted security re-read of design 013 for approval, round 8 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 9a19411f
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fixes of round 7 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged round 7's Major 1 and Nits 3 to 6 closed.
Counts: Blocker 0, Major 1, Nit 2

1. **Major. Requirement 013-026, the operator commands.** The ring is unnamed: the attempt-sealing ring's own command drops old keys after minutes, failing every CLI token at a rotation; `SAGAT_SECRETS_KEYS` is held by every engine, which with database access could forge refresh tokens for any session (against Requirement 013-025). Fix: a dedicated `cli-refresh` ring in the identity mount, two-step rotation by its own command that keeps the old key 7 days and one interval, a test of the command, an architecture rule against the secrets ring, one task owning the `KeyRing` change.
2. **Nit. Requirement 013-026, Task 013-9.** A fixed window lets 12 grace rows coexist across a boundary, so "at most seven" is false. Fix: state 12 or slide the window; a burst in the test.
3. **Nit. Technical Plan 3.** A lower generation with no row is unspecified. Fix: outside the grace, its reuse revokes; the test includes a deleting refresh.
