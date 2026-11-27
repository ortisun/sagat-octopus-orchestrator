# Targeted security re-read of design 013 for approval, round 7 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 4dea6eaa
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fix of round 6 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged forgery closed (the purpose key never in the database) and reuse detection sound without grace rows.
Counts: Blocker 0, Major 2, Nit 4

1. **Major. Technical Plans 3 and 6, Task 013-13.** Nothing deletes the grace rows, so the rate limit is the only bound (60,480 rows per session per week); the storage test states no bound. Fix: delete a session's old rows in each refresh, purge those of ended sessions, assert at most seven rows, decide the grace by time.
2. **Major. Requirement 013-026, `crates/infra-secrets/src/lib.rs`.** The token names its key id but `derive_purpose_key` takes no key selector and the ring keeps no bytes by id, so a ring rotation fails every live CLI token, or invites a loosened check. Fix: `derive_purpose_key(key_id, label, info)`, verify under the token's key, mint under the active key, retire a key only after 7 days; two tests.
3. **Nit. Requirement 013-026.** The order of the rate limit, the MAC and reuse detection, and the counter's place, are unstated; forged requests could exhaust the limit. Fix: MAC, then class, then limit; columns on the session; a test.
4. **Nit. Requirement 013-026.** A deterministic MAC makes a reissued or restored generation an exact earlier token; a generation above the current one is unspecified. Fix: never reuse a number, refuse above-current, rotate the random value after a restore.
5. **Nit. Technical Plan 3.** The row still lists a refresh token hash. Fix: the random value and the current generation.
6. **Nit. Requirement 013-026, the logout route.** No constant-time comparison is required; logout by refresh token has no forged-token test. Fix: `Mac::verify_slice`; a test.
