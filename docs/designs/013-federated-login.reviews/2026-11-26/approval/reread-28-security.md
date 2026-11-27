# Targeted security re-read of design 013 for approval, round 28 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 5a5ba25902f89c881088bd37e78642d50ba34563
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of the fourth architecture re-read (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the per-request ring snapshots, the closed `key` label, the `revoke` batch order, the `measured` labels and the widened allowlist (no derived `PartialEq` on `Tag`, the sealed supertrait) sound.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Technical Plan 12, *Transport* and *The rule*; Task 013-8; Architecture decision 0140.** `IdentityOpenBao` is "built only from the `SAGAT_IDENTITY_OPENBAO_*` settings" but no constructor is named or allowed, and the existing `OpenBaoSettings::from_lookup` reads only `SAGAT_OPENBAO_*`, so a builder adds `IdentityOpenBao::new(OpenBao)` or a `From<OpenBao>`; wrapping the secrets-mount client by mistake then writes the `cli-refresh` material where every engine's token reaches. Fix: one allowlisted constructor in `identity_ring`, `IdentityOpenBao::from_lookup(get, development)`, reading only the identity settings, with a test that it never reads the secrets-mount names; negative fixtures for a constructor taking an `OpenBao` or `OpenBaoConfig`, a `From<OpenBao>` and a non-private field; the constructor named in "Enforced by".
