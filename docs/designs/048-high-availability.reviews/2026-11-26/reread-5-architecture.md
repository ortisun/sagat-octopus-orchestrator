# Targeted re-read 5 of design 048: architecture (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: 8464d0553c1751e6830e785c9504eaa8b3bf8202
Reviewer: architecture
Agent-blob: c6fbfd6fa75c2429ef857c4daa78fc1a3f5ea3d9
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 4, not a full review. The reviewer read files only and ran nothing.

Re-read 4's Major and four Nits are resolved.

No Blocker, no Major.

## Nit

1. One sentence still says stores are built by `DbHandle::new`, and the `test-support` gate does not cover it.
2. Task 048-7 moves every caller to `connect_lazy`, so a process would start without reaching the database before the
   gate exists; say the pools stay eager until Task 048-10.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
