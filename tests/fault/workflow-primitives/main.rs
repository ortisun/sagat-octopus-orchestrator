//! Workflow-primitive crash, duplicate, race and recovery tests against real platform adapters. Fault cases
//! use deterministic barriers around transaction edges and independently inspect persisted state.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod switch;
