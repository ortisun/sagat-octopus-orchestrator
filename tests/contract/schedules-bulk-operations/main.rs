//! Schedules and bulk run operations (feature 018): the REST and AsyncAPI contracts stay additive and
//! the reasons are documented.
//!
//! The modules land with the tasks that need them (see `specs/018-schedules-bulk-operations/tasks.md`).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod rest;
