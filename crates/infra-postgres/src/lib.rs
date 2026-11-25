//! PostgreSQL adapter of Sagat Octopus Orchestrator.
//!
//! Owns the schema migrations and implements the storage ports declared in `sagat-application`.
//! PostgreSQL is the source of truth (ADR-0002); every statement that touches tenant data carries a
//! `tenant_id` filter, and the schema additionally rejects cross-tenant references.

use sqlx::migrate::Migrator;

/// The predicate of a rate-limit grant (alias `$g`, its window row `$w`) that the purge may delete at the instant `$now`
/// (FR-022-056: once its window row's longest period and one minute more have passed since its lease; exactly at that
/// instant it is kept). One text for the purge (transaction T25) and for a decision's reclaim of its namespace's windows
/// (decision D022-066 item 1), so the purge's boundary test holds for both.
macro_rules! purgeable_grant {
    ($g:literal, $w:literal, $now:literal) => {
        concat!(
            $g,
            ".leased_at IS NOT NULL AND ",
            $g,
            ".leased_at + make_interval(secs => ",
            $w,
            ".max_period_ms / 1000.0 + 60) < ",
            $now
        )
    };
}

/// The run of a waiter (alias `$r`) that the grant pass serves: running or waiting, and not paused (ADR-0089 decision 4).
/// One text for the pass's eligibility and for the place estimate's "skipped" (task T022-112, decision D022-068 item 6).
macro_rules! live_gate_run {
    ($r:literal) => {
        concat!(
            $r,
            ".status IN ('RUNNING', 'WAITING') AND NOT ",
            $r,
            ".paused"
        )
    };
}

/// A waiter (alias `$w`, its run `$r`) the grant pass may grant at the instant `$now`: not past its queue deadline, of a
/// run the pass serves ([`live_gate_run!`]). Its own limit is checked beside it by each statement, over its own window.
/// One text for every use of the predicate (the grant routine's waiters ahead, the pass's listing of a key's waiters,
/// admission's eligible waiter, the place estimate; task T022-112, decision D022-068 item 6), as [`purgeable_grant!`]
/// is for the purge; `scripts/check-architecture.py` refuses a copy written out elsewhere.
macro_rules! eligible_gate_waiter {
    ($w:literal, $r:literal, $now:literal) => {
        concat!($w, ".deadline_at > ", $now, " AND ", live_gate_run!($r))
    };
}

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
mod flow_control;
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
mod reveal_grants;
mod schedules;
mod scope_compensation;
mod scope_frontier;
mod scoped_leaf_finish;
mod scripts;
mod secrets;
mod signals;
mod telemetry;
mod trigger_schedules;
mod versioning;
mod workers;

pub use approval_progress::PgApprovalProgressStore;
pub use batch::PgBatchStore;
/// The statements of a batch selector by business-id prefix, exposed so a test can ask the planner about the very text they run.
#[doc(hidden)]
pub use batch::{PREVIEW_BY_PREFIX_SQL, SELECTION_BY_PREFIX_SQL};
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
/// The deciding transactions of feature 022's gates file their keys through these, inside the store; exposed for the
/// integration suite that proves the rotation (T022-095).
#[doc(hidden)]
pub use flow_control::{
    FiledKey, KEY_ROW_VANISHED, KeyTable, KeyToFile, file_keys, key_versions, missing_key_versions,
};
/// The statements of the concurrency-key listing, exposed so a test can ask the planner about the very text they run
/// (D022-058).
#[doc(hidden)]
pub use flow_control::{
    KEY_LISTING_OF_GROUP_SQL, KEY_LISTING_SQL, RATE_KEY_LISTING_OF_GROUP_SQL, RATE_KEY_LISTING_SQL,
};
pub use identity::PgIdentityStore;
pub use instances::{
    FeatureStep, OperatorBounds, PgInstanceStore, live_capable_step, no_feature_step,
};
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
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    MIGRATOR.run(pool).await
}

mod blobs;

pub use blobs::PgBlobCatalog;

mod secret_cleanup;

#[cfg(test)]
mod gate_predicate_tests {
    /// Task T022-112: the one text of the grant pass's eligibility, as each of its uses expands it.
    #[test]
    fn the_eligibility_of_a_waiter_is_one_text() {
        assert_eq!(
            eligible_gate_waiter!("w", "r", "now()"),
            // split as the macro is, so that the architecture check does not read it as a copy
            concat!(
                "w.deadline_at > now() AND r.status IN ('RUNNING', 'WAITING') ",
                "AND NOT r.paused"
            )
        );
        assert_eq!(
            live_gate_run!("mr"),
            concat!("mr.status IN ('RUNNING', 'WAITING') ", "AND NOT mr.paused")
        );
    }
}
