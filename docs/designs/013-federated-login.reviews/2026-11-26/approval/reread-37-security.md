# Targeted security re-read of design 013 for approval, round 37 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: aa2175e872d62154f264eb6463cca352476710a9
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-36 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the 16-character windows and the unsalted derivation consistent with the document.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 12, *Errors and printing*.** Nothing ties the test's own derivation to the ring's, so a builder deriving differently makes the search look for a key the code never produces. Fix: the test mints a tag with the ring and checks that HMAC-SHA256 under its own derived key over the same inputs equals it.
