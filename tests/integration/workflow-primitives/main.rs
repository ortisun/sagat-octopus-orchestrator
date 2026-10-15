//! Workflow-primitive integration tests against real PostgreSQL. The shared support starts PostgreSQL or
//! fails when infrastructure is unavailable; scenario modules use deterministic barriers at commit
//! boundaries rather than timing sleeps.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod approval_progress;
mod approvals_http;
mod chain_progress;
mod chain_storage;
mod chains_http;
mod collection_restore;
mod identity_storage;
mod journal_identity;
mod loop_progress;
mod operations;
mod operations_http;
mod primitive_transactions;
mod scoped_worker;
mod security;
mod switch_storage;
