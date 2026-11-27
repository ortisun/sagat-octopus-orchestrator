# Targeted security re-read of design 050 for approval, round 4 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 86b9f140
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fixes of round 3 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged round 3's three findings closed with no new defect.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 6, Architecture decision 0144 decision 5.** One purpose key for the whole deployment still spends a random-nonce budget across all tenants. Fix: the tenant in the HKDF `info`, or a nonce-misuse-resistant cipher.
2. **Nit. Task 050-4, `crates/infra-secrets/src/lib.rs`.** `KeyRing` cannot derive a purpose key today and no task names it; a builder could fall back to the ring's own `seal`. Fix: `KeyRing::derive_purpose_key`, and a test that the ring's `open` refuses a cursor.
3. **Nit. Task 050-5, Technical Plan 6.** The publication list's sealing, its `503` and its separate label have no tests; the 512-byte bound has none on either route. Fix: name them.
