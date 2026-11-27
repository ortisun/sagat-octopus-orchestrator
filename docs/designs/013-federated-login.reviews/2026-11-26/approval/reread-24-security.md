# Targeted security re-read of design 013 for approval, round 24 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: dfdc731969336b12acb8062680e1d0862b1dc3db
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-23 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Technical Plan 12, *The rule*; Technical Plan 2; Task 013-8.** The new child `openbao::identity_mount` can call the client's private helpers, but the allowlist covers only `identity_ring`, so a builder adding a `pub(crate)` raw-path read in `identity_mount.rs`, or an `IdentityPath` variant carrying a `String`, passes every clause and lets any module read the rings; and the enumeration of `openbao.rs`'s public items omits the `pub(crate) mod identity_mount;` and the re-export of `identity_ring` that the design itself needs, inviting an ad hoc relaxation. Fix: extend the allowlist to `identity_mount` (only `IdentityPath`, with fixed variants, and named operations taking it); refuse a non-private function of either child taking a `&str`, `String`, `Url` or `ProviderReference` path; add the two items to the enumeration; negative fixtures for a `&str`-path function in `identity_mount.rs` and a `String`-carrying `IdentityPath` variant.
