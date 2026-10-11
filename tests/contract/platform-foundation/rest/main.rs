//! The REST API against its own OpenAPI contract: every response the API really produces must use a
//! documented status, conform to the documented schema and carry the documented headers.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod conformance;
