//! The worker protocol as feature 003 extends it (additively, protocol 1.2): a compensation and a
//! lookup are tasks like any other that say what they are about, a worker that knows nothing of
//! them keeps working, and nothing a compensation does can be mistaken for forward work
//! (T003-031, NFR-003-COMP).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod protocol;
