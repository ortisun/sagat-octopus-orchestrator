# Targeted re-read 12 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 535a68462d4ac3aefa11f645fbc1f9e149246420
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the fixes of `reread-11-security.md` (six resolved, Major 3 in part), not a full review; the reviewer read the design
and ran nothing. Suspicion, not counted: the sandbox-runner's engine socket could create a container sharing a
namespace in the middle of a turn. Fixed at the next commit by a different mechanism: the detection of each way a
session can change durability (startup guard, statement scan, call-site lists) is replaced by a check inside the
database server that refuses any commit whose effective `synchronous_commit` is below `local` or whose `commit_delay`
is not 0, which no spelling, startup path or namespace reaches around; Major B's namespace refusals are kept for
isolation.

## Blocker

None.

## Major

1. The setting name is matched as text, so `E'synchronous\x5fcommit'`, `U&'synchronous\005fcommit'` or a quoted
   `"set_config"(` skip the value allowlist, the scan and the call-site pattern, and the session commits
   asynchronously undetected.
2. A container sharing the guard's network namespace (`network_mode: "service:<guard>"`, `--network container:`)
   reaches the server from the guard's address, which `pg_hba.conf` admits, with any startup options. Fix: refuse every
   `network_mode` and `service:` or `container:` forms of `pid:` and `ipc:`, and check `HostConfig.NetworkMode`.

## Nit

1. A `SECURITY DEFINER` wrapper that builds the reset call at run time, with arguments, deletes one entry without moving
   `stats_reset`, so a `SET synchronous_commit = off` can be erased before the scan.

Counts: Blocker 0, Major 2, Nit 1
