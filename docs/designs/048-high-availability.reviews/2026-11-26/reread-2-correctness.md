# Targeted re-read 2 of design 048: correctness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 70822f28866daaf0f6ba726c14a5ad8ae088f99f
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read
of the fixes of re-read 1, not a full review. The reviewer read files only and ran nothing.

Re-read 1: A, B, C, 1, 2, 3 and Nits 1, 3, 4, 5, 6 resolved; D partly (Major 1); Nit 2 partly (Nit 2).

## Major

1. A stale null read with a later ticket can move a fresh confirmation's identity into the superseded set, after which
   every connection is refused until the server restarts. Fix: an applied confirmation removes its identity from the
   superseded set; a test with a barrier.
2. Success criterion 048-003 gates on the first transition, while a lease survives by each worker's heartbeats; and it
   may test nothing if no repetition qualifies. Fix: each worker's heartbeat gap, and a qualifying repetition.

## Nit

1. Requirement 048-011 still says every command starts read-only; a missing `registry_first_confirmation` (`42P01`).
2. Task 048-12 still says 15 s; the derivation omits the 3 s connect (27 s).
3. The shard takeover is P + R + 4 s.
4. Whether a null token counts as different, and which reasons a confirmation clears.
5. A confirmation that commits but does not clear needs a wait.
6. For fault (3), the 310 s window counts from the fault and the gate from the standbys' return.
7. Reading ReadyForQuery's status needs `sqlx-postgres`; a successful ping is enough.

Counts: Blocker 0, Major 2, Nit 7

All nine were fixed at `bd3c8fe8`.
