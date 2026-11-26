# Approval confirmation of design 049: architecture (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 44d6a0b3cd3afe687d5144167c8d18ef73870e19
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2 (one full round, `architecture-full.md` at 66634fb9; the
fixes re-read in `reread-1-architecture.md` to `reread-4-architecture.md`, each Major fixed before the next), not a full
review. Design 049's text is the same as at bfa17146; the fixes of re-read 4 went into design 053 and ADR-0132. The
reviewer read files only and ran nothing.

Re-read 4: Majors 1 and 2 resolved; Nits 1 to 3 resolved; Nit 4 partly (design 053's wrapping, Nit 4 below).

No Blocker, no Major.

## Nit

1. Task 049-13 names only the test that the two permanence lists agree, not
   `a_permanent_capability_refuses_both_deactivation_phases`, which Task 053-5 and ADR-0132 say it runs.
2. Design 049's merge order does not state that design 053's Task 053-3 comes before Task 049-5 (design 053 says so).
3. Task 049-5 says the heartbeat returns only the epoch; under design 053 it returns the set when the epoch moves.
4. Design 053's wrapping at three places.

Counts: Blocker 0, Major 0, Nit 4

Settled (see the egm-review summary of this approval): Nits 1 to 3 are recorded as design 049's follow-up Task 049-19,
added after the approval as the approval rule allows; Nit 4 is fixed in design 053.
