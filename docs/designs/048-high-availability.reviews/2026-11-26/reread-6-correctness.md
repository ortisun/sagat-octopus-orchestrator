# Targeted re-read 6 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 5e874cbfc29024bfad5cb7e0b2e80716ba795cd7
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 5, not a full review. The reviewer read files only and ran nothing.

Re-read 5's seven findings are resolved.

## Major

1. A command on a fresh database "proceeds without waiting" but has no confirmed identity, so its pool's connections are
   refused on the null token, and it cannot write; `seed-dev` and `migrate` likewise. Fix: admit its connections on a
   null token once the fresh check has passed; a test.

## Nit

1. The "any repetition" clause's 5 s tolerance is below a worker's backoff plus a request (8 s) and ignores control
   planes that confirm later.
2. The idle-drop test checks only the backend; also check the transaction and the row.
3. `kind` includes the passive observer, which reports nothing.

Counts: Blocker 0, Major 1, Nit 3

All four were fixed at the next commit.
