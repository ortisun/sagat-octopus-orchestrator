//! PostgreSQL target execution for the isolated runner. Never link this adapter into a scheduler.
mod error;
mod executor;
mod transport;
pub use executor::{DatabaseResult, PostgresExecutor};
mod frame;
mod network;
mod statement;
mod values;
pub use error::ExecutionError;
pub use network::{DestinationScope, NetworkPolicy};
pub use statement::validate_statement;
