# Targeted re-read 10 of design 049: correctness (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 39cbc2c38d90022e1d2f2024d782c414e59fd2ee
Reviewer: correctness
Agent-blob: d1de651dc4a3676084974c440b0684eed2cd4888
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 9, not a full review. The reviewer read files only and ran nothing.

Re-read 9: Major 1 resolved (the audit covers only this design's functions); Nits 2 and 3 resolved (a retry succeeds in
both cases, checked); Nit 1 partly (Nit 2 below).

No Blocker, no Major.

## Nit

1. It is not said whether the generated list holds only names or also the expected grants; if the grants come from the
   migrations, the audit checks them against themselves; feature 020's purge function is not created by this design's
   migrations. Fix: generate the names only, with a hand-written table of attributes.
2. A tag cannot select a libfaketime image (the repository is always `postgres`); `SAGAT_TEST_POSTGRES_TAG` already means
   the version (design 041); moving the clock needs a clock file and an isolated server. Fix: an image override for an
   image built here, the clock file and `isolated_server`.
3. The `RUN_PURGED` test cannot run in Task 049-8, before `runs.purged_at` exists.
4. A join does not guarantee that no partition is read; say that no partition's row is served.

Counts: Blocker 0, Major 0, Nit 4

All four were fixed at `d459361f`.
