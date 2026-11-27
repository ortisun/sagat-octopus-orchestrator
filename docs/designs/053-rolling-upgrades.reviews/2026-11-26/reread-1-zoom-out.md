# Targeted re-read 1 of design 053: egm-zoom-out (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: 8e330550192e7505989251ecbe735fb9e7175f6a
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`zoom_out_report.md` (at 212e4262, the first draft), not a full review.

Full round: Majors 1 to 8 resolved; Nits 1, 3, 4 and 5 resolved, Nit 2 partly (below).

## Major

1. The partitioned-index procedure is defined now and built later, yet the 0.3.0 lint already accepts its header, so a
   loop without the procedure would run it as a plain `CREATE INDEX` on the parent, blocking inserts for the build.
   Fix: the lint refuses `-- partitioned-index` until the design that builds the procedure lifts it.
2. Form (b) protects a removal 0.3.0 never needs: `journal-buckets-v1` ships in 0.3.0, so every release a rollback can
   reach has it, yet it buys a trigger on every `runs` insert, a serialization path, a retry, a partial index and a
   share lock in phase 2. Fix: no fence for it; defer form (b) to the first removable capability written at the hot
   path's rate.
3. The eviction table adds no safety beyond the runbook's stop-first step (a re-registered row holds phase 2 again with
   a null applied epoch; a compromised process can register under another identifier). Fix: a plain delete.

## Nit

1. Requirement 053-004 still carries the retry arithmetic, the SQLSTATE list and the language reasoning, which belong
   in Task 053-4.

Counts: Blocker 0, Major 3, Nit 1

Majors 1 and 2 were fixed at the next commit (form (b) stays defined for the capability that first needs it, built by
none in 0.3.0). Major 3 and the Nit are settled by keeping the design's choice: the eviction record answers security
re-read 1's Major (a process whose stop step failed or was skipped keeps re-registering under its identifier), at the
cost of one table and one read at registration; the bounds stay in the requirement, where the correctness and
comprehension reviews asked for them.
