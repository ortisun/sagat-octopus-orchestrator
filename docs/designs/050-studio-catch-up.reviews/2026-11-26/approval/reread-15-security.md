# Targeted security re-read of design 050 for approval, round 15 (2026-11-26)

Doc: docs/designs/050-studio-catch-up.md
Reviewed-commit: eebdf737d53acb9be8ffb2b715fe135fec3bb66e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: a re-read of the round-14 fix (Owner decision 2026-11-25-2, step 4); condensed by the author, every severity and fix kept. The reviewer found the order admission, size cap, decoding, key lookup sound, and an unknown key id answered with the same `400`.
Counts: Blocker 0, Major 0, Nit 1

1. **Nit. Technical Plan 6, "Cursor sealing"; Task 050-4.** The canonical form does not require zero trailing bits in base64url, so a lenient decoder accepts several spellings of one salt or ciphertext; no forgery, but "one canonical form" is not delivered or tested. Fix: require zero trailing bits (the decoder's strict default) and test a salt whose last character sets one.
