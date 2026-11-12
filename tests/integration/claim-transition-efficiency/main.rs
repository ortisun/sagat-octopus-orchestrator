//! Design 038 (claim and transition efficiency): what a lease and a completion cost the database,
//! and the claim behaviours that keep that cost down. Selected on a pull request that changes the
//! hot path, not only this directory (`scripts/select-test-targets.py`, OD-2026-11-12-4).
#![allow(clippy::expect_used, clippy::panic)] // helpers (not `#[test]` fns) fail the test by panicking

#[path = "../../support/mod.rs"]
mod support;

mod window_order;
mod work_budget;
