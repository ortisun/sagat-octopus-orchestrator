# Targeted re-read 4 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: b7a3126d3c8efb666d0fd436513f7571f370ac2b
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A fourth targeted re-read
of the fixes of re-read 3 (changes since b2edd5bc), not a full review. The reviewer read files only and ran nothing.

Re-read 3: Major 1 resolved (every function serializes on the retirement's row; `ABANDONING` refused by the others; the
two interleaving tests); Nits 1 and 2 resolved; Nit 3 partly resolved (Nit 4 below).

No Blocker, no Major.

## Nit

1. The bound counts a 2 s wait on the partition that `NOWAIT` makes zero, and no test shows the partition lock failing
   at once. Fix: 2 s plus catalog time; a test with the partition held behind a barrier.
2. No test covers a reactivation after an abandon past step 3, where kept bytes would be released twice. Fix: reset the
   step and clear the bytes in the reactivating transaction; the test.
3. What a report says after an `ABANDONED` retirement for a tree with a run left unpurged is not stated.
4. The Verification Plan still names the per-second counter for Success criterion 049-002.

Counts: Blocker 0, Major 0, Nit 4

All four were fixed at `df04d3a3`.
