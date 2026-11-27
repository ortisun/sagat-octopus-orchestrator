# Targeted security re-read of design 013 for approval, round 33 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: 34a282d5e36e93ae99d939852ddb1eb51617fd7d
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the fixes of the fifth architecture re-read (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the production role binding failing closed, only the agent sidecars able to log in, the fixed policy name, the settings isolation, the development token's directory, the closed metric names and the sealed supertrait sound.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 2, port row; Technical Plan 12, *The rule*.** The new `RefreshMac([u8; 32])` and `Tag` carry the MAC, the only part of a refresh token the server does not store, but nothing says how they print, so a derived `Debug` in a traced redemption would log a usable token. Fix: a hand-written `Debug` printing `[REDACTED]`, no `Display` or `Serialize`; call the tag the secret part of a refresh token; add both to `an_identity_key_ring_never_prints_key_material`.
