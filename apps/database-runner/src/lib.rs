//! Managed SQL executes only in this process; it has no platform SQL store or scheduler.
mod attempt;
mod capability;
pub mod config;
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
