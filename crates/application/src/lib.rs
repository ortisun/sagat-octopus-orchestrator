//! Use cases and ports of Sagat Octopus Orchestrator.
//!
//! The crate depends on the domain only. Infrastructure (PostgreSQL, HTTP, ...) implements the
//! ports declared here, so the dependency always points inward (see `docs/architecture/overview.md`).

pub mod api_admission;
pub mod approval_progress;
pub mod approval_scheduler;
pub mod authz;
pub mod bindings;
pub mod capacity_discovery;
pub mod capacity_finish;
pub mod capacity_vacate;
pub mod chain_progress;
pub mod classification;
pub mod classification_index;
pub mod clock;
pub mod collection_source;
pub mod connectors;
pub mod database;
pub mod database_results;
pub mod declared_operations;
pub mod definitions;
pub mod diff;
pub mod dispatch;
pub mod egress;
pub mod entropy;
pub mod error;
pub mod event_payloads;
pub mod events;
pub mod execution;
pub mod identity;
pub mod leaf_certification;
pub mod loop_progress;
pub mod maintenance;
pub mod managed_secrets;
pub mod operation_evaluation;
pub mod operations;
pub mod outbound;
pub mod payload_disclosure;
pub mod payloads;
pub mod primitives;
pub mod publishing;
pub mod quotas;
pub mod reveal_grants;
pub mod runs;
pub mod sandbox;
pub mod schedule_codec;
pub mod schedules;
pub mod scope_frontier;
pub mod scoped_leaf_finish;
pub mod script_signing;
pub mod scripts;
pub mod secret_provider;
pub mod secrets;
pub mod shards;
pub mod signing;
pub mod telemetry;
pub mod worker;

pub mod blobs;

pub mod secret_cleanup;
