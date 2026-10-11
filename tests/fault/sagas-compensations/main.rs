//! Failures around the sagas of feature 003: a process that dies between the plan and the lease, a
//! provider that accepts before it times out, duplicates, races between a timeout and a signal,
//! a lost broker delivery, and database connections killed under load. Each test states an
//! invariant that must hold whatever was cut off (NFR-003-DUR, T003-012, T003-017, T003-022,
//! T003-027, T003-030).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod chaos;
mod common;
mod crashes;
mod provider;
mod races;
