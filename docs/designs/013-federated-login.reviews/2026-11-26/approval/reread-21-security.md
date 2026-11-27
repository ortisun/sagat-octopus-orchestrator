# Targeted security re-read of design 013 for approval, round 21 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: bbe54dad9cb5c36e7513a972990a967f2d7a56cc
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-20 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the error and printing redaction, the private derivation and newtypes, the attempt salt, the constant-time verify, the `rotate` and `revoke` sequencing and the stale-replica bound sound.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 12, *Errors and printing*; Technical Plan 2; Architecture decision 0140.** The existing OpenBao client reads only version 1 of credential paths and writes only with `cas: 0`, so the builder must add a raw versioned read and a check-and-set write in `openbao.rs`, outside `identity_ring`, where any crate code holding the client could read a ring document and its material; no rule catches it, and "alone holds the material" does not hold. Fix: name the two transport operations, taking a `RingPath` constructible only inside `identity_ring` from `P::PATH`, in an `identity_ring` submodule, with bounded bodies and no body in errors; the rule refuses calls of them outside the module; a negative fixture for a raw-body read in `openbao.rs`.
2. **Nit. Technical Plan 12, *Types* and *The rule*; Technical Plan 2.** `RingPurpose` is allowlisted and not sealed, so outside code can implement it with the `cli-refresh` path and a label built with `concat!`, loading that material under a foreign label. Fix: seal `RingPurpose` with a private supertrait; the rule refuses any `impl RingPurpose` outside the module, with a negative fixture.
