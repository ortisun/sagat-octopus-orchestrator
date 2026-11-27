# Targeted re-read 11 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 339ca37ee02357b1fff4857b7de2272e4f4f9941
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 10, not a full review. The reviewer read files only and ran nothing.

Re-read 10's three Nits are resolved (grants set by default privileges are written into `proacl` at creation, so the
`coalesce` misses nothing; the generator test and the test that the migration matches the generator cover the allowed
triggers).

No Blocker, no Major.

## Nit

1. `aclexplode` reports `PUBLIC` as grantee `0`; an inner join to the role catalog drops it and voids the fix; the
   privileges paragraph still says "compares each function's `proacl`". Fix: compare by identifier with `0` as `PUBLIC`;
   a negative fixture with a null `proacl`.
2. The generator test asserts the tenant and the bucket but not `root_run_id`, so a lookup could cross trees of one
   tenant. Fix: assert `root_run_id` too.

Counts: Blocker 0, Major 0, Nit 2

Both were fixed at `7a8dedb9`.
