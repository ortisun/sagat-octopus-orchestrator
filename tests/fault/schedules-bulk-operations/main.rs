//! Schedules and bulk run operations (feature 018): crashes at each statement, stale fences,
//! duplicates, lost hints, restarts and clocks that disagree with the database's (`T018-020`,
//! `SC-018-002`, `NFR-018-001`).
//!
//! The modules land with the tasks that need them (see `specs/018-schedules-bulk-operations/tasks.md`).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod clocks;
mod firing;
mod tripwire;
