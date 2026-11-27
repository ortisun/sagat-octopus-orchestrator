# Targeted re-read 4 of design 048: egm-implementation-readiness (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 7a51e09a893032aad7f28ec9fb78e9fcea14da07
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A fourth targeted re-read
of the fixes of re-read 3, not a full review.

Re-read 3: Nits 1 to 4 resolved; Majors 1 and 2 partly (Major 1 and Nit 1 below).

## Major

1. Task 048-13 names no CloudNativePG metric or query, and the Patroni selector is hidden (every member reports 0 or 1,
   so a bare count never fires). Fix: both expressions in full.

## Nit

1. "The port" still names two things; `DatabaseHealth` and `InertHealth` placed in two files; `Admission` undefined.
2. A Rust `Drop` cannot send `ROLLBACK TO`; queue it before the next statement or commit.
3. The pool builder needs the health object before the `DbHandle`; `Arc::new(InertHealth)`.

Counts: Blocker 0, Major 1, Nit 3

All four were fixed at `e15a7ba1`.
