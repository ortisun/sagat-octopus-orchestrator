//! Feature 009 integration suite against a real PostgreSQL: quotas, worker bindings, secrets and
//! egress policy, with the tenant boundary asserted in every module.
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod quotas;
