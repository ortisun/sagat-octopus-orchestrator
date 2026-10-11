//! The definition and run operations against the OpenAPI contract: every response the API really
//! produces must use a documented status, conform to the documented schema and carry the
//! documented headers (T002-032, FR-001-002).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod conformance;
