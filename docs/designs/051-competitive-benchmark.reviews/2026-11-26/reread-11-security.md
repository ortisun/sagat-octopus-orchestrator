# Targeted re-read 11 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 2e1be21457c5529f3bbccd922b6350817187981e
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the startup guard and the other changes made for `reread-3-correctness.md`, not a full review; the reviewer read the
design and the reports, searched for `set_config(` call sites, and ran nothing. No finding on the removal of the `sudo`
read of `/proc` (the host gets smaller), the channel's grammar, the two names of `proconfig`, the snapshots, or the
removal of the `PGOPTIONS` and `PGSERVICE` refusals (the guard covers them). Suspicions, not counted: feature 006's
planned statement raises `synchronous_commit` with a `CASE`, not a literal; from PostgreSQL 17 a `SET` value may be
stored as a placeholder. Every finding was fixed at the next commit; the `SECURITY DEFINER` part of Nit 2 by a check
of function sources, since this platform's migrations define such functions.

## Blocker

None.

## Major

1. Only the spelling `off` is refused, while PostgreSQL also accepts `false`, `no` and `0` in any case, names case
   insensitively, a formatted string passes as "a literal", and a `SET` value may be a placeholder. Fix: an allowlist of
   values, names compared without case, the same rule for `SET`, `SET LOCAL` and `ALTER ... SET`, with fixtures.
2. The guard's parsing is not tied to the server's: other whitespace, double backslashes, an empty name before the end
   and protocol 3.x minor versions pass a naive guard while the server applies the setting. Fix: accept only protocol
   3.0, refuse duplicates, early empty names, trailing bytes, backslashes and other whitespace, check each admitted
   value's pattern, with fixtures.
3. Nothing proves the server's only peer is the guard built at the commit (a forwarder could carry its name, its
   allowlists could be configured), and a shared socket directory bypasses it (with `peer` or `trust`, as superuser).
   Fix: `docker inspect` of the single peer and its image, compiled allowlists, no shared mount, the local line for the
   probe's role only, and the compose rule refusing `volumes_from`, shared volumes, `ports:` and YAML merges.

## Nit

1. A role with `ALTER SYSTEM` on `fsync` and `EXECUTE` on `pg_reload_conf` can turn it off and back between the two
   `pg_settings` checks. Fix: refuse any `pg_parameter_acl` row for the roles and `EXECUTE` on `pg_reload_conf`.
2. The extra reset of fault (c) stays admissible until the first scan, after products reconnect, and a
   `SECURITY DEFINER` wrapper defeats the `EXECUTE` check. Fix: admit a reset only up to the server's acceptance of
   connections, which the host measures.
3. This platform's `set_config(` search is literal and its runtime list not tied to the call sites found. Fix: match
   `(?i)\bset_config\s*\(` and build the list from the call sites that passed.
4. The upload sentences contradict each other (`run.sh` gathers, the `host` subcommand copies). Fix: the `host`
   subcommand gathers every upload and `run.sh` copies nothing from a directory a container can write.

Counts: Blocker 0, Major 3, Nit 4
