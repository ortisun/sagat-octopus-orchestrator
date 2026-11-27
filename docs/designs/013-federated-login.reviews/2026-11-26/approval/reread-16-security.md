# Targeted security re-read of design 013 for approval, round 16 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 5e179ac2296952a50b52f6f1dab232c0bb37d4bb
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-15 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the stale-replica cut-offs for sealing and opening, the attempt-key `revoke` and the test lists sound.
Counts: Blocker 0, Major 0, Nit 2

1. **Nit. Technical Plan 4, Authorize, against "Format and MAC".** The attempt cookie, fully attacker-controlled and carrying a key id, has no stated format, length, key-id rule or salt and nonce lengths, nor a refusal before the ring is consulted; and the refresh token's fields accept several spellings (`+5`, `005`, braced or unhyphenated UUIDs). Fix: the cookie `<key id>.<salt>.<nonce>.<ciphertext>`, at most 1 KiB, a decimal key id, a 16-byte salt and a 12-byte nonce, a mismatch refused as an unknown attempt before any ring lookup, with `a_malformed_attempt_cookie_is_refused_before_any_lookup`; canonical refresh-token fields (decimal without sign or leading zero, the lowercase hyphenated UUID).
2. **Nit. Technical Plan 2, Technical Plan 12, Task 013-8, Architecture decision 0140 "Enforced by".** The architecture rule is worded two ways, and "appear only in the module of `IdentityKeyRing<P>`" cannot hold while `derive_purpose_key` is a method of `KeyRing` in the crate root beside `impl SecretCipher for KeyRing`; a substring match on the label would also hit the command `identity cli-refresh-key`. Fix: move the byte map and the derivation into the `IdentityKeyRing` module, make the label an associated constant of the purpose marker, and have the rule match call sites and exact literals.
