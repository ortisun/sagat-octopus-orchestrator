# Targeted security re-read of design 013 for approval, round 9 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: dcfd91b1
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fixes of round 8 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged the dedicated ring sound (engines can no longer forge refresh tokens) and round 8's two Nits closed.
Counts: Blocker 0, Major 1, Nit 3

1. **Major. Tasks 013-8 and 013-9, Technical Plan 2.** The guards of the dedicated ring (the `KeyRing` change, the ring's loading, the rotate command, its test, the architecture rule and its fixture) are in no task, and Technical Plan 2 still lists three rules. Fix: name them in the tasks; four rules; the architecture check in the verification.
2. **Nit. Requirement 013-026.** Both rings are one type, so a static rule cannot tell which reaches the `cli-refresh` derivation. Fix: an `IdentityKeyRing` type the refresh code takes, the label confined to its module.
3. **Nit. Technical Plan 12.** Nothing creates the first `cli-refresh` key, inviting a fallback to the secrets ring. Fix: `rotate` on an empty ring creates one; rollout runs it; start-up refuses an empty ring.
4. **Nit. Requirement 013-026, Risks.** A leaked `cli-refresh` key stays accepted 7 days with no revoke. Fix: `identity cli-refresh-key revoke` that replaces the key and rotates every random value, or the rotation on the emergency page.
