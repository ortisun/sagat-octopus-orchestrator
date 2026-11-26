# Approval confirmation of design 049: architecture (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 3c82957ecde6e5dd24a4138fca874b3cf9e29df8
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. The last of the targeted
re-reads of the approval run under Owner decision 2026-11-25-2: one full round (`architecture-full.md`, at 66634fb9, 2
Blockers and 9 Majors), then the fixes re-read in `reread-1-architecture.md` to `reread-5-architecture.md`, each Major
fixed before the next. Not a full review. The reviewer read files only and ran nothing.

Re-read 5's four Nits are resolved (Nit 3 with Nit 1 below). Checked as agreeing with each other: Requirement 049-009 step
(1), the Retirement section's step 1, the lock key, Requirement 049-016, design 053's Requirement 053-005 and ADR-0132's
"Enforced by"; the six retirement functions are listed the same way everywhere.

No Blocker, no Major.

## Nit

1. The Constitution Check's row II still says the heartbeat returns only the epoch.
2. The `TimeZone`, `DateStyle` and `search_path` pins have no check; a later migration could recreate a function
   without them. Fix: Requirement 049-013 and Task 049-7 compare `proconfig`, named in ADR-0132's "Enforced by".
3. No task owns the grant of design 053's handshake to `sagat_retirer`: the role does not exist when Task 053-3 runs, and
   Task 049-14 does not list the grant. Fix: Task 049-14 writes the grant with the role and tests the command's handshake
   as `sagat_retirer`.
4. Changed and older paragraphs of design 049 run past the wrap width.

Counts: Blocker 0, Major 0, Nit 4

Settled: Nits 1 to 3 are recorded as design 049's follow-up Task 049-19, added after the approval as the approval rule
allows, so that they are fixed in the build, test first; Nit 4 (layout only) is left to the next edit of the design.
