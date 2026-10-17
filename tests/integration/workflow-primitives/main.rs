//! Workflow-primitive integration tests against real PostgreSQL. The shared support starts PostgreSQL or
//! fails when infrastructure is unavailable; scenario modules use deterministic barriers at commit
//! boundaries rather than timing sleeps.
#![allow(clippy::expect_used, clippy::panic)]

#[path = "../../support/mod.rs"]
mod support;

mod approval_combinations;
mod approval_progress;
mod approval_scheduler;
mod approvals_http;
mod byte_ceilings;
mod chain_handoff;
mod chain_progress;
mod chain_storage;
mod chains;
mod chains_http;
mod collection_restore;
mod collections_http;
mod hint_contract;
mod identity_storage;
mod journal_identity;
mod limits;
mod loop_progress;
mod loops_http;
mod observability;
mod operations;
mod operations_http;
mod primitive_transactions;
mod projection;
mod scope_compensation_http;
mod scoped_worker;
mod security;
mod switch_storage;
