# Targeted security re-read of design 013 for approval, round 18 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 98609559009f7b0d30699273f9c602559f831a1a
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-17 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 12, "The `cli-refresh` ring"; Task 013-8; Architecture decision 0140 "Enforced by".** "Its public operations return no key material" is checked only on return types; the new `IdentityKeyRing<P>`, which does not wrap `KeyRing`, inherits none of `KeyRing`'s `Zeroizing` material, redacting `Debug` or its test, so a derived `Debug` on the ring or an adapter would write every key into a log. Fix: key bytes and derived keys held as `Zeroizing`, a hand-written `Debug` showing only ids and states, no `Serialize` or copying `Clone`; `an_identity_key_ring_never_prints_key_material` over the ring and both adapters, listed in "Enforced by".
2. **Nit. Technical Plan 2; Technical Plan 12; Task 013-8.** A text rule on "a public function returning a key" cannot tell what a key is, passes a `pub(crate)` function returning `Hmac<Sha256>` and an `AsRef<[u8]>` on the key type. Fix: module-private newtypes for key bytes and derived keys; the rule refuses any non-private item whose signature names them or `Hmac`, `Hkdf`, `Aes256Gcm`; a `pub(crate) fn -> Hmac<Sha256>` negative fixture.
