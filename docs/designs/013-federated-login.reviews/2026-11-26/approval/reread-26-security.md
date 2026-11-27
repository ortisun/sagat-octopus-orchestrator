# Targeted security re-read of design 013 for approval, round 26 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: a1e3101d2d9f8c20cd5bca99313cd117c38ee043
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-25 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the per-operation component check and its test sufficient to keep paths away from the rings.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 12, *Transport*, against Task 013-8's negative fixture.** The fixture refuses a named list of `str`-backed types on `IdentityPath` only, so a newtype over `String` (the existing `SecretName`) or a data-carrying `IdentitySecretName` variant passes, and "no free text" is not enforced. Fix: an allowlist of `IdentityPath` field types (`u32`, the tenant id, the connection id, `IdentitySecretName`) and a fieldless `IdentitySecretName`, with negative fixtures for a `String` newtype field and a data-carrying variant.
