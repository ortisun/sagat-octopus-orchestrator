//! Feature 010 integration suite against a real PostgreSQL: shard leases and their fence, a replica
//! working on its own shards only, failover, and fairness between tenants (T010-006, T010-007;
//! FR-010-001, FR-010-002; US01..US04).
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod counts;
mod fairness;
mod lag;
mod outbox;
mod passes;
mod shards;
mod splits;
