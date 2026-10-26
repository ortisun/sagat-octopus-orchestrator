# crates/infra-postgres

The only crate that talks to PostgreSQL. The database is the authority (constitution I); the journal is append-only and projections must equal a replay (`verify_run`).

- Start transactions with `begin_safely()`, never `pool.begin()` (ADR-0014: a cancelled `begin` leaves the connection inside a transaction in sqlx 0.9).
- The runtime role `sagat_app` cannot take row locks (`FOR UPDATE`/`FOR SHARE` need the `UPDATE` privilege on the table). Use a plain read or an advisory lock. `tests/integration/platform-foundation/runtime_privileges` audits every statement against the role; it reads SQL literals, so build statements from literals or `macro_rules!` fragments, not `format!`.
- Migrations: `migrations/<YYYYMMDDHHMMSS>_<slug>.{up,down}.sql`, additive so an older reader keeps working, with grants to `sagat_app` in the same file and a `down` that refuses to destroy data. Every new variable-size column is classified in `storage-classification.json`. The checklist is `/migration`.
- Append-only tables are guarded by triggers (`ENABLE ALWAYS`); a test that needs to remove rows disables them explicitly and says why.
- Dynamic SQL in tests needs `AssertSqlSafe`; in production code it needs an audit comment and a fixed vocabulary.
- Test with real PostgreSQL, also as the runtime role (`SAGAT_TEST_AS_RUNTIME_ROLE=1`). Two replicas, a dead scheduler and a lapsed lease are modelled with `scheduler_shards` and `scheduler_members` rows.
