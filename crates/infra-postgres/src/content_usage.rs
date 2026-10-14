//! Shared PostgreSQL primitives for durable byte, rate, and retention policies (ADR-0027).
//!
//! These functions read policy only after taking the matching transaction-scoped fence. They are
//! deliberately crate-private: callers compose them into the transaction that owns the durable
//! mutation, and cannot supply a pre-read limit or an application clock.

// Consumer transactions call only the primitives relevant to their durable mutation. Keep unused
// foundation helpers crate-private while the remaining catalog integrations are still gated by
// ADR-0027; that gate must not turn adapter internals into a public API.
#![allow(dead_code)]

use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::quotas::{NamespaceApiAdmission, RateDecision, RetainedUsage};
use sagat_domain::connectors::ConnectorId;
use sagat_domain::events::EndpointId;
use sagat_domain::identity::{NamespaceId, TenantId};
use sagat_domain::quota::QuotaResource;
use sqlx::{PgConnection, Row};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::store_error;
use crate::pool::BeginSafely;

const MICROTOKENS_PER_REQUEST: i64 = 1_000_000;
const MAX_REFILL_MICROS: i64 = 2_000_000;

/// A stored endpoint or connector may add its own bucket to the namespace event bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EventRateSource {
    Endpoint(EndpointId),
    Connector(ConnectorId),
}

impl EventRateSource {
    fn parts(self) -> (&'static str, Uuid) {
        match self {
            Self::Endpoint(id) => ("ENDPOINT", id.as_uuid()),
            Self::Connector(id) => ("CONNECTOR", id.as_uuid()),
        }
    }
}

/// Text retained after applying the exact remaining tenant log allowance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LogAllocation {
    pub text: String,
    pub truncated: bool,
}

fn policy_key(tenant: TenantId, namespace: Option<NamespaceId>, resource: QuotaResource) -> String {
    namespace.map_or_else(
        || format!("quota-policy:{resource}:{tenant}"),
        |namespace| format!("quota-policy:{resource}:{tenant}:{namespace}"),
    )
}

async fn policy_lock(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    resource: QuotaResource,
    exclusive: bool,
) -> Result<(), StoreError> {
    let statement = if exclusive {
        "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))"
    } else {
        "SELECT pg_advisory_xact_lock_shared(hashtextextended($1, 0))"
    };
    sqlx::query(statement)
        .bind(policy_key(tenant, namespace, resource))
        .execute(conn)
        .await
        .map_err(store_error)?;
    Ok(())
}

/// Fences an administrative policy change. Namespace policy follows tenant policy in the lock
/// order, preserving the activity-quota hierarchy used elsewhere.
pub(crate) async fn policy_change(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    resource: QuotaResource,
) -> Result<(), StoreError> {
    if namespace.is_some() {
        policy_lock(conn, tenant, None, resource, false).await?;
    }
    policy_lock(conn, tenant, namespace, resource, true).await
}

async fn admission_policy(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    resource: QuotaResource,
) -> Result<u64, StoreError> {
    policy_lock(conn, tenant, None, resource, false).await?;
    if namespace.is_some() {
        policy_lock(conn, tenant, namespace, resource, false).await?;
    }
    effective_policy(conn, tenant, namespace, resource).await
}

async fn effective_policy(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: Option<NamespaceId>,
    resource: QuotaResource,
) -> Result<u64, StoreError> {
    let rows: Vec<(Option<Uuid>, i64, bool)> = sqlx::query_as(
        "SELECT namespace_id, max_value, false FROM quota_limits \
         WHERE tenant_id = $1 AND resource = $2 AND (namespace_id IS NULL OR namespace_id = $3) \
         UNION ALL \
         SELECT NULL, max_value, true FROM quota_ceilings \
         WHERE tenant_id = $1 AND resource = $2",
    )
    .bind(tenant.as_uuid())
    .bind(resource.as_str())
    .bind(namespace.map(NamespaceId::as_uuid))
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    let stored = |scoped: bool| {
        rows.iter()
            .find(|(namespace, _, ceiling)| !ceiling && namespace.is_some() == scoped)
            .and_then(|(_, value, _)| u64::try_from(*value).ok())
    };
    let granted = rows
        .iter()
        .find(|(_, _, ceiling)| *ceiling)
        .and_then(|(_, value, _)| u64::try_from(*value).ok());
    let ceiling = sagat_domain::quota::ceiling(resource, granted);
    Ok(sagat_domain::quota::resolve(
        resource,
        namespace.and_then(|_| stored(true)),
        stored(false),
        ceiling,
    )
    .0)
}

