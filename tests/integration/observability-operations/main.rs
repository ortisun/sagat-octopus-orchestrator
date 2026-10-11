//! Feature 008 integration suite against a real PostgreSQL: the migrations go back and forward with
//! data in the tables they touch (T008-006), the tenant boundary holds in every read the feature
//! added (T008-007), and a projection that disagrees with the journal settles instead of spinning
//! (T008-030).
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod isolation;
mod migration;
mod projection;
