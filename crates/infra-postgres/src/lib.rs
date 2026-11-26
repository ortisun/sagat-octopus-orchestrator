//! PostgreSQL adapter of Sagat Octopus Orchestrator.
//!
//! Owns the schema migrations and implements the storage ports declared in `sagat-application`.
//! PostgreSQL is the source of truth (ADR-0002); every statement that touches tenant data carries a
//! `tenant_id` filter, and the schema additionally rejects cross-tenant references.

use sqlx::migrate::Migrator;

mod api_admission;
mod approval_progress;
mod approval_scheduler;
mod batch;
mod bindings;
mod breaker;
mod business_ids;
mod capacity_acquire;
mod capacity_discovery;
mod capacity_finish;
mod capacity_transfer;
mod capacity_vacate;
mod chain_progress;
mod clock;
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
mod handover_auto;
mod handover_role;
mod identity;
mod instances;
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
/// Design 041's spike T041-35 only (`perf-spike`): runs the step-log carry inside a report transaction.
#[cfg(feature = "perf-spike")]
pub mod report_hook;
mod reveal_grants;
mod schedules;
mod scope_compensation;
mod scope_frontier;
mod scoped_leaf_finish;
mod scripts;
mod secrets;
mod signals;
mod step_logs;
mod telemetry;
mod trigger_schedules;
mod versioning;
mod workers;

pub use approval_progress::PgApprovalProgressStore;
pub use batch::PgBatchStore;
/// The statements of a batch selector by business-id prefix, exposed so a test can ask the planner about the very text they run.
#[doc(hidden)]
pub use batch::{
    PREVIEW_BY_IDS_SQL, PREVIEW_BY_PREFIX_SQL, SELECTION_BY_IDS_SQL, SELECTION_BY_PREFIX_SQL,
};
pub use capacity_acquire::PgCapacityAcquireStore;
pub use capacity_finish::PgCapacityFinishStore;
pub use capacity_vacate::PgCapacityVacateStore;
pub use chain_progress::{PgChainProgressStore, initialize_chain_in_transaction};
pub use clock::DatabaseClock;
pub use declared_operations::PgDeclaredOperationStore;
pub use definitions::{
    PgDefinitionStore, StoredVersion, schedules_exist, stored_versions_page, version_still_started,
};
/// The scan of the pending resets whose source did not end within the window, exposed so a test can check its plan.
#[doc(hidden)]
pub use execution::EXPIRED_RESETS_SQL;
pub use execution::{PgRunStore, PgRunTx};
pub use handover_auto::{
    AuditCompleted, AutoActivated, AutoAsk, HANDOVER_ROLE, HandoverDbError,
    ROUND_STATEMENT_TIMEOUT, Retirement, Round, RoundOutcome, complete_handover_audit,
    connect_handover, handover_activate, handover_note, handover_round, last_attempt,
    may_auto_hand_over, read_handover_mode, retire_handover_role,
};
pub use handover_role::{
    HANDOVER_ROLE_LOCK, LoginSet, RoleEnsured, RoleGrants, ensure_handover_role, is_scram_verifier,
    set_handover_floor, set_handover_login,
};
pub use identity::PgIdentityStore;
pub use instances::{
    CapabilityCensus, FOLLOWED_REQUIREMENT_MISSING, FeatureStep, OperatorBounds, PgInstanceStore,
    capability_census, census_step, may_activate, requirement_recorded,
};
pub use leaf_certification::PgLeafCollectionSuccessStore;
pub use loop_progress::PgLoopProgressStore;
pub use operations::{MONITOR_LEASE, READ_AUDIT_SQL};
pub use pool::{
    APPLICATION_NAME_PREFIX, BeginSafely, PoolConfig, PoolName, connect, spawn_pool_metrics,
    step_log_wait_slots, step_log_wait_slots_for,
};
pub use primitives::PgCollectionStore;
pub use privilege::{LeastPrivilegeError, PrivilegeFinding};
pub use reveal_grants::has_effective_payload_reveal_grant;
pub use schedules::PgScheduleStore;
pub use scoped_leaf_finish::PgScopedLeafFinishStore;
pub use sqlx::PgPool;
pub use telemetry::PROJECTOR_LEASE;
pub use trigger_schedules::{
    Census, HandoverWritten, complete_handover, engine_census, handover_step, may_hand_over,
    read_epoch, read_handover_row,
};
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
///
/// After the migrations it ensures the role of the automatic handover, `sagat_handover`, and grants it again (ADR-0091 decision 1,
/// D2264): created `NOLOGIN` when it is missing and the migrating role may create roles, refused its grants when it is wider than
/// the design. Neither outcome fails the migration; each is logged with what the operator must do.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await?;
    let (ensured, granted) = ensure_handover_role(pool).await?;
    match ensured {
        RoleEnsured::Created => tracing::info!(
            role = HANDOVER_ROLE,
            "the role of the automatic handover was created without a login; give migrate SAGAT_HANDOVER_PASSWORD to let it log in"
        ),
        RoleEnsured::NotPermitted => tracing::warn!(
            role = HANDOVER_ROLE,
            "the role of the automatic handover does not exist and the migrating role may not create roles: create it once \
             (CREATE ROLE sagat_handover NOLOGIN) and run migrate again, or keep the handover manual (docs/runbooks/schedules.md, 6.1)"
        ),
        RoleEnsured::Exists => {}
    }
    if granted == RoleGrants::Wider {
        tracing::error!(
            role = HANDOVER_ROLE,
            "the role sagat_handover is wider than the automatic handover allows (an attribute beyond LOGIN, a membership in a \
             role, or an object it owns): it was given none of its grants, and the job cannot hand over until it is made narrow \
             (docs/runbooks/schedules.md, 6.1)"
        );
    }
    Ok(())
}

mod blobs;

pub use blobs::PgBlobCatalog;

mod secret_cleanup;
