# Architecture confirmation check of design 041 (third re-read)

Doc: docs/designs/041-step-logs.md
Reviewed-commit: f9d4a65fa278e1c782a59e3d15adadd6e764371a
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: a fresh `architecture-reviewer`, asked to confirm the fix of the Major of `architecture-reread-2.md` (the script marker's lock timeout made a store parameter); no shell. Recorded condensed. With `architecture-reread.md` and `architecture-reread-2.md`, this closes the targeted re-read of the owner's approval path (OD-2026-11-16-7).

The previous Major is fixed: the timeout is a store parameter (200 ms by default; 60 s in the T041-13 tests that hold a lock the marker waits on), stated alike in section 7, T041-13 and data-model.md §9; the failure test does not depend on the clock; production behaviour is unchanged.

## Blocker

None.

## Major

None.

## Nit

1. "60 s, as the ingest's held transactions have" mixes two timeouts; the design calls the parameter a builder and data-model.md a constructor parameter; no test pins the 200 ms default.

Counts: Blocker 0, Major 0, Nit 1
