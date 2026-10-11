//! What feature 008 added to the REST API against the OpenAPI contract: every response it really
//! produces uses a documented status, conforms to the documented schema and carries the documented
//! headers, and every operation it added has a scenario that succeeds (T008-031, FR-001-002).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod conformance;
