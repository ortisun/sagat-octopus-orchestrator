# Targeted re-read 3 of design 049: architecture (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 6f4999480340671a3e493d1d2032b6aae6a3573b
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 2, not a full review. The reviewer read files only and ran nothing.

Re-read 2: Major 2 and Nits 1, 2, 3, 6, 7 resolved; Major 1 partly (Major A); Nit 4 partly (Major B); Nit 5 partly (Nit c).

## Major

A. Nothing can set `permanent`: requirement rows are inserted only at activation, so a migration has no row to set and
   the default `false` never refuses; the older `platform_requirement_remove` still deletes any row; two sources of
   permanence with no test that they agree. Fix: a check `permanent = (capability IN (...))` rewritten by the migration
   that adds a permanent capability; the guard refuses deleting or draining a permanent row; the test also calls the
   older function; a test that `is_permanent()` matches the check.
B. `is_active` relies on requirements the heartbeat does not return (it carries the epoch only, and the requirements are
   read only when the epoch moves), so a process registered after the switch creates legacy roots for ever. Fix: the
   gate keeps the requirement set from every registration and epoch move; the test
   `a_process_registered_after_the_switch_creates_dated_roots`.

## Nit

a. Design 053's row V still says the handshake is executable by the runtime role only.
b. Row II omits Task 049-14.
c. The CI step that refuses `test-probes` is in no task.
d. The order does not say Task 049-13 needs Task 053-5 and Task 049-14 needs Task 053-3.

Counts: Blocker 0, Major 2, Nit 4

All were fixed at `bfa17146`.
