# Targeted security re-read of design 013 for approval, round 14 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: e2601032
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of the third correctness and egm-review re-reads (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found no finding on the key-id classification, the order of the answers before any session lookup, the token endpoint's body, the grace, rate-limit and reuse rules, the resumed rotation, the revoke retry or the lock order of the refresh grant and logout.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Technical Plan 4, Authorize; Technical Plan 12, operator commands.** The attempt ring seals on every anonymous authorize under one key with random nonces, with no stated rate limit or rotation cadence, so the per-key message bound is in an anonymous caller's hands; a stale replica may still seal; there is no `revoke`, though the cookie holds the provider-side PKCE verifier and `nonce`. Fix: a key per attempt derived by HKDF-SHA256 from a random 16-byte salt carried in the cookie; refuse to seal (`503`) when the ring is older than two intervals; add `identity attempt-key revoke` or state why none is needed.
2. **Nit. Technical Plan 4, "Verification order and locking".** The session row is locked `FOR UPDATE` before the MAC is verified, so anyone who knows a `sid` can serialize the owner's refresh, logout and the `revoke` batch behind forged tokens. Fix: read the random value without a lock and verify the MAC; only a token that verifies takes `FOR UPDATE` and re-checks that the random value has not changed.
3. **Nit. Technical Plan 12, "The `cli-refresh` ring"; Task 013-8.** Nothing ties the stored document's path to the purpose `P`, so a wiring mistake could load the attempt document as `IdentityKeyRing<CliRefresh>` and both purposes would share material. Fix: the path is an associated constant of the purpose marker, so `load::<CliRefresh>()` cannot read another path; assert it in a test listed in Architecture decision 0140's "Enforced by".
