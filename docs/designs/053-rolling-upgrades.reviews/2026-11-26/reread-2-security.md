# Targeted re-read 2 of design 053: security (2026-11-26)

Doc: docs/designs/053-rolling-upgrades.md
Reviewed-commit: dfdd55566b126f73e48c0a3b4be9e2811d89dadc
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A re-read of the fixes of
`reread-1-security.md` (all five resolved), not a full review. The reviewer read files only and ran nothing.

No Blocker, no Major.

## Nit

1. An eviction can interleave with a registration of the same instance (registration reads the evictions, then
   upserts; the deleted row no longer conflicts), registering the evicted instance again. Fix: `instance_evict` takes
   the epoch row `FOR UPDATE`; a barrier case in the test.
2. `CODEOWNERS` binds only with "Require review from Code Owners", unstated; the owned list omits `CODEOWNERS` and the
   catalog test's file; the closed-list templates may use `%s`. Fix: state the setting; own both files; refuse `%s`.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at the next commit.
