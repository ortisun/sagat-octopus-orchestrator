# Targeted re-read 5 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 61c71c97fd4a43a78994f4d70a0f91cb4cf3be1b
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the fixes of `reread-4-security.md` (all three resolved), not a full review; the reviewer read files only and ran
nothing. Remark, not counted: the size bound on uploads was stated only in Task 051-7; the requirement now states it.
All four Nits were fixed at the next commit.

## Blocker

None.

## Major

None.

## Nit

1. The scan runs only after each rung, while statistics are reset after each restore and at each rung's start, so a
   `set_config` or `SET` at connection-pool start-up or between rungs is deleted unseen; an entry evicted by
   `pg_stat_statements.max` is lost the same way. Fix: scan immediately before every reset and fail the turn on a
   find or on a change of `pg_stat_statements_info.dealloc`.
2. The `set_config` list is checked per normalized statement text but trusted per cited source line, while one text
   can come from several call sites. Fix: admit an entry only when every call site producing that text, all listed,
   uses a constant setting name not about durability.
3. `Config.Env` misses a `PGOPTIONS` exported by an entrypoint and `PGSERVICE` or `PGSERVICEFILE` pointing at a
   `pg_service.conf` with `options`. Fix: read `/proc/<pid>/environ` of every process in each product container's
   control group, refuse the three variables, and refuse `pg_service.conf` files carrying `options`.
4. Introduced by the fix: the environment check reads values that hold the campaign's database credentials and the job
   log is public. Fix: print only the container's and the variable's names, run with tracing off, and test with a
   fixture that a refused value does not reach the output.

Counts: Blocker 0, Major 0, Nit 4
