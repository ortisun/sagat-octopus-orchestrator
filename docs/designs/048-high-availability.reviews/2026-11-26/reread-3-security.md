# Targeted re-read 3 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 5c2168b9360b0a10e93c4e4614f031ace47bc657
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A third targeted re-read
of the fixes of re-read 2, not a full review. The reviewer read files only and ran nothing.

Re-read 2: Major 1 and Nits 1, 2, 3, 4, 6, 7, 8 resolved; Nit 5 partly (Nits 2 and 3 below).

No Blocker, no Major.

## Nit

1. The database objects paragraph is stale (it says `sagat_server_identity()` writes the table), and the grants the
   fixes need are unstated; a table-level `GRANT SELECT` on `serving_instances` would expose `token_hash`. Fix: the
   function's attributes and column grants.
2. The 27 s bound holds per statement, not per request.
3. Compose's isolation fault cuts the primary from every process, so the release-gating drill never exercises the
   bound, and a request served by the new primary before the revocation's commit would fail the check.
4. The pre-authentication refusal still depends on the reason (`BINARY_CAPABILITY_MISSING` or not), and every
   closed-gate refusal runs before authentication.

Two suspicions for the correctness reviewer: a command not yet holding an identity cannot connect to wait; what the
confirmation compares after a crash is unclear.

Counts: Blocker 0, Major 0, Nit 4

All four and both suspicions were fixed at `6e923c17`.
