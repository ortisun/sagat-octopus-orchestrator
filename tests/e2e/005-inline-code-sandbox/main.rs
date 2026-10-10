//! The user stories of feature 005 through the public interfaces: an author writes and tries a
//! script, publishes an immutable signed artifact that workflows pin, and a hostile script is held
//! to its limits and audited. REST for the authors, the worker protocol for the runner, a real
//! PostgreSQL and real containers.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod us01;
mod us02;
mod us03;
mod us04;
