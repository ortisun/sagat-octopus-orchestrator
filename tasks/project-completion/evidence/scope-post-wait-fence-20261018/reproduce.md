# Collection/loop scheduler authority expiry during lock wait

Base6211938c, disposable repository PostgreSQL. Unset SAGAT_TEST_DATABASE_URL; set
SAGAT_TEST_AS_RUNTIME_ROLE=1, CARGO_INCREMENTAL=0, CARGO_BUILD_JOBS=3,
RUST_MIN_STACK=16777216. Commands, exits and wall times are in commands.json and
regression-commands.json. New stores explicitly use the runtime pool; the shared barrier
requires the actual waiter to be sagat_app when this option is requested.

- compile-static-sql-error.txt/test-prototype.diff: SQLx0.9 rejects dynamically assembled SQL.
  No tests executed; use one literal atomic snapshot query, no AssertSqlSafe bypass.
- audit-masked-red.txt/test-audit-mask.diff: loop incorrectly commits after expiry; collection
  fails on duplicate audit primary key, not an expired-authority commit. Two overflow cases pass.
- audit-only.diff: remove only enter_collection's duplicate audit INSERT; finish_version
  remains the atomic audit writer. This production correction precedes final clean RED.
- red.txt/test-red.diff/red-source.json: two overflow cases pass; both collection and loop
  incorrectly commit after authority expiry (6.54s). The source is base plus audit-only fix.
- production.diff: transaction-typed shared post-lock clock checks retaining error precedence,
  plus audit deduplication. green.txt passes4/4 (6.16s).

The unchanged-tuple barrier observes a real live-lease wait before DB-clock expiry, then
rolls back its unchanged guard. No sleep-only ordering oracle. Seventeen tenant tables are
compared atomically. Loop carry1 is absent beforehand; collection supplies a traced audit
and requires exactly one after fresh-authority success. Exact replay changes no state.
Scope is expiry while waiting for authority, not clock validity continuously until commit.
Historical native fixtures may use owner pools; filtered counts alone do not prove app roles.
No full-workspace execution, SDK58 matrix, Compose/advisory/resource or nesting gate claimed.
