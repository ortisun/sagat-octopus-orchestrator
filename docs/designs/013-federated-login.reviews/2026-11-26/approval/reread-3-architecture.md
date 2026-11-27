# Targeted architecture re-read of design 013 for approval, round 3 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: d8a16e9a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a confirmation of round 2 and a re-read of the changes since (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. Round 2's seven findings are closed.
Counts: Blocker 0, Major 1, Nit 6

1. **Major. Technical Plan 2 (`RefreshTokenMac`), Technical Plan 4 "Before the transaction".** The port's `verify` needs the locked row's random value, and no method can do the pre-transaction key check without I/O inside the transaction. Fix: `handle(key_id)` before `BEGIN`, in-memory `verify` and `mint` on the handle, as `TokenSigner` does.
2. **Nit. Technical Plans 4 and 12.** Nothing drives "the key set's refresh" for the two rings. Fix: a background task re-reading both rings each interval.
3. **Nit. Technical Plan 4, Architecture decision 0140 decision 7.** Gating logout on a stale ring stops a safe revocation in an outage. Fix: gate minting paths only; a test.
4. **Nit. Technical Plan 3.** The grace counter has no column. Fix: add it.
5. **Nit. Tasks.** Two tests belong to no task. Fix: place them.
6. **Nit. Task 013-8, Architecture decision 0140 decision 11.** The derivation function is unnamed though part of the token format; the crates are missing; `KeyRing` must keep bytes by id. Fix: name HKDF-SHA256 with its salt and `info`; the crates; a test.
7. **Nit. Requirement 013-025.** The intent-write-verify-activate clause now covers the rings. Fix: limit it to the connections' secrets.
