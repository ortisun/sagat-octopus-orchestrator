# Targeted re-read 2 of design 048: architecture (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 7b5f574cc219e38da135109a08c35a2549ae6c07
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read
of the fixes of re-read 1, not a full review. The reviewer read files only and ran nothing.

Re-read 1: Majors 1, 3, 4, 5 and Nits 1 to 5 resolved; Blocker 1 partly (Major 1); Major 2 partly (Major 2).

## Major

1. The passive observer is still refused on a standby: the identity admission refuses a null token on any connection
   but two, and the standby-names check is on by default; ADR-0130 decision 3 contradicts decision 8; no test.
2. Write commands go around the guard that replaced ADR-0067 decision 4: they never call the confirmation. Fix: their
   wait through a definer function that refuses a binary without `region-v1` in an initialized cell.
3. The `REGION_*` exception never takes effect in a passive region if the gate is checked first. Fix: the region's role
   before the gate; a test.

## Nit

1. "Three" profiles and "a fourth"; list the four together.
2. The lint on protected names has no exception for `pool.rs`, which sends them.
3. The constant answer before authentication is a decision; make it ADR-0130's decision 9.
4. The dropped-`WriteTx` test passes vacuously unless the pool has one connection and the transaction is checked.

Counts: Blocker 0, Major 3, Nit 4

All seven were fixed at `e21f8403`.
