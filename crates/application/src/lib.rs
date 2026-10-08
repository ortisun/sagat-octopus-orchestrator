//! Use cases and ports of Sagat Octopus Orchestrator.
//!
//! The crate depends on the domain only. Infrastructure (PostgreSQL, HTTP, ...) implements the
//! ports declared here, so the dependency always points inward (see `docs/architecture/overview.md`).

pub mod clock;
pub mod definitions;
pub mod entropy;
pub mod error;
pub mod execution;
pub mod identity;
pub mod maintenance;
pub mod runs;
pub mod worker;
