# Targeted re-read 11 of design 048: security (2026-11-26)

Doc: docs/designs/048-high-availability.md
Reviewed-commit: e102812a1a4f25387e4fbb14333a5e23e448ef27
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fix of re-read 10, not a full review. The reviewer read files only and ran nothing.

Re-read 10's Nit is resolved.

No Blocker, no Major.

## Nit

1. Cargo merges features across the build, so a crate under `crates/` enabling `test-support` in its normal dependencies
   would turn it on for every binary. Fix: refuse it in every workspace manifest's normal dependencies, or check each
   application's `cargo tree`.
2. Task 048-7 says `new(PgPool)` has "its architecture rule", which arrives with Task 048-10.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
