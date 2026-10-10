//! The user stories of feature 006 through the public interfaces: an application starts a run from an
//! SDK, a worker of any language registers and serves it, a saga is compensated and reconciled with the
//! same semantics in every language, and a worker of an older or newer minor version of the protocol is
//! served. Each story plays its scenarios of the conformance suite (`tests/contract/sdk-conformance`) in
//! the SDK of every language against a real platform (PostgreSQL, execution engine, REST API) and holds
//! the result to the story's acceptance scenario. `SAGAT_SDK_LANGS` narrows the languages.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

#[path = "../../contract/sdk-conformance/harness.rs"]
mod harness;

mod us01;
mod us02;
mod us03;
mod us04;
