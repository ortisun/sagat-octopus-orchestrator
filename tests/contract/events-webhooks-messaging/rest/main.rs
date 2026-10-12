//! The event operations against the OpenAPI contract: every response the API really produces must
//! use a documented status, conform to the documented schema and carry the documented headers
//! (T004-032, FR-004-001..004, FR-004-006).
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod conformance;

mod identity_profiles;
