# Targeted re-read 13 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: 229b5a4ebff1ad10c27769c78d85b2bf2ca0049b
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the commit check that replaced the detection of each way to change durability, not a full review; the reviewer read
the design and ran nothing, its PostgreSQL behaviour from source and documentation. All three findings of
`reread-12-security.md` are resolved or moot (the value is read where it takes effect; deferred triggers fire before
the pre-commit event; a procedure with a `SET` clause cannot commit). No new way to reach the repository, a release, a
secret or the host. Every finding was fixed at the next commit.

## Blocker

None.

## Major

1. Unlogged tables skip the write-ahead log while `synchronous_commit` stays `on`, and a table's owner may run
   `ALTER TABLE ... SET UNLOGGED`; nothing looks at a relation's persistence. Fix: the extension's utility hook
   refuses every unlogged form, and `pg_class` holds no `relpersistence = 'u'` at the turn's start, after every rung
   and at its end, with tests.
2. Nothing confirms the extension is loaded, and the fixed constraints still set `shared_preload_libraries =
   'pg_stat_statements'`, so a server without the check would pass with a count of 0. Fix: name both libraries, run a
   refused canary at each turn's start whose count rises by exactly 1, and make the read error when uninitialised.

## Nit

1. Membership in `pg_execute_server_program` gives a shell as `postgres`, which could rewrite `postgresql.auto.conf`,
   reload and revert between the two reads. Fix: refuse direct or inherited membership in the three server-file and
   program roles, require `pg_parameter_acl` empty, check `pg_reload_conf` with `has_function_privilege`.
2. A role with `EXECUTE` on `pg_stat_statements_reset` can drop single entries without moving `stats_reset` or
   `dealloc`, shrinking a latency difference. Fix: fail the rung when a key of the start snapshot is missing at the end
   or its `calls` fell.
3. Compiling the extension needs a compiler and headers, normally fetched in a networked build no rule covers. Fix: a
   builder image pinned in `images.lock`, pinned packages, and the compile stage without network.

Counts: Blocker 0, Major 2, Nit 3
