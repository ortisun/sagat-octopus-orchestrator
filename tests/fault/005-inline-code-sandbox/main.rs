//! Feature 005 under failure: a runner that dies mid-build and mid-step, a runner that restarts, an
//! engine that cannot start containers, a control plane that cannot be reached, a run cancelled
//! while its script is running, and a runner asked for more than it has room for. Real PostgreSQL,
//! the real stack and real containers, with a stand-in sandbox for the failures the engine cannot be
//! made to have on demand.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod engine;
mod loss;
mod races;
