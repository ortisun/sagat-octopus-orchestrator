//! Use cases and ports of Sagat Octopus Orchestrator.
//!
//! The crate depends on the domain only. Infrastructure (PostgreSQL, HTTP, ...) implements the
//! ports declared here, so the dependency always points inward (see `docs/architecture/overview.md`).

pub mod clock;
pub mod error;
pub mod identity;
