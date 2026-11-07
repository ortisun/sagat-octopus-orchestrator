//! Schedules and bulk run operations (feature 018): real PostgreSQL, real services.
//!
//! The modules land with the tasks that need them (see `specs/018-schedules-bulk-operations/tasks.md`).
#![allow(clippy::expect_used)]

#[path = "../../support/mod.rs"]
mod support;

mod actions;
mod admission;
mod authority;
mod backfill;
mod batch;
mod batch_actions;
mod batch_executor;
mod batch_pass;
mod batch_security;
mod business_id_lookup;
mod business_id_rerun;
mod business_ids;
mod business_ids_purge;
mod business_ids_rest;
mod cancel_other;
mod cancel_source;
mod cancel_source_rest;
mod catchup;
mod changes;
mod counted;
mod delay;
mod disclosure;
mod disclosure_rest;
mod firing;
mod lag;
mod limits;
mod migration;
mod pass;
mod pending_resets_migration;
mod privacy;
mod purge;
mod reapply;
mod reset;
mod reset_rest;
mod reset_telemetry;
mod rest;
mod retained_arguments;
mod schedules;
mod service;
mod store;
mod store_support;
mod trigger;
mod update_with_start;
mod update_with_start_rest;
mod wait_via;
mod with_start;
mod with_start_rest;
