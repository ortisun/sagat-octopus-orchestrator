//! Feature 011 integration suite against a real PostgreSQL: approvals, the active pointer and its
//! history, promotion between environments, the migration with data, and the tenant boundary of
//! everything the feature added (T011-006, T011-007).
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod activation;
mod approvals;
mod common;
