# Targeted re-read 14 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: e52e9e73cef7a861fb33bb232b7cec407c311c35
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. Filed first as the
approval confirmation, then refiled as a re-read when the correctness confirmation at the same commit
(`reread-4-correctness.md`) led to text changes. A re-read in the approval run under Owner decision 2026-11-25-2: one full round (`security-full.md`), then the fixes
re-read in `reread-1-security.md` to `reread-13-security.md`, every finding of each fixed before the next. Not a full
review. The reviewer read the design and ran nothing.

All five findings of re-read 13 are resolved: the extension's utility hook refuses every unlogged form (inside
functions and `DO` blocks too) and no relation is unlogged at the turn's start, after every rung and at its end; both
libraries are preloaded and a refused canary proves the check at each turn's start and after fault (c)'s restart, its
read erroring when uninitialised; the roles hold no server-program or server-file membership, no `pg_reload_conf`, and
`pg_parameter_acl` is empty; a rung fails on a missing key or falling `calls`; the extension builds from a pinned
builder image without network. Suspicion, not counted: on PostgreSQL 15 or earlier a role with `CREATEROLE` could grant
itself `pg_execute_server_program` and revoke it between the reads; from 16 the grant needs `ADMIN OPTION`, and the
repository pins `postgres:16-alpine`.

Every finding of every security report of this approval run is fixed; none is accepted.

No Blocker, no Major, no Nit.

Counts: Blocker 0, Major 0, Nit 0
