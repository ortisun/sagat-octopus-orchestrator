//! Fault injection for feature 001 (NFR-001-DUR, plan risk "JWT expired/revoked or JWKS
//! temporarily unavailable"): provider outages, database outages, duplicate deliveries and
//! crashes inside handlers. The invariant under test is always the same: the system fails
//! closed, answers with a retryable error where recovery is possible, and recovers by itself.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod database;
mod provider;
mod resilience;
