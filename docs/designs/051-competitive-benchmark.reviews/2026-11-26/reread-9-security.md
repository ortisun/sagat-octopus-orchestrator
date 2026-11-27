# Targeted re-read 9 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 6718a53c5f07a219f4e82a4d5d5fab2ab07439d0
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the changes made for `reread-2-correctness.md`, not a full review; the reviewer read the design,
`crates/infra-postgres/src/pool.rs` and `crates/infra-database/src/executor.rs`, and ran nothing. Judged and not
raised: the stated residual of fault (c) is acceptable (a session setting dies with the kill, persistent changes are
caught by the end-of-turn reads, `ALTER SYSTEM` needs a superuser); no bypass of the admitted `-c` settings; the
snapshots' keys, `cpu.stat`, `pg_stat_statements.max`, the acknowledgement file, the read from the last offset and the
host `crash` subcommand open nothing. Every finding was fixed at the next commit.

## Blocker

None.

## Major

1. This platform builds its startup options in code (`pool.rs:112-124`, `.options(settings)`), which no file check
   reads, and the published options are written by hand; a commit adding `synchronous_commit=off` there passes every
   check (no statement to scan, no role or function setting, no environment variable). Fix: the rule finds every
   `.options(` call site and `-c` literal in the workspace at the commit, refuses a name outside the admitted list or
   not constant, and generates the published options from those sites, with a refused `synchronous_commit` fixture.

## Nit

1. Neither per-product list (`proconfig`, `set_config`) refuses a durability setting mechanically. Fix: refuse
   `synchronous_commit` and `commit_delay` in both, with a fixture each.
2. After the kill, `stats_reset` is set when the module's shared memory starts, milliseconds after
   `pg_postmaster_start_time()`, so "equal to" fails every fault (c) turn; a tolerance window is safe only if the roles
   cannot reset the statistics. Fix: admit one reset between the postmaster's start and the first scan after the
   restart, and check that the roles have no `EXECUTE` on `pg_stat_statements_reset`.
3. The host reads the driver's directory "under the rule below", whose root is the record root, and `run.sh` cannot
   call `openat2`, so a plain read by path could follow a link planted by a compromised driver, or interpolate its
   content. Fix: name the host binary, give each directory its own root handle, and fix the posts' grammar.

Counts: Blocker 0, Major 1, Nit 3
