# Targeted re-read 10 of design 051: security (2026-11-26)

Doc: docs/designs/051-competitive-benchmark.md
Reviewed-commit: dfcc0c742a17fa12ea7e55ee019356ba80d105ab
Reviewer: security
Agent-blob: c02123dd239a7128f22e02c9dc39de8f110cc811
Note: condensed by the author from the reviewer's output; every finding and severity is kept. A confirmation read of
the fixes of `reread-9-security.md` (the Major and all three Nits resolved), not a full review; the reviewer read the
design and ran nothing. Not reopened: the residual between the last scan and the kill of fault (c). The Nit was fixed
at the next commit.

## Blocker

None.

## Major

None.

## Nit

1. Options also come from a connection URL's `options` query parameter (`pool.rs:122` starts from
   `PgConnectOptions::from_str(&config.url)`, and `.options(settings)` appends to it), and PostgreSQL also accepts
   `-cname=value` and `--name=value`; a URL literal carrying `options=-csynchronous_commit%3Doff` escapes the literal
   search, the generated options and the start probe. Fix: decode and parse the `options` parameter of every
   connection URL in the compose files, the configurations and the workspace's literals, parse all three forms, generate
   the published options from URLs and call sites, add a refused URL fixture, and name the searches and fixtures in
   Task 051-6.

Counts: Blocker 0, Major 0, Nit 1
