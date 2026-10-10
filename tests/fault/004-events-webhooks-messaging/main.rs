//! Failures around the events of feature 004: dispatchers that die at any point or race for the same
//! events, database connections killed under load, the race between an event, a timeout and a
//! second dispatcher, and a delivery whose answer was never recorded. Each test states an invariant
//! that must hold whatever was cut off (NFR-004-DUR, T004-012, T004-017, T004-022, T004-027,
//! T004-030).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod cancellation;
mod deliveries;
mod dispatch;
mod ingress;
