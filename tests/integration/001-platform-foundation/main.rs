//! Feature 001 integration suite against a real PostgreSQL (T001-006, T001-007, T001-028).
#![allow(clippy::expect_used)]

#[path = "../../support/mod.rs"]
mod support;

mod constraints;
mod flows;
mod migrations;
mod runtime_role;
mod store;
