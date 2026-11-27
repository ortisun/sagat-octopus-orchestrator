//! The platform's security audit (`security_audit_records`; feature 009, decision D60; gap G041-028): the one writer of a
//! lookup that named another tenant's resource. No API reads the table; the operator does, with SQL.
//!
//! Bounded so that a flood cannot fill it: the same caller, kind and identifier once in a window (first in this process,
//! without a write, then in the table); in a window at most [`MAX_PROBES_PER_CALLER`] records per caller's tenant, actor
//! and owning tenant, and [`MAX_PROBES_PER_TENANT`] per caller's tenant and owning tenant, so that a caller who fills its
//! share with identifiers of a tenant of its own cannot hide a probe of another (the security review of 2026-11-27); and a
//! gate of [`MAX_PENDING_PROBES`] writes of its own, separate from the gate of the tenant's records, so that what the check
//! found cannot change how likely the caller's tenant's record is to be written. Every probe is counted before any of
//! that (`cross_tenant_probes_total`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use sagat_application::error::StoreError;
use sagat_domain::audit::{CrossTenantProbe, ResourceType};
use sqlx::PgPool;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::identity::store_error;
use crate::pool::BeginSafely;

/// Most records one caller (its tenant and actor) leaves in a window about the resources of one tenant; the rest are
/// counted and not written.
pub const MAX_PROBES_PER_CALLER: i64 = 100;

/// Most records the callers of one tenant leave in a window about the resources of one tenant.
pub const MAX_PROBES_PER_TENANT: i64 = 1000;

/// Most probes this process remembers to drop a repeat without a write.
const RECENT_PROBES: usize = 4096;

/// The probes this process wrote in the last window, by caller's tenant, actor, kind and identifier: a repeat is
/// dropped without taking a place of the gate or a connection, so that repeating one identifier costs nothing.
#[derive(Debug, Default)]
pub(crate) struct RecentProbes(Mutex<HashMap<ProbeKey, Instant>>);

/// A probe as a repeat is recognised: the caller's tenant, the actor, the kind and the identifier.
type ProbeKey = (Uuid, String, &'static str, String);

impl RecentProbes {
    /// Whether `probe` was written in the last `window`; otherwise it is remembered now, when there is room.
    fn repeated(&self, probe: &CrossTenantProbe, window: Duration) -> bool {
        let key = (
            probe.caller_tenant_id().as_uuid(),
            probe.actor().as_str().to_owned(),
            probe.resource_type().as_str(),
            probe.resource_id().to_owned(),
        );
        let now = Instant::now();
        let mut seen = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if seen
            .get(&key)
            .is_some_and(|at| now.duration_since(*at) < window)
        {
            return true;
        }
        if seen.len() >= RECENT_PROBES {
            seen.retain(|_, at| now.duration_since(*at) < window);
        }
        if seen.len() < RECENT_PROBES {
            seen.insert(key, now);
        }
        false
    }

    /// Forgets `probe`, whose write did not happen.
    fn forget(&self, probe: &CrossTenantProbe) {
        let key = (
            probe.caller_tenant_id().as_uuid(),
            probe.actor().as_str().to_owned(),
            probe.resource_type().as_str(),
            probe.resource_id().to_owned(),
        );
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&key);
    }
}

/// Most probe writes pending at the same time; beyond this they are counted and dropped.
pub(crate) const MAX_PENDING_PROBES: usize = 64;

/// What a write of a probe came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Written {
    Recorded,
    /// The same probe is in the window, or the caller has its most records there.
    Suppressed,
}

/// Counts `probe` and writes it off the answer path, under `slots`; never fails and never waits.
pub(crate) fn record_probe(
    pool: &PgPool,
    slots: &Arc<Semaphore>,
    recent: &Arc<RecentProbes>,
    probe: &CrossTenantProbe,
    window: Duration,
) {
    metrics::counter!("cross_tenant_probes_total", "resource_type" => probe.resource_type().as_str())
        .increment(1);
    metrics::counter!("security_denials_total", "action" => probe.action().as_str()).increment(1);
    if recent.repeated(probe, window) {
        metrics::counter!("security_audit_writes_total", "outcome" => "suppressed").increment(1);
        return;
    }
    let Ok(slot) = Arc::clone(slots).try_acquire_owned() else {
        recent.forget(probe);
        metrics::counter!("security_audit_writes_total", "outcome" => "dropped").increment(1);
        tracing::warn!(
            resource_type = %probe.resource_type(),
            "too many security audit writes pending; dropping a probe across the tenant boundary"
        );
        return;
    };
    let pool = pool.clone();
    let probe = probe.clone();
    let recent = Arc::clone(recent);
    tokio::spawn(async move {
        let _slot = slot;
        let outcome = match insert_probe(&pool, &probe, window).await {
            Ok(Written::Recorded) => "recorded",
            Ok(Written::Suppressed) => "suppressed",
            Err(error) => {
                recent.forget(&probe);
                tracing::error!(%error, "failed to record a probe across the tenant boundary");
                "failed"
            }
        };
        metrics::counter!("security_audit_writes_total", "outcome" => outcome).increment(1);
    });
}

