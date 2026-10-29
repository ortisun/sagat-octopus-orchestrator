//! Schedules and bulk run operations (feature 018): real PostgreSQL, real services.
//!
//! The modules land with the tasks that need them (see `specs/018-schedules-bulk-operations/tasks.md`).
#![allow(clippy::expect_used)]

#[path = "../../support/mod.rs"]
mod support;

mod migration;
