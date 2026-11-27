# Targeted security re-read of design 013 for approval, round 23 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 7e6a16d25222661f03a9db4fbd524a165a73b348
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-22 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the child-module placement, `RingPath` and `RingDocument`, the printing test, the reuse of the request helper and body bound, and the credentials policies refusing ring paths sound.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Technical Plan 12, *Transport* and *The rule*; Technical Plan 2; Task 013-8.** "No other module gains a raw read of the identity mount" is not enforced: the rule refuses neither a second child module of `openbao` nor a new non-private path-taking helper in `openbao.rs`, and `TokenSigner` and `IdentitySecrets` must reach the identity mount somehow, so a builder adds one of those; a path built from connection input then reaches `rings/cli-refresh` outside the ring module. Fix: the rule refuses any other child module of `openbao` and any non-private item of `openbao.rs` beyond an enumerated list; name the one accessor `TokenSigner` and `IdentitySecrets` use, with a path type built only from fixed prefixes other than `rings/` and checked components; negative fixtures for `mod identity_secrets;` and a `pub(crate) fn` taking a `&str` path.