/// The statement that reads which tenant holds a resource of `kind`, for the kinds a lookup across the boundary names.
const fn owner_statement(kind: ResourceType) -> Option<&'static str> {
    match kind {
        // a chain is named by the identifier of its first run, and outlives it
        ResourceType::Run => Some(
            "SELECT tenant_id FROM runs WHERE id = $1 \
             UNION ALL SELECT tenant_id FROM execution_chains WHERE chain_id = $1 LIMIT 1",
        ),
        ResourceType::Workflow => Some("SELECT tenant_id FROM workflows WHERE id = $1"),
        ResourceType::Namespace => Some("SELECT tenant_id FROM namespaces WHERE id = $1"),
        ResourceType::Workspace => Some("SELECT tenant_id FROM workspaces WHERE id = $1"),
        ResourceType::PublicationRequest => {
            Some("SELECT tenant_id FROM publication_requests WHERE id = $1")
        }
        ResourceType::ApprovalRequest => {
            Some("SELECT tenant_id FROM approval_requests WHERE request_id = $1")
        }
        _ => None,
    }
}

async fn insert_probe(
    pool: &PgPool,
    probe: &CrossTenantProbe,
    window: Duration,
) -> Result<Written, StoreError> {
    let mut tx = pool.begin_safely().await.map_err(store_error)?;
    // Two probes of one caller's tenant racing would both pass the bounds before either commits: a lock on the caller's
    // tenant, held to the end of the transaction, lets the second see the first.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1::text, 0))")
        .bind(format!("security-audit|{}", probe.caller_tenant_id()))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
    let owner = match (
        owner_statement(probe.resource_type()),
        Uuid::parse_str(probe.resource_id()),
    ) {
        (Some(statement), Ok(id)) => sqlx::query_scalar::<_, Uuid>(statement)
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(store_error)?,
        _ => None,
    };
    let inserted = sqlx::query(
        "INSERT INTO security_audit_records \
         (id, caller_tenant_id, actor, action, resource_type, resource_id, owner_tenant_id, trace_id, created_at) \
         SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9 \
         WHERE NOT EXISTS ( \
             SELECT 1 FROM security_audit_records \
             WHERE caller_tenant_id = $2 AND coalesce(owner_tenant_id, '00000000-0000-0000-0000-000000000000'::uuid) = coalesce($7::uuid, '00000000-0000-0000-0000-000000000000'::uuid) AND actor = $3 \
               AND resource_type = $5 AND resource_id = $6 AND created_at > $9 - make_interval(secs => $10)) \
           AND (SELECT count(*) FROM security_audit_records \
                WHERE caller_tenant_id = $2 AND coalesce(owner_tenant_id, '00000000-0000-0000-0000-000000000000'::uuid) = coalesce($7::uuid, '00000000-0000-0000-0000-000000000000'::uuid) AND actor = $3 \
                  AND created_at > $9 - make_interval(secs => $10)) < $11 \
           AND (SELECT count(*) FROM security_audit_records \
                WHERE caller_tenant_id = $2 AND coalesce(owner_tenant_id, '00000000-0000-0000-0000-000000000000'::uuid) = coalesce($7::uuid, '00000000-0000-0000-0000-000000000000'::uuid) \
                  AND created_at > $9 - make_interval(secs => $10)) < $12",
    )
    .bind(probe.id().as_uuid())
    .bind(probe.caller_tenant_id().as_uuid())
    .bind(probe.actor().as_str())
    .bind(probe.action().as_str())
    .bind(probe.resource_type().as_str())
    .bind(probe.resource_id())
    .bind(owner)
    .bind(probe.trace_id())
    .bind(probe.created_at())
    .bind(window.as_secs_f64())
    .bind(MAX_PROBES_PER_CALLER)
    .bind(MAX_PROBES_PER_TENANT)
    .execute(&mut *tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    tx.commit().await.map_err(store_error)?;
    Ok(if inserted == 1 {
        Written::Recorded
    } else {
        Written::Suppressed
    })
}
