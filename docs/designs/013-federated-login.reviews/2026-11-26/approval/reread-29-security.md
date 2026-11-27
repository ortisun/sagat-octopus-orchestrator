# Targeted security re-read of design 013 for approval, round 29 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: bc3e6c6d7a039feb533cc9d98ee71d5ee954bb26
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-28 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 1, Nit 1

1. **Major. Technical Plan 12, *Transport*; Task 013-8; Architecture decision 0140.** The newtype stops a wrong Rust value, not wrong settings: `SAGAT_IDENTITY_OPENBAO_TOKEN_FILE` pointing at the secrets mount's token sink and the mount set to `secret` make `rotate` write the ring material with the secrets-mount token, against Requirement 013-025; and the test that the constructor never reads `SAGAT_OPENBAO_*` rules out a comparison. Fix: the token check refuses a token whose policies are not exactly `identity` and `default`; `an_identity_client_refuses_a_token_without_only_the_identity_policy`; a development client refused outside `SAGAT_ENV=dev`.
2. **Nit. Technical Plan 12, *Transport* and *The rule*.** "Reads only the `SAGAT_IDENTITY_OPENBAO_*` settings" cannot hold, since the client needs the address, timeout, CA and development flag; a builder passing `development` straight through would skip the two-key gate and the token check; and `IdentityOpenBao` is missing from the exact `pub use` list. Fix: name the shared transport settings with the same two-key gate, reword "only" to "never `SAGAT_OPENBAO_MOUNT` or `SAGAT_OPENBAO_TOKEN_FILE`", add `IdentityOpenBao` to the `pub use` list.
