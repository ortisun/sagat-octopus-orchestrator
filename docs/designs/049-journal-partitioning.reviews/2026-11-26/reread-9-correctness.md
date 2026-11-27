# Targeted re-read 9 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: b4c70e382374232b5644cb22a11972feb0d2f083
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 8, not a full review. The reviewer read files only and ran nothing.

Re-read 8: Major 1 resolved; Nits 2 to 5 resolved (Nit 4 with the Major below); Nit 1 partly (Nit 1 below).

## Major

1. The new audit clause fails on any owner-owned definer function that no design's list names; about 30 exist from
   earlier features, and the startup check refuses to start on a finding, so Task 049-7 would stop every process. Fix: a
   list of the existing ones, or a clause limited to this design's functions.

## Nit

1. The tests "drive the database clock", but nothing in the test support shifts `clock_timestamp()`; name the method
   (for example a libfaketime image through `SAGAT_TEST_POSTGRES_TAG`).
2. The `purged_at` check must be a join in the same statement (no new statement on the hot path); per-run reads exist in
   many more files with different shapes, so one function per read shape; `mark_projected` carries a bucket read
   earlier.
3. "A retry takes the legacy bucket" holds only for a root that takes its command's time; say "a retry succeeds".

Counts: Blocker 0, Major 1, Nit 3

All four were fixed at `39cbc2c3`: the audit compares this design's functions against a list generated from its own
migrations (Major 1); the libfaketime image in Task 049-3 (Nit 1); one function per read shape with the join, the files
named, the carried bucket (Nit 2); "a retry succeeds" (Nit 3).
