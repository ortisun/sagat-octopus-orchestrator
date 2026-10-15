//! Use cases and ports of Sagat Octopus Orchestrator.
//!
//! The crate depends on the domain only. Infrastructure (PostgreSQL, HTTP, ...) implements the
//! ports declared here, so the dependency always points inward (see `docs/architecture/overview.md`).

pub mod api_admission;
pub mod approval_progress;
pub mod authz;
pub mod bindings;
pub mod clock;
pub mod connectors;
pub mod declared_operations;
pub mod definitions;
pub mod diff;
pub mod dispatch;
pub mod egress;
pub mod entropy;
pub mod error;
pub mod events;
pub mod execution;
pub mod identity;
pub mod loop_progress;
pub mod maintenance;
pub mod operations;
pub mod outbound;
pub mod payloads;
pub mod primitives;
pub mod publishing;
pub mod quotas;
pub mod runs;
pub mod sandbox;
pub mod script_signing;
pub mod scripts;
pub mod secrets;
pub mod shards;
pub mod signing;
pub mod telemetry;
pub mod worker;

pub mod blobs;
