# Targeted security re-read of design 013 for approval, round 22 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 6dadf15625adba9dc3399597c6bbe0b9a4f58b03
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-21 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the sealed `RingPurpose`, the policies refusing ring paths to the credentials token, the check-and-set on a missing document, the response ceiling and Architecture decision 0140 sound.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 12, *Transport*; Technical Plan 2; Task 013-8.** The transport is "built on the client's request helper and its bounded body", but `OpenBao::request`, `bounded_body`, `url` and the client's fields are private to `openbao`, and a sibling module cannot reach them; the natural widening to `pub(crate)` gives every module of the crate, `TokenSigner` and `IdentitySecrets` included, a raw read of the ring documents through the public `P::PATH`, and no rule catches a generic helper. Fix: state the seam and its visibility, extend the rule to `openbao.rs`, and refuse a `pub(crate)` request or body helper there by a negative fixture. (The author placed `identity_ring` as a child of `openbao`, so the helpers stay private and need no widening.)
2. **Nit. Technical Plan 12, *Transport*.** The transport's `Zeroizing<Vec<u8>>` document prints every key in base64 through `Debug`, and the printing test does not format it. Fix: a module-private `RingDocument` newtype with no `Clone` and a redacting `Debug`, formatted by the test.
