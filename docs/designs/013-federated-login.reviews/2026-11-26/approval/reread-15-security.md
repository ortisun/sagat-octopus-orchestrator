# Targeted security re-read of design 013 for approval, round 15 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 1854e0bca46de49b47d983fbc250d678911af029
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the changes since round 13 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the refusal order before any session lookup, the forged token's lock-free path, the rate limit after the MAC, the lock order, the OpenBao read bound, the monotonic key ids and per-purpose paths, the stale verifier reachable only by logout and the authorize `503` sound.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 4, Authorize; Technical Plan 12, `identity attempt-key revoke`.** A stale replica stops sealing but keeps opening under its cached ring, so a key that `revoke` dropped still opens forged attempts there, defeating the browser binding and "every sign-in in flight fails". Fix: open nothing once the last successful read is older than two intervals plus the 10-minute attempt lifetime; extend `an_attempt_key_revoke_ends_every_sign_in_in_flight` with a replica that cannot read the ring.
2. **Nit. Architecture decision 0140 "Enforced by"; Technical Plan 12; Task 013-8.** `the_secrets_ring_cannot_reach_the_derivation` cannot be a runtime test: `derive_purpose_key` is `pub(crate)` on `KeyRing`, so `SecretCipher`'s code in the same crate can call it. Fix: confine the derivation to the identity ring's module and enforce it by a check that fails on a call from `SecretCipher`'s module or another crate.
3. **Nit. Technical Plan 4, "Before the transaction".** The token's fields have no stated syntax or range, so an id like `0`, one beyond the integer range or a huge one could reach the re-read branch or panic. Fix: each field parses to a stated bounded type, anything else `invalid_grant` before any ring or session lookup, with one test.
