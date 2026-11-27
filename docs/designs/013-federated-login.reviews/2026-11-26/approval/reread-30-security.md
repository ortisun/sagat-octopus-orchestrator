# Targeted security re-read of design 013 for approval, round 30 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: b644f65e6ca72d68b7e4f38415584de416ef636c
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-29 fixes (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Technical Plan 12, *Transport*; Task 013-8; Architecture decision 0140.** The policy check runs only "at start", while the operator commands never go through the control plane's start and the per-request token check of `openbao.rs` refuses only a root policy or a bad TTL; a `rotate` with a secrets-policy token file still writes the ring material into the secrets mount, and a token file changed after start is never checked. Fix: the policy rule is part of every token read the identity client makes, refusing before any ring or identity-mount request; the test drives `rotate_begin` with a secrets-policy token file and swaps the token file after one successful request.
