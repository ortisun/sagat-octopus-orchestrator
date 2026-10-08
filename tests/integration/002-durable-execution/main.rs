//! Feature 002 integration suite against a real PostgreSQL (T002-006, T002-007).
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod definitions;
