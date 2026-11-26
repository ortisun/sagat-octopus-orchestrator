# Targeted security re-read of design 050 for approval, round 14 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: 5e179ac2296952a50b52f6f1dab232c0bb37d4bb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-13 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the length-prefixed fields and the `Subject` binding sound.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 6, "Cursor sealing"; Task 050-4.** The key id's encoding (decimal or base64url) is ambiguous, `+1` and `01` parse as the same id, and the salt and nonce lengths are unstated, so an adapter that builds the nonce from an unchecked slice panics on an 11-byte nonce. Fix: the key id decimal `[1-9][0-9]{0,18}` checked against `i64::MAX`; the other fields base64url without padding, salt exactly 16 bytes, nonce exactly 12, ciphertext at least 16; any mismatch the malformed-cursor `400` before any key lookup; a test with `+1`, `01`, `0`, a 15-byte salt and an 11-byte nonce.
