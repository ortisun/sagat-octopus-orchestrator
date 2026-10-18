# Pinned protocol sources

PostgreSQL16 official references, checked2026-10-18:

- [Message formats](https://www.postgresql.org/docs/16/protocol-message-formats.html): Query Q,
  CommandComplete C, ErrorResponse E, ReadyForQuery Z; status I denotes no active transaction.
- [Message flow](https://www.postgresql.org/docs/16/protocol-flow.html): query completion and error
  responses are followed by ReadyForQuery. The test requires the actual result and idle status.
- [Error fields](https://www.postgresql.org/docs/16/protocol-error-fields.html): field C is SQLSTATE;
  field n is the constraint name. The injected abort has its own explicit test-only label.

Cargo.lock pins SQLx0.9.0. Verified local registry primary source:
`sqlx-core-0.9.0/src/transaction.rs:290` outer COMMIT literal;
`sqlx-postgres-0.9.0/src/transaction.rs:40` calls execute;
`sqlx-core-0.9.0/src/executor.rs:236` string execution has no arguments;
`sqlx-postgres-0.9.0/src/connection/executor.rs:282` selects simple query;
`sqlx-postgres-0.9.0/src/message/query.rs:9` Q/string encoding.
The harness intentionally requires the exact Q/COMMIT marker and must be rechecked on upgrade.
