//! What feature 007 added to the REST contract (`x-implemented: '007'`) against the live API
//! (T007-032): every response must use a documented status, conform to the documented schema and
//! carry the documented headers.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod conformance;
