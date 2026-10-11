//! The part of the worker protocol that lets SDKs and servers of different ages meet: version
//! negotiation (US04, FR-006-001). The SDKs' own behaviour is checked by the conformance suite in
//! `tests/contract/sdk-conformance/`.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../../support/mod.rs"]
mod support;

mod negotiation;
