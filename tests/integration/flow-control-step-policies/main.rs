//! Flow control and step policies (feature 022) against a real PostgreSQL: the migrations of the 0.3.0 slice, the
//! fleet activation of its two capabilities, the one admission point and the hot-path budget it must not move.
//!
//! The modules land with the tasks that need them (see `specs/022-flow-control-step-policies/tasks.md`).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod admission;
mod ceilings;
mod child_starts;
mod concurrency_locks;
mod consistency;
mod lifecycle;
mod migrations;
mod publication;
mod queue;
mod rate_creators;
mod rate_gate;
mod rate_lifecycle;
mod rate_limit_locks;
mod rate_rest;
mod readiness;
mod readiness_sources;
mod rest;
mod returns;
mod rotation;
mod run_admission;
mod run_creators;