/// Exact usage used by quota views and reconciliation.
pub(crate) async fn retained_usage(
    conn: &mut PgConnection,
    tenant: TenantId,
) -> Result<RetainedUsage, StoreError> {
    let row = sqlx::query(
        "SELECT content_bytes, content_reserved_bytes, log_bytes \
         FROM tenant_retained_usage WHERE tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .fetch_optional(conn)
    .await
    .map_err(store_error)?;
    let Some(row) = row else {
        return Ok(RetainedUsage::default());
    };
    let value = |name| -> Result<u64, StoreError> {
        let value: i64 = row.try_get(name).map_err(store_error)?;
        u64::try_from(value).map_err(|_| StoreError::Internal {
            detail: "a retained-usage counter is negative".into(),
        })
    };
    Ok(RetainedUsage {
        content_bytes: value("content_bytes")?,
        content_reserved_bytes: value("content_reserved_bytes")?,
        log_bytes: value("log_bytes")?,
    })
}

/// Applies a reservation-only delta under the content policy and usage locks. Ordinary retained
/// rows use the generated accounting triggers instead. The migration intentionally grants the
/// runtime no direct counter-table DML; integration code must use a tracked catalog mutation.
#[allow(dead_code)]
pub(crate) async fn apply_content_delta(
    conn: &mut PgConnection,
    tenant: TenantId,
    content_delta: i64,
    reserved_delta: i64,
) -> Result<(), StoreError> {
    let limit = admission_policy(conn, tenant, None, QuotaResource::TenantContentBytes).await?;
    sqlx::query(
        "INSERT INTO tenant_retained_usage \
             (tenant_id, content_bytes, content_reserved_bytes, log_bytes) \
         VALUES ($1, 0, 0, 0) ON CONFLICT (tenant_id) DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    let row = sqlx::query(
        "SELECT content_bytes, content_reserved_bytes FROM tenant_retained_usage \
         WHERE tenant_id = $1 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    let content: i64 = row.try_get("content_bytes").map_err(store_error)?;
    let reserved: i64 = row.try_get("content_reserved_bytes").map_err(store_error)?;
    let next_content = content
        .checked_add(content_delta)
        .ok_or_else(corrupt_usage)?;
    let next_reserved = reserved
        .checked_add(reserved_delta)
        .ok_or_else(corrupt_usage)?;
    if next_content < 0 || next_reserved < 0 {
        return Err(corrupt_usage());
    }
    let current = content.checked_add(reserved).ok_or_else(corrupt_usage)?;
    let next = next_content
        .checked_add(next_reserved)
        .ok_or_else(corrupt_usage)?;
    if next > current && u64::try_from(next).unwrap_or(u64::MAX) > limit {
        return Err(StoreError::QuotaExceeded {
            what: QuotaResource::TenantContentBytes.limit_name(),
        });
    }
    sqlx::query(
        "UPDATE tenant_retained_usage SET content_bytes = $2, content_reserved_bytes = $3 \
         WHERE tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .bind(next_content)
    .bind(next_reserved)
    .execute(conn)
    .await
    .map_err(store_error)?;
    Ok(())
}

fn corrupt_usage() -> StoreError {
    StoreError::Internal {
        detail: "retained-usage arithmetic overflow or underflow".into(),
    }
}

/// Chooses a valid UTF-8 prefix under the exact remaining log allowance. The row write that stores
/// this result must follow immediately in the same transaction; its generated trigger applies the
/// measured delta at commit.
pub(crate) async fn take_log_prefix(
    conn: &mut PgConnection,
    tenant: TenantId,
    already_stored_bytes: u64,
    redacted: &str,
) -> Result<LogAllocation, StoreError> {
    // A log finalization may also persist content (for example a script result). Take the content
    // fence before the log fence and usage-row lock so deferred content admission never reverses
    // the global policy -> usage hierarchy.
    policy_lock(conn, tenant, None, QuotaResource::TenantContentBytes, false).await?;
    let limit = admission_policy(conn, tenant, None, QuotaResource::TenantLogBytes).await?;
    let projected: i64 = sqlx::query_scalar("SELECT sagat_quota_lock_log_usage($1)")
        .bind(tenant.as_uuid())
        .fetch_one(&mut *conn)
        .await
        .map_err(store_error)?;
    let without_current = projected
        .checked_sub(i64::try_from(already_stored_bytes).map_err(|_| corrupt_usage())?)
        .ok_or_else(corrupt_usage)?;
    if without_current < 0 {
        return Err(corrupt_usage());
    }
    let available = limit.saturating_sub(u64::try_from(without_current).unwrap_or(u64::MAX));
    let allocation = log_prefix(redacted, available);
    if allocation.truncated {
        // This is a decision counter: a later transaction rollback does not undo telemetry.
        metrics::counter!("quota_log_truncated_total").increment(1);
    }
    Ok(allocation)
}

fn log_prefix(redacted: &str, available: u64) -> LogAllocation {
    let take = usize::try_from(available)
        .unwrap_or(usize::MAX)
        .min(redacted.len());
    let mut end = take;
    while end > 0 && !redacted.is_char_boundary(end) {
        end -= 1;
    }
    LogAllocation {
        text: redacted[..end].to_owned(),
        truncated: end < redacted.len(),
    }
}

/// Takes the namespace event bucket and, when present, the stored source bucket in one transaction.
pub(crate) async fn take_event_rate(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    source: Option<EventRateSource>,
) -> Result<RateDecision, StoreError> {
    // Lock source configuration before policy/bucket locks. This makes a concurrent rate decrease
    // wait for the admission and preserves the global source -> policy -> bucket lock order.
    let source_rate = match source {
        None => None,
        Some(EventRateSource::Endpoint(id)) => Some(
            sqlx::query_scalar::<_, i32>(
                "SELECT rate_per_second FROM event_endpoints \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 FOR SHARE",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?
            .ok_or(StoreError::MissingReference)?,
        ),
        Some(EventRateSource::Connector(id)) => Some(
            sqlx::query_scalar::<_, i32>(
                "SELECT rate_per_second FROM event_connectors \
                 WHERE tenant_id = $1 AND namespace_id = $2 AND id = $3 FOR SHARE",
            )
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&mut *conn)
            .await
            .map_err(store_error)?
            .ok_or(StoreError::MissingReference)?,
        ),
    }
    .map(|rate| u64::try_from(rate).unwrap_or(0));
    let namespace_rate = admission_policy(
        conn,
        tenant,
        Some(namespace),
        QuotaResource::EventsPerSecond,
    )
    .await?;
    if namespace_rate == 0 {
        return Err(StoreError::QuotaExceeded {
            what: QuotaResource::EventsPerSecond.limit_name(),
        });
    }

    let mut buckets = vec![("NAMESPACE", Uuid::nil(), namespace_rate)];
    if let (Some(source), Some(rate)) = (source, source_rate) {
        let (kind, id) = source.parts();
        buckets.push((kind, id, rate));
    }
    // Stable order and a dry run under both row locks prevents one bucket being decremented when
    // the other refuses.
    buckets.sort_unstable_by_key(|(kind, id, _)| (*kind, *id));
    let mut states = Vec::with_capacity(buckets.len());
    for (kind, id, rate) in buckets {
        if rate == 0 {
            return Err(StoreError::QuotaExceeded {
                what: QuotaResource::EventsPerSecond.limit_name(),
            });
        }
        states
            .push(lock_bucket(conn, tenant, namespace, "events_per_second", kind, id, rate).await?);
    }
    if let Some(wait) = states.iter().filter_map(|state| state.retry_after()).max() {
        return Ok(RateDecision::Refused {
            retry_after_secs: wait,
        });
    }
    for state in states {
        state.consume(conn).await?;
    }
    Ok(RateDecision::Admitted)
}

struct BucketState {
    tenant: TenantId,
    namespace: NamespaceId,
    resource: &'static str,
    source_kind: &'static str,
    source_id: Uuid,
    available: i64,
    rate: u64,
    at: OffsetDateTime,
}

impl BucketState {
    fn retry_after(&self) -> Option<u32> {
        if self.available >= MICROTOKENS_PER_REQUEST {
            return None;
        }
        let deficit = u64::try_from(MICROTOKENS_PER_REQUEST - self.available).ok()?;
        let per_second = self.rate.checked_mul(MICROTOKENS_PER_REQUEST as u64)?;
        Some(
            u32::try_from(deficit.div_ceil(per_second))
                .unwrap_or(u32::MAX)
                .max(1),
        )
    }

    async fn consume(self, conn: &mut PgConnection) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE quota_rate_buckets SET microtokens = $6, updated_at = $7 \
             WHERE tenant_id = $1 AND namespace_id = $2 AND resource = $3 \
               AND source_kind = $4 AND source_id = $5",
        )
        .bind(self.tenant.as_uuid())
        .bind(self.namespace.as_uuid())
        .bind(self.resource)
        .bind(self.source_kind)
        .bind(self.source_id)
        .bind(self.available - MICROTOKENS_PER_REQUEST)
        .bind(self.at)
        .execute(conn)
        .await
        .map_err(store_error)?;
        Ok(())
    }
}

async fn lock_bucket(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    resource: &'static str,
    source_kind: &'static str,
    source_id: Uuid,
    rate: u64,
) -> Result<BucketState, StoreError> {
    let capacity = rate
        .checked_mul(2)
        .and_then(|value| value.checked_mul(MICROTOKENS_PER_REQUEST as u64))
        .and_then(|value| i64::try_from(value).ok())
        .ok_or_else(corrupt_usage)?;
    sqlx::query(
        "INSERT INTO quota_rate_buckets \
             (tenant_id, namespace_id, resource, source_kind, source_id, microtokens, updated_at) \
         VALUES ($1, $2, $3, $4, $5, $6, clock_timestamp()) \
         ON CONFLICT DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(resource)
    .bind(source_kind)
    .bind(source_id)
    .bind(capacity)
    .execute(&mut *conn)
    .await
    .map_err(store_error)?;
    let row = sqlx::query(
        "SELECT microtokens, updated_at FROM quota_rate_buckets \
         WHERE tenant_id = $1 AND namespace_id = $2 AND resource = $3 \
           AND source_kind = $4 AND source_id = $5 FOR UPDATE",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(resource)
    .bind(source_kind)
    .bind(source_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    let tokens: i64 = row.try_get("microtokens").map_err(store_error)?;
    let updated_at: OffsetDateTime = row.try_get("updated_at").map_err(store_error)?;
    // Read database time only after the bucket row lock. Reading it in the locking SELECT lets a
    // blocked waiter observe a time from before the prior owner advances `updated_at`.
    let now = sqlx::query(
        "WITH quota_now AS MATERIALIZED (SELECT clock_timestamp() AS observed_at) \
         SELECT greatest(observed_at, $2::timestamptz) AS at, \
             greatest(0::bigint, least($1::bigint, \
             floor(extract(epoch FROM (observed_at - $2::timestamptz)) * 1000000)::bigint)) AS elapsed_us \
         FROM quota_now",
    )
    .bind(MAX_REFILL_MICROS)
    .bind(updated_at)
    .fetch_one(&mut *conn)
    .await
    .map_err(store_error)?;
    let elapsed: i64 = now.try_get("elapsed_us").map_err(store_error)?;
    let at: OffsetDateTime = now.try_get("at").map_err(store_error)?;
    let refill = elapsed
        .checked_mul(i64::try_from(rate).map_err(|_| corrupt_usage())?)
        .ok_or_else(corrupt_usage)?;
    let available = tokens
        .checked_add(refill)
        .ok_or_else(corrupt_usage)?
        .min(capacity)
        .max(0);
    Ok(BucketState {
        tenant,
        namespace,
        resource,
        source_kind,
        source_id,
        available,
        rate,
        at,
    })
}

/// Refuses zero/over-ceiling event retention and preserves the replay window required by skew.
pub(crate) async fn check_event_retention(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    requested: Duration,
    skew: Option<Duration>,
) -> Result<(), StoreError> {
    let limit = admission_policy(
        conn,
        tenant,
        Some(namespace),
        QuotaResource::EventRetentionSeconds,
    )
    .await?;
    let minimum = skew.map_or(60, |skew| {
        skew.as_secs().saturating_mul(2).saturating_add(60)
    });
    if !event_retention_allowed(limit, requested.as_secs(), minimum) {
        return Err(StoreError::QuotaExceeded {
            what: QuotaResource::EventRetentionSeconds.limit_name(),
        });
    }
    Ok(())
}

pub(crate) async fn check_script_retention(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    requested_days: u16,
) -> Result<(), StoreError> {
    let limit = admission_policy(
        conn,
        tenant,
        Some(namespace),
        QuotaResource::ScriptRetentionDays,
    )
    .await?;
    if !script_retention_allowed(limit, requested_days) {
        return Err(StoreError::QuotaExceeded {
            what: QuotaResource::ScriptRetentionDays.limit_name(),
        });
    }
    Ok(())
}

/// Resolves the retention for a new execution record without exposing policy outside the adapter.
/// Existing script/version retention is an upper bound, while a later namespace policy decrease
/// applies to newly created execution records. A zero on either side disables new retention.
pub(crate) async fn script_execution_retention(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    requested_days: u16,
) -> Result<u16, StoreError> {
    let limit = admission_policy(
        conn,
        tenant,
        Some(namespace),
        QuotaResource::ScriptRetentionDays,
    )
    .await?;
    bounded_script_retention(limit, requested_days).ok_or(StoreError::QuotaExceeded {
        what: QuotaResource::ScriptRetentionDays.limit_name(),
    })
}

fn event_retention_allowed(limit: u64, requested: u64, minimum: u64) -> bool {
    limit != 0 && requested >= minimum && requested <= limit
}

fn script_retention_allowed(limit: u64, requested_days: u16) -> bool {
    limit != 0 && requested_days != 0 && u64::from(requested_days) <= limit
}

fn bounded_script_retention(limit: u64, requested_days: u16) -> Option<u16> {
    let limit = u16::try_from(limit).unwrap_or(u16::MAX);
    (limit != 0 && requested_days != 0).then(|| requested_days.min(limit))
}

impl NamespaceApiAdmission for PgRunStore {
    async fn admit_namespace_request(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<RateDecision, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let exists: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM namespaces WHERE tenant_id = $1 AND id = $2)",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if !exists {
            return Err(StoreError::MissingReference);
        }
        let rate = admission_policy(
            &mut tx,
            tenant,
            Some(namespace),
            QuotaResource::ApiRequestsPerSecond,
        )
        .await?;
        if rate == 0 {
            return Err(StoreError::QuotaExceeded {
                what: QuotaResource::ApiRequestsPerSecond.limit_name(),
            });
        }
        let state = lock_bucket(
            &mut tx,
            tenant,
            namespace,
            "api_requests_per_second",
            "NAMESPACE",
            Uuid::nil(),
            rate,
        )
        .await?;
        let decision = match state.retry_after() {
            Some(retry_after_secs) => RateDecision::Refused { retry_after_secs },
            None => {
                state.consume(&mut tx).await?;
                RateDecision::Admitted
            }
        };
        tx.commit().await.map_err(store_error)?;
        Ok(decision)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        bounded_script_retention, event_retention_allowed, log_prefix, script_retention_allowed,
    };

    #[test]
    fn log_prefix_never_splits_utf8_and_reports_truncation() {
        assert_eq!(log_prefix("aéz", 3).text, "aé");
        assert!(log_prefix("aéz", 2).text == "a" && log_prefix("aéz", 2).truncated);
        assert!(!log_prefix("aéz", 4).truncated);
    }

    #[test]
    fn retention_requires_a_nonzero_policy_and_request_and_honors_the_replay_minimum() {
        assert!(!event_retention_allowed(3_600, 119, 120));
        assert!(event_retention_allowed(3_600, 120, 120));
        assert!(!event_retention_allowed(0, 120, 120));
        assert!(!event_retention_allowed(3_600, 3_601, 120));
        assert!(!script_retention_allowed(365, 0));
        assert!(script_retention_allowed(365, 365));
        assert!(!script_retention_allowed(0, 1));
        assert!(!script_retention_allowed(365, 366));
        assert_eq!(bounded_script_retention(90, 365), Some(90));
        assert_eq!(bounded_script_retention(365, 90), Some(90));
        assert_eq!(bounded_script_retention(0, 90), None);
        assert_eq!(bounded_script_retention(365, 0), None);
    }
}
