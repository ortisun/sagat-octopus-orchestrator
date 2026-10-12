//! The four stories of feature 010 against the real `execution-engine` (T010-008, T010-013,
//! T010-018, T010-023; US01..US04; SC-010-002): the engine is started the way the binary starts it,
//! from environment variables, and what the stories promise is checked from outside it.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod isolated;
mod us01;
mod us02;
mod us03;
mod us04;
