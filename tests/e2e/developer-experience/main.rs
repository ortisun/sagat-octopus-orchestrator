//! The four user stories of feature 012 through the public interfaces: `sagat` (the library the
//! executable wraps) writes a workflow from a template, publishes it, serves it with canned
//! answers and runs it against a real platform (PostgreSQL, the HTTP API, the engine as a process),
//! and REST shows what the platform made of it.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod dev_environment;
mod diagnostics;
mod doctor;
mod onboarding_browser;
mod order_processing;
mod saga;
mod scripts;
mod stories;
mod us03;

pub use crate::support::dx::{Dir, Outcome, cli, cli_as, engine, rest, run_with, within};
