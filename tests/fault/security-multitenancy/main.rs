//! Failures around the transactions of feature 009: database connections killed while quotas are
//! judged, secrets are stored and resolved, and bindings are revoked. Each test states an invariant
//! that must hold whatever was cut off (NFR-009-DUR, T009-012, T009-017, T009-022, T009-027).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod faults;
