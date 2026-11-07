//! Managed SQL executes only in this process; it has no platform SQL store or scheduler.
mod attempt;
mod capability;
pub mod config;
/// The lake profile of the runner (feature 035). Public so that what is built here, and not yet called by the driver, is not
/// a dead-code allowance that would outlive the driver: an item of a public module is part of the crate's surface.
pub mod lake;
mod rpc;
mod runner;
mod slots;
pub use runner::Runner;

/// Closed errors: never format transport, provider, SQL, parameters or credential diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RunnerError {
    #[error("invalid database runner configuration")]
    Configuration,
    #[error("database runner credentials unavailable")]
    Credentials,
    #[error("database worker transport unavailable")]
    Transport,
    #[error("invalid managed database capability")]
    Capability,
}
