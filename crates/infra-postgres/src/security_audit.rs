//! The platform's security audit (`security_audit_records`; feature 009, decision D60; gap G041-028): the one writer of a
//! lookup that named another tenant's resource. No API reads the table; the operator does, with SQL.
//!
//! Bounded so that a flood cannot fill it: the same caller, kind and identifier once in a window (first in this process,
//! without a write, then in the table); in a window at most [`MAX_PROBES_PER_CALLER`] records per caller's tenant, actor
//! and owning tenant, and [`MAX_PROBES_PER_TENANT`] per caller's tenant and owning tenant, so that a caller who fills its
//! share with identifiers of a tenant of its own cannot hide a probe of another (the security review of 2026-11-27); a
//! gate of [`MAX_PENDING_PROBES`] writes for the whole process, separate from the gate of the tenant's records, so that
//! what the check found cannot change how likely the caller's tenant's record is to be written; and at most
//! [`PROBE_WRITERS`] of those writes holding a connection at once, so that probes never take the connections the answers
//! need (the correctness review of 2026-11-27). Every probe is counted before any of that (`cross_tenant_probes_total`,
//! exported at zero for every kind from the start, so that a first burst is an increase).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
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

/// Most probe writes pending at the same time in this process; beyond this they are counted and dropped.
pub(crate) const MAX_PENDING_PROBES: usize = 64;

/// Most probe writes holding a database connection at the same time in this process.
pub(crate) const PROBE_WRITERS: usize = 2;

/// Most probes this process remembers to drop a repeat without a write.
const RECENT_PROBES: usize = 4096;

/// The kinds a lookup across the boundary names: the label values of `cross_tenant_probes_total`.
const PROBED_KINDS: [ResourceType; 6] = [
    ResourceType::Run,
    ResourceType::Workflow,
    ResourceType::Namespace,
    ResourceType::Workspace,
    ResourceType::PublicationRequest,
    ResourceType::ApprovalRequest,
];

/// The state of the writer, one for the process whatever the number of stores built in it.
struct Probes {
    slots: Arc<Semaphore>,
    writers: Arc<Semaphore>,
    recent: Mutex<HashMap<ProbeKey, Instant>>,
}

static PROBES: LazyLock<Probes> = LazyLock::new(|| Probes {
    slots: Arc::new(Semaphore::new(MAX_PENDING_PROBES)),
    writers: Arc::new(Semaphore::new(PROBE_WRITERS)),
    recent: Mutex::new(HashMap::new()),
});

/// A probe as a repeat is recognised: the caller's tenant, the actor, the kind and the identifier.
type ProbeKey = (Uuid, String, &'static str, String);

fn key_of(probe: &CrossTenantProbe) -> ProbeKey {
    (
        probe.caller_tenant_id().as_uuid(),
        probe.actor().as_str().to_owned(),
        probe.resource_type().as_str(),
        probe.resource_id().to_owned(),
    )
}

/// Whether `probe` was accepted for a write in the last `window`; otherwise it is remembered now, when there is room.
fn repeated(probe: &CrossTenantProbe, window: Duration) -> bool {
    let key = key_of(probe);
    let now = Instant::now();
    let mut seen = PROBES.recent.lock().unwrap_or_else(PoisonError::into_inner);
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

/// Forgets `probe`, whose write did not happen, so that its next repeat is written.
fn forget(probe: &CrossTenantProbe) {
    PROBES
        .recent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&key_of(probe));
}

/// Exports `cross_tenant_probes_total` at zero for every kind, so that the first burst after a start is an increase.
pub(crate) fn register_probe_counters() {
    for kind in PROBED_KINDS {
        metrics::counter!("cross_tenant_probes_total", "resource_type" => kind.as_str())
            .absolute(0);
    }
}

/// Waits until every probe write already accepted in this process has finished.
pub(crate) async fn flush() {
    if let Ok(permits) = Arc::clone(&PROBES.slots)
        .acquire_many_owned(MAX_PENDING_PROBES as u32)
        .await
    {
        drop(permits);
    }
}

/// What a write of a probe came to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Written {
    Recorded,
    /// The same probe is in the window, or the caller's share for the owning tenant is full.
    Suppressed,
}

/// Counts `probe` and writes it off the answer path; never fails and never waits.
pub(crate) fn record_probe(pool: &PgPool, probe: &CrossTenantProbe, window: Duration) {
    metrics::counter!("cross_tenant_probes_total", "resource_type" => probe.resource_type().as_str())
        .increment(1);
    metrics::counter!("security_denials_total", "action" => probe.action().as_str()).increment(1);
    if repeated(probe, window) {
        // accepted for a write lately (that write may still be pending): this one adds nothing
        metrics::counter!("security_audit_writes_total", "outcome" => "repeated").increment(1);
        return;
    }
    let Ok(slot) = Arc::clone(&PROBES.slots).try_acquire_owned() else {
        forget(probe);
        metrics::counter!("security_audit_writes_total", "outcome" => "dropped").increment(1);
        tracing::warn!(
            resource_type = %probe.resource_type(),
            "too many security audit writes pending; dropping a probe across the tenant boundary"
        );
        return;
    };
    let pool = pool.clone();
    let probe = probe.clone();
    tokio::spawn(async move {
        let _slot = slot;
        let outcome = match Arc::clone(&PROBES.writers).acquire_owned().await {
            Ok(_writer) => match insert_probe(&pool, &probe, window).await {
                Ok(Written::Recorded) => "recorded",
                Ok(Written::Suppressed) => "suppressed",
                Err(error) => {
                    forget(&probe);
                    tracing::error!(%error, "failed to record a probe across the tenant boundary");
                    "failed"
                }
            },
            Err(_) => {
                forget(&probe);
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
