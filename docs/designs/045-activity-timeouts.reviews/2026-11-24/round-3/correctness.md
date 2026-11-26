# Correctness review of design 045, round 3 (2026-11-24)

Doc: docs/designs/045-activity-timeouts.md
Reviewed-commit: 7675358f
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author; every finding and its severity kept, with the author's answer (applied in the fourth revision).

Counts: Blocker 0, Major 5, Nit 10

Round 2: Majors 1, 2, 4, 7, 10 and Nits 1 to 11 closed; Major 5 closed with a gap (Major 4 below); Majors 3, 6, 8, 9 and Nit 12
partly closed.

## Major

1. **A paused run's step can still fail `SCHEDULE_TO_START_TIMEOUT`** (a withdrawal, an operator's retry or a grant during the pause
   writes a deadline). *Answer: accepted*: no claimable transition writes a start deadline while the run is paused (Technical Plan 15);
   tests for a withdrawal and an operator's retry during a pause.
2. **The withdrawal's 6 s window still fails healthy steps under a backlog** (the withdrawal resets `ready_at`). *Answer: accepted*: a
   full window from the withdrawal, and the step keeps its `ready_at`.
3. **A node with `heartbeatTimeoutMs` below 30,000 ms can wait without end for a 1.9 host.** *Answer: accepted*: every level-2 node
   requires `scheduleToStartMs`.
4. **Republishing identical bytes gives a level-0 version.** *Answer: accepted*: refused with `DOCUMENT_NEEDS_REPUBLISH`.
5. **The 1.9 first-beat rule clashes with Architecture decision 0099 for a flagged lease.** *Answer: accepted*: a flagged lease's first
   beat follows 0099 decision 6; a conformance case with the acknowledgement at 12 s.

## Nit (each accepted)

1. "At or after the cutoff" everywhere. 2. Architecture decision 0110 decision 2 aligned with decision 3. 3. The withdrawal writes no
field for a node without `scheduleToStartMs` or at level 0. 4. Requirement 045-020 restated through review: a hold clears the close
budget. 5. The claim barrier sets the column relative to `db_now`. 6. Stopping runs are excluded by a primary-key probe. 7. The hold's
lock timeout answers `UNAVAILABLE` with `REQUIREMENT_BUSY`. 8. The metric kinds and counted rules are stated. 9. The delay rule is one
rule. 10. A close cutoff during a pause runs its failure route at the resume.
