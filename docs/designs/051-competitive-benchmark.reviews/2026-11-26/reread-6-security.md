# Targeted re-read 6 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: f16aa505937910d442a5c9b936532df84d2a11e4
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the fixes of `reread-5-security.md` (all four resolved), not a full review; the reviewer read files only and ran
nothing, and its claims about `pg_stat_statements_reset()` come from PostgreSQL's documentation. Remark, not counted:
masking each campaign credential with `::add-mask::` is an optional second layer. All four Nits were fixed at the next
commit, Nits 1 and 2 by the reviewer's cleaner option (one reset per turn before the product starts, per-rung latency
from snapshot differences).

## Blocker

None.

## Major

None.

## Nit

1. A reset without arguments also resets `pg_stat_statements_info`, so `dealloc` compared across a turn with resets in
   it shows no change and an evicted `SET synchronous_commit` goes unnoticed. Fix: require `dealloc` at 0 since the last
   reset, or stop resetting during the turn.
2. A statement between a scan and the reset that follows it, such as a pool reconnecting at a rung boundary, is
   deleted unseen. Fix: pause the load and wait for idle sessions, or (cleaner) stop resetting after the restore and
   take the per-rung latency from snapshot differences, which also removes Nit 1.
3. `/proc/<pid>/environ` of a container process is readable only with root, and a check that skips an unreadable file
   passes; the connection-string key `service=` is not refused, while a vendor image may carry its own
   `pg_service.conf`. Fix: read through `sudo` keeping only names, refuse on any error but a process that exited, test
   an unreadable file, and refuse `service`, `PGSERVICE` and `PGSERVICEFILE` in the configurations too.
4. This platform's call sites change between campaigns, so a later commit can add an unlisted `set_config` site that
   produces a listed text. Fix: `check_competitive_benchmark` finds every `set_config` call site of the workspace at
   the commit under test and requires each listed with a constant setting name.

Counts: Blocker 0, Major 0, Nit 4
