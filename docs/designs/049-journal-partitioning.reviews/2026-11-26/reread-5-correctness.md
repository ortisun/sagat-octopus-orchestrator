# Targeted re-read 5 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 7da51b794cf58d05b69a4c01ed8e2ab7635ff28f
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the changes since b7a3126d (the answers to the security, architecture, comprehension and readiness re-reads), not a full
review. The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. A dated root accepted at its month's end plus 24 h can still be open when a retirement starts and commit after step 3.
   Fix: the `runs` trigger takes the bucket's lock in shared mode for a dated root of an ended month and checks
   `journal_bucket_retiring`.
2. The process and the trigger use the same 24 h on different clocks, so skew alone can refuse a correct choice. Fix: the
   process switches to legacy earlier.
3. Erasure locks the root then each run by key; writers lock a parent before its child. Fix: lock by depth, then
   identifier.
4. The skew justification of `journal_retire_start`'s 24 h is wrong for a command's time; cite the trigger.
5. Design 053's drain check says "is live" where design 049 says "exists".
6. With a permanent capability active, the mixed-writer test never exercises the rollback of the others. Fix: deactivate
   and roll back before activating the permanent ones.
7. Task 049-12 activates the capability before Task 049-13 creates it. Fix: set the store flag directly.

Counts: Blocker 0, Major 0, Nit 7

All seven were fixed at `8a18ee1b`.
