# Targeted re-read 2 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 781553630e322baacdeae0e257975f0e23f8321a
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A second targeted re-read
of the fixes of re-read 1 (changes since df04d3a3), not a full review. The reviewer read files only and ran nothing.

Re-read 1: Majors 1 and 2 resolved; Nits 1, 2, 4, 5, 6 (per-run reads) and 7 resolved; Nit 3 partly (below).

No Blocker, no Major.

## Nit

A. (Suspicion.) A root chain's next segment takes its `created_at` from its command's time; if that lags a day, the root
   lands in a month that may be retiring. Fix: the `runs` trigger refuses a dated root whose month ended more than 24 h
   before `clock_timestamp()`.
B. Erasure checks the tree and then purges without saying both hold one lock; a run that turns ineligible between them
   leaves the tree partly erased. Fix: hold the root's row and each run's row in key order through the check.
C. If the mark-only form and the abandon's deletes are modes of the purge function `sagat_app` executes, the runtime role
   could mark runs purged with no retirement active. Fix: refuse those modes unless the bucket's retirement is active, or
   keep them out of that function.
D. Statements that span runs still read rows of purged runs awaiting the drop. Fix: each allow-list entry joins `runs`
   with `purged_at IS NULL`, enforced by the check.
E. (Re-read 1 Nit 3, partly.) A definer function returns null `xact_start` for other roles' sessions unless its owner
   holds `pg_read_all_stats`.

Counts: Blocker 0, Major 0, Nit 5

All five were fixed at `acd8c828`: a late root takes the legacy bucket and the trigger refuses a dated one (A); erasure
marks the whole tree `PURGING` under the root's row and each run's row in key order before any chunk (B); the two forms
run only in the retirement role's functions, which check the active row (C); the allow-list entries skip purged runs,
checked (D); the owner role is granted `pg_read_all_stats` (E).
