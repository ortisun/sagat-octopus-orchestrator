# Targeted security re-read of design 013 for approval, round 10 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: db9e8a50
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a confirmation of the fixes of round 9 (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer judged all four findings of round 9 closed and none reopened.
Counts: Blocker 0, Major 0, Nit 3

1. **Nit. Requirement 013-026, Technical Plan 2.** "Built only by the identity-mount loader" is unenforced: a public constructor or `From<KeyRing>` would let the secrets ring be wrapped. Fix: name the crate; the loader is the only constructor.
2. **Nit. Requirement 013-026, Risks.** Revoke drops only the active key, so a retained predecessor or a pending key from a leaked mount still verifies. Fix: drop every key revoke did not create, cancel an unfinished rotation; test from mid-rotation.
3. **Nit. Requirement 013-026, Technical Plan 12.** The derivation's owner still names Task 013-9; the commands paragraph omits `revoke`. Fix: Task 013-8; `rotate|revoke`; the Refs.
