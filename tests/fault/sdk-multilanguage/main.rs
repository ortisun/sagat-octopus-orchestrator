//! Fault injection for the SDKs of feature 006: answers that are lost, calls that never leave, a
//! network that refuses, workers that go silent, identities that are not allowed. The conformance
//! scenarios inject the faults at the transport hooks of each SDK (an HTTP handler, a gRPC interceptor)
//! and the SDKs, of every language, must come to the same state of the platform.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

#[path = "../../contract/sdk-conformance/harness.rs"]
mod harness;

mod network;
mod tenant;
mod workers;
