//! Workflow-primitive public-interface scenarios against the real platform. A missing PostgreSQL, broker or
//! language toolchain is a test failure when a scenario requires it, never a successful substitute.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;
