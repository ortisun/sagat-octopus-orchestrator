# Targeted re-read 7 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 657c41aa56ad19c63d30d5f47b048c18d6645955
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the fixes of `reread-6-security.md` (all four resolved), not a full review; the reviewer read files only and ran
nothing. Remark, not counted, adopted at the next commit: each scan also confirms that `stats_reset` is the time of
the turn's single reset. The Nit was fixed at the next commit.

## Blocker

None.

## Major

None.

## Nit

1. Suspicion: the per-rung `pg_stat_statements` snapshots carry the `query` column, which may keep literals of utility
   statements (a start-up `ALTER ROLE ... PASSWORD`), and the upload allowlist does not say whether they are on it.
   Fix: keep the snapshots in the host-only directory and upload only the computed differences per table (`queryid`,
   table, `calls`, `total_exec_time`), never `query`.

Counts: Blocker 0, Major 0, Nit 1
