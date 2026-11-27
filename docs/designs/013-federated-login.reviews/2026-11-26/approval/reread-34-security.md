# Targeted security re-read of design 013 for approval, round 34 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 04d6a1ceb8edcdcbaa6cb679bcbe72c2ead86f52
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-33 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 12, *Errors and printing* and *The rule*.** The architecture rule cannot reach the application's `RefreshMac`, so the printing test is its only guard, and a test that only looks for the base64url of the bytes passes a derived `Debug` printing the byte array. Fix: the test asserts that the `{:?}` and `{:#?}` output of `Tag` and `RefreshMac`, built from distinctive bytes, is exactly `[REDACTED]`.
