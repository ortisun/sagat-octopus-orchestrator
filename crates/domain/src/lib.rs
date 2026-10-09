//! Pure domain model of Sagat Octopus Orchestrator.
//!
//! This crate must stay free of web, database, broker and serialization-format
//! dependencies (see `AGENTS.md` rule 5). The rule is enforced in CI by
//! `scripts/check-architecture.py`.

#[macro_use]
mod macros;

pub mod audit;
pub mod binding;
pub mod connectors;
pub mod definition;
pub mod egress;
pub mod events;
pub mod execution;
pub mod identity;
pub mod outbound;
pub mod quota;
pub mod saga;
pub mod secrets;
pub mod triggers;
pub mod workflow;
