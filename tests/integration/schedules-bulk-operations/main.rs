//! Schedules and bulk run operations (feature 018): real PostgreSQL, real services.
//!
//! The modules land with the tasks that need them (see `specs/018-schedules-bulk-operations/tasks.md`).
#![allow(clippy::expect_used)]

#[path = "../../support/mod.rs"]
mod support;

mod admission;
mod authority;
mod backfill;
mod batch;
mod batch_executor;
mod batch_pass;
mod cancel_other;
mod catchup;
mod changes;
mod firing;
mod limits;
mod migration;
mod pass;
mod purge;
mod rest;
mod schedules;
mod service;
mod store;
mod store_support;
mod trigger;
