//! The saga surface of the REST API against the OpenAPI contract: every response it really
//! produces uses a documented status, conforms to the documented schema and carries the documented
//! headers (T003-031, FR-001-002).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod conformance;
