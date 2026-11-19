# Architecture confirmation check of design 041 (second re-read)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: fac00787bcaea68b47252139aadbe20bdfc660a5
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer`, asked to confirm the fixes of its re-read's two Majors (resolution.md, "The re-read of 2026-11-16 and its Majors"); no shell. Recorded condensed.

Re-read Majors: 1 (the marker's transaction without lock timeout or failure path) fixed; 2 (ADR-0084 decision 9) fixed, with the old Nit 2.

## Blocker

None.

## Major

1. The marker now waits under a 200 ms lock timeout, so T041-13's terminal-transition test, which holds the namespace's step-log policy fence while a cancel ends the run, passes only if all of it happens within 200 ms, a test against the clock; make the timeout a store parameter that the test sets long.

## Nit

None new.

Counts: Blocker 0, Major 1, Nit 0
