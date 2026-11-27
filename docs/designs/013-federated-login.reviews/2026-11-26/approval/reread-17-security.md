# Targeted security re-read of design 013 for approval, round 17 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: eebdf737d53acb9be8ffb2b715fe135fec3bb66e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-16 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the attempt cookie's refusal before any lookup, the length caps, the per-purpose paths, `KeyRing` unchanged and Architecture decision 0140 matching the design.
Counts: Blocker 0, Major 1, Nit 2

1. **Major. Technical Plan 12, "The `cli-refresh` ring"; Technical Plan 2; Tasks 013-8 and 013-9.** The key bytes and the derivation are private to the ring's module, but the ring's public operations are not specified, so the `RefreshTokenMac` and `AttemptSealer` adapters, elsewhere in the crate and built by another task, cannot compute a MAC or seal an attempt; the easiest way out, a public accessor of the purpose key under another name, escapes the call-site rule and puts derived keys outside the module. Fix: specify the ring's whole public surface, none of it returning key material (`mint` and a constant-time `verify` for `CliRefresh`, `seal` and `open` for `SigninAttempt`); the adapters call only these; the rule refuses a public function whose return type contains a key.
2. **Nit. Technical Plan 12, `derive_purpose_key(key_id, salt)`.** The per-purpose salt rule is a plain argument, so an attempt key derived without a salt compiles and every attempt would share one key. Fix: the salt an associated type of the purpose marker (`()` for `CliRefresh`, `[u8; 16]` for `SigninAttempt`).
3. **Nit. Technical Plan 4, "Format and MAC" and Authorize.** The `<mac>` and the cookie's base64url fields have free trailing bits, so "one spelling" is not delivered or tested. Fix: require zero trailing bits and test a variant `<mac>` and cookie.
