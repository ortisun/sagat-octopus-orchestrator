//! Workflow-primitive integration tests against real PostgreSQL. The shared support starts PostgreSQL or
//! fails when infrastructure is unavailable; scenario modules use deterministic barriers at commit
//! boundaries rather than timing sleeps.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod identity_storage;
mod journal_identity;
mod security;
mod switch_storage;
