//! Workflow-primitive crash, duplicate, race and recovery tests against real platform adapters. Fault cases
//! use deterministic barriers around transaction edges and independently inspect persisted state.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod approvals;
mod chains;
mod collection_sources;
mod collections;
mod loops;
mod operations;
mod scope_compensation;
mod switch;
