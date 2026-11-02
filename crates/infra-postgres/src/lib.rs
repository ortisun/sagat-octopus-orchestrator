//! PostgreSQL adapter of Sagat Octopus Orchestrator.
//!
//! Owns the schema migrations and implements the storage ports declared in `sagat-application`.
//! PostgreSQL is the source of truth (ADR-0002); every statement that touches tenant data carries a
//! `tenant_id` filter, and the schema additionally rejects cross-tenant references.

use sqlx::migrate::Migrator;

mod api_admission;
mod approval_progress;
mod approval_scheduler;
mod bindings;
mod breaker;
mod capacity_acquire;
mod capacity_discovery;
mod capacity_finish;
mod capacity_transfer;
mod capacity_vacate;
mod chain_progress;
mod collection_outputs;
mod connectors;
mod content_usage;
mod database;
mod database_results;
mod declared_operations;
mod definitions;
mod dispatch;
mod egress;
mod event_codec;
mod event_payloads;
mod events;
mod execution;
mod identity;
mod leaf_certification;
mod loop_progress;
mod maintenance;
mod managed_secrets;
mod operations;
mod outbound;
mod partitioning;
mod payload_disclosure;
mod payload_inspection;
mod pool;
mod primitives;
mod privilege;
mod quotas;
mod reveal_grants;
mod schedules;
mod scope_compensation;
mod scope_frontier;
mod scoped_leaf_finish;
mod scripts;
mod secrets;
mod signals;
mod telemetry;
mod versioning;
mod workers;

pub use approval_progress::PgApprovalProgressStore;
pub use capacity_acquire::PgCapacityAcquireStore;
pub use capacity_finish::PgCapacityFinishStore;
pub use capacity_vacate::PgCapacityVacateStore;
pub use chain_progress::{PgChainProgressStore, initialize_chain_in_transaction};
pub use declared_operations::PgDeclaredOperationStore;
pub use definitions::PgDefinitionStore;
pub use execution::{PgRunStore, PgRunTx};
pub use identity::PgIdentityStore;
pub use leaf_certification::PgLeafCollectionSuccessStore;
pub use loop_progress::PgLoopProgressStore;
pub use operations::{MONITOR_LEASE, READ_AUDIT_SQL};
pub use pool::{BeginSafely, PoolConfig, connect, spawn_pool_metrics};
pub use primitives::PgCollectionStore;
pub use privilege::{LeastPrivilegeError, PrivilegeFinding};
pub use reveal_grants::has_effective_payload_reveal_grant;
pub use schedules::PgScheduleStore;
pub use scoped_leaf_finish::PgScopedLeafFinishStore;
pub use sqlx::PgPool;
pub use telemetry::PROJECTOR_LEASE;
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

mod blobs;

pub use blobs::PgBlobCatalog;

mod secret_cleanup;
