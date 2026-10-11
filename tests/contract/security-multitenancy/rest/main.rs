//! The governance operations against the OpenAPI contract: every response the API really produces
//! must use a documented status, conform to the documented schema and carry the documented headers
//! (T009-032, FR-009-006).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod battery;
mod conformance;
