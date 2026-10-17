//! End-to-end target reserved by T016-003. Real API journeys are introduced by the owning feature
//! tasks; this registration intentionally contains no synthetic substitute.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

#[path = "../../support/canary.rs"]
mod canary_support;

mod canary;
mod cli;
mod quickstart;
