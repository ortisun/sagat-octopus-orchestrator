# Targeted re-read 7 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: f0a8115d8fad8bd2981860c3401bc1a9fcfafa26
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 6, not a full review. The reviewer read files only and ran nothing.

Re-read 6: Nits 1 to 3 resolved; the Major partly (Nit 1 below).

No Blocker, no Major.

## Nit

1. The general admission rule and the refusal step still list only the registry's and a waiting command's connection.
2. Once the fresh check passes, a command's connections stay admitted on a null token for its whole run, even after a
   process confirms and the primary crash-restarts. Fix: only while the table stays empty, or adopt the first confirmed
   identity.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
