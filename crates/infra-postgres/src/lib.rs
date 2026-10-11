//! PostgreSQL adapter of Sagat Octopus Orchestrator.
//!
//! Owns the schema migrations and implements the storage ports declared in `sagat-application`.
//! PostgreSQL is the source of truth (ADR-0002); every statement that touches tenant data carries a
//! `tenant_id` filter, and the schema additionally rejects cross-tenant references.

use sqlx::migrate::Migrator;

mod bindings;
mod breaker;
mod connectors;
mod definitions;
mod dispatch;
mod egress;
mod event_codec;
mod events;
mod execution;
mod identity;
mod maintenance;
mod outbound;
mod pool;
mod privilege;
mod quotas;
mod scripts;
mod secrets;
mod signals;
mod workers;

pub use definitions::PgDefinitionStore;
pub use execution::{PgRunStore, PgRunTx};
pub use identity::PgIdentityStore;
pub use pool::{BeginSafely, PoolConfig, connect, spawn_pool_metrics};
pub use privilege::{LeastPrivilegeError, PrivilegeFinding};
pub use sqlx::PgPool;
/// The query that finds the runs a worker may claim, exposed so a test can check its plan.
#[doc(hidden)]
pub use workers::CLAIMABLE as CLAIMABLE_SQL;

/// Embedded, ordered schema migrations (`migrations/<version>_<name>.{up,down}.sql`).
pub static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

/// Applies all pending migrations. Idempotent: applying twice is a no-op, and concurrent runners
/// are serialised by an advisory lock taken by sqlx.
///
/// # Errors
/// Any migration or connection failure.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}
