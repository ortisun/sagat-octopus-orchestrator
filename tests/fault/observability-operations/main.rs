//! What goes wrong around the telemetry of a run, and what the platform does about it: a collector
//! that is down, a projector that dies half way, two projectors, a run with no trace, a hostile
//! event. The invariant: no span is lost to an outage, none is made twice with a different id, and
//! nothing the projector does makes a transition wait (NFR-008-DUR, NFR-008-PERF).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod monitor;
mod projection;
