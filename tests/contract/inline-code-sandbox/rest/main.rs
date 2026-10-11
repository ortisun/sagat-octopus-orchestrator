//! The script operations against the OpenAPI contract: every response the API really produces must
//! use a documented status, conform to the documented schema and carry the documented headers
//! (T005-032, FR-005-001..008). The runner is played by the test, over the same HTTP interface.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod conformance;
