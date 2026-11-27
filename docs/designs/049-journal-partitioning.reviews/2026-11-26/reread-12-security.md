# Targeted re-read 12 of design 049: security (2026-11-26)

Doc: docs/designs/049-journal-partitioning.md
Reviewed-commit: 7a8dedb91e3818d96ff31f31ee5a722a81d167d4
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation re-read of
the fixes of re-read 11, not a full review. The reviewer read files only and ran nothing.

Re-read 11's two Nits are resolved. None of the three edits adds a tenant-boundary, privilege or injection problem.

No Blocker, no Major.

## Nit

1. The audit compares grantee and privilege but not the grant option, so a grant `WITH GRANT OPTION` to `sagat_app`
   would pass and let it grant `EXECUTE` onward. Fix: compare `is_grantable`, false for every grantee but the owner; a
   second negative fixture.

Counts: Blocker 0, Major 0, Nit 1

Fixed at `c621f25b`.
