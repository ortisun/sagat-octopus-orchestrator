# Targeted security re-read of design 013 for approval, round 35 (2026-11-26)

Doc: docs/designs/013-federated-login.md
Reviewed-commit: d38756f841140d5278f91cad4f979d01a808ec5d
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-34 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept.
Counts: Blocker 0, Major 1, Nit 0

1. **Major. Technical Plan 12, *Errors and printing*.** The exact `[REDACTED]` requirement covers only `Tag` and `RefreshMac`, not `RingKey` and `RingDocument`, which hold the real key material, and `PurposeKey` is not formatted at all; a builder reads the split as deliberate and checks the key types for one encoding only, so a later `Debug` printing base64 or the decimal byte list leaks the HMAC root of every CLI refresh token. Fix: the exact requirement for `RingKey`, `PurposeKey`, `RingDocument`, `Tag` and `RefreshMac`, built from distinctive bytes; for the ring, adapters, errors, audit rows and log lines, the distinctive bytes in no form (hex either case, base64 either alphabet with or without padding, the decimal byte list).
