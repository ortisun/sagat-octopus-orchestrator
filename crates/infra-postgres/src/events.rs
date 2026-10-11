//! The endpoints and the inbox in PostgreSQL (`FR-004-002`, `FR-004-003`, `FR-004-008`).
//!
//! The primary key of `inbox_events` is the deduplication: admitting an event is one insert, and a
//! second insert of the same key does nothing. Whether that second insert was a retry or a reuse of
//! the id for something else is decided by comparing the digest of the content, in the same
//! transaction.

use crate::pool::BeginSafely;
use std::time::Duration;

use sagat_application::definitions::DefinitionStore;
use sagat_application::error::StoreError;
use sagat_application::events::{
    Admission, Delivery, EndpointRecord, EventStore, NewEvent, StoredEvent,
};
use sagat_application::secrets::StoredSecret;
use sagat_domain::audit::AuditRecord;
use sagat_domain::events::{
    EndpointId, EndpointLimits, EndpointStatus, EventEnvelope, EventId, EventKey, InboxStatus,
};
use sagat_domain::execution::{CorrelationKey, RunId};
use sagat_domain::identity::{Namespace, NamespaceId, Slug, TenantId};
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::EventType;
use sqlx::Row;
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::quotas::{ensure_room_for_endpoint, lock_event_endpoints, lock_event_sources};

/// `SELECT` of endpoints ending in `$tail`. A macro so the result is a string literal: `sqlx` only
/// accepts literal SQL.
macro_rules! select_endpoints {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, slug, secret_name, default_event_type, ",
            "dedupe_ttl_seconds, skew_seconds, max_body_bytes, rate_per_second, status, ",
            "created_by, created_at, updated_at FROM event_endpoints ",
            $tail
        )
    };
}

fn seconds(value: i32) -> Result<Duration, StoreError> {
    u64::try_from(value)
        .map(Duration::from_secs)
        .map_err(|_| corrupt("event endpoint setting"))
}

fn endpoint_from_row(row: &PgRow) -> Result<EndpointRecord, StoreError> {
    let slug: String = col(row, "slug")?;
    let secret: String = col(row, "secret_name")?;
    let default_type: Option<String> = col(row, "default_event_type")?;
    let status: String = col(row, "status")?;
    let body: i32 = col(row, "max_body_bytes")?;
    let rate: i32 = col(row, "rate_per_second")?;
    Ok(EndpointRecord {
        id: EndpointId::new(col::<Uuid>(row, "id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        slug: Slug::parse(&slug).map_err(|_| corrupt("event endpoint slug"))?,
        secret_name: SecretName::parse(&secret).map_err(|_| corrupt("event endpoint secret"))?,
        default_event_type: default_type
            .map(|t| EventType::parse(&t).map_err(|_| corrupt("event endpoint type")))
            .transpose()?,
        limits: EndpointLimits::new(
            seconds(col(row, "dedupe_ttl_seconds")?)?,
            seconds(col(row, "skew_seconds")?)?,
            usize::try_from(body).map_err(|_| corrupt("event endpoint body limit"))?,
            u32::try_from(rate).map_err(|_| corrupt("event endpoint rate"))?,
        )
        .map_err(|_| corrupt("event endpoint limits"))?,
        status: status
            .parse::<EndpointStatus>()
            .map_err(|()| corrupt("event endpoint status"))?,
        created_by: col(row, "created_by")?,
        created_at: col(row, "created_at")?,
        updated_at: col(row, "updated_at")?,
    })
}

fn secs(value: Duration) -> i32 {
    i32::try_from(value.as_secs()).unwrap_or(i32::MAX)
}

fn envelope_from_row(row: &PgRow) -> Result<EventEnvelope, StoreError> {
    let source: String = col(row, "source")?;
    let id: String = col(row, "event_id")?;
    let kind: String = col(row, "event_type")?;
    let subject: Option<String> = col(row, "subject")?;
    let correlation: Option<String> = col(row, "correlation_key")?;
    EventEnvelope::new(
        EventKey {
            source: Slug::parse(&source).map_err(|_| corrupt("event source"))?,
            event_id: EventId::parse(&id).map_err(|_| corrupt("event id"))?,
        },
        EventType::parse(&kind).map_err(|_| corrupt("event type"))?,
        subject.as_deref(),
        correlation
            .map(|k| CorrelationKey::parse(&k).map_err(|_| corrupt("event correlation key")))
            .transpose()?,
    )
    .map_err(|_| corrupt("event"))
}

impl EventStore for PgRunStore {
    async fn namespace_by_slug(
        &self,
        tenant: TenantId,
        slug: &Slug,
    ) -> Result<Option<Namespace>, StoreError> {
        self.definitions.find_namespace_by_slug(tenant, slug).await
    }

    async fn create_endpoint(
        &self,
        endpoint: &EndpointRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_event_endpoints(&mut tx, endpoint.tenant_id, endpoint.namespace_id).await?;
        lock_event_sources(&mut tx, endpoint.tenant_id, endpoint.namespace_id).await?;
        // A slug names one source of events in a namespace: an endpoint or a connector.
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM event_connectors \
                            WHERE tenant_id = $1 AND namespace_id = $2 AND slug = $3)",
        )
        .bind(endpoint.tenant_id.as_uuid())
        .bind(endpoint.namespace_id.as_uuid())
        .bind(endpoint.slug.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if taken {
            return Err(StoreError::Conflict(
                sagat_application::error::ConflictKind::EventEndpointSlug,
            ));
        }
        ensure_room_for_endpoint(&mut tx, endpoint.tenant_id, endpoint.namespace_id, limit).await?;
        sqlx::query(
            "INSERT INTO event_endpoints \
             (id, tenant_id, namespace_id, slug, secret_name, default_event_type, \
              dedupe_ttl_seconds, skew_seconds, max_body_bytes, rate_per_second, status, \
              created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $13)",
        )
        .bind(endpoint.id.as_uuid())
        .bind(endpoint.tenant_id.as_uuid())
        .bind(endpoint.namespace_id.as_uuid())
        .bind(endpoint.slug.as_str())
        .bind(endpoint.secret_name.as_str())
        .bind(endpoint.default_event_type.as_ref().map(EventType::as_str))
        .bind(secs(endpoint.limits.dedupe_ttl()))
        .bind(secs(endpoint.limits.skew()))
        .bind(i32::try_from(endpoint.limits.max_body()).unwrap_or(i32::MAX))
        .bind(i32::try_from(endpoint.limits.rate_per_second()).unwrap_or(i32::MAX))
        .bind(endpoint.status.as_str())
        .bind(&endpoint.created_by)
        .bind(endpoint.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn update_endpoint(
        &self,
        endpoint: &EndpointRecord,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let changed = sqlx::query(
            "UPDATE event_endpoints SET secret_name = $3, default_event_type = $4, \
                 dedupe_ttl_seconds = $5, skew_seconds = $6, max_body_bytes = $7, \
                 rate_per_second = $8, status = $9, updated_at = $10 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(endpoint.tenant_id.as_uuid())
        .bind(endpoint.id.as_uuid())
        .bind(endpoint.secret_name.as_str())
        .bind(endpoint.default_event_type.as_ref().map(EventType::as_str))
        .bind(secs(endpoint.limits.dedupe_ttl()))
        .bind(secs(endpoint.limits.skew()))
        .bind(i32::try_from(endpoint.limits.max_body()).unwrap_or(i32::MAX))
        .bind(i32::try_from(endpoint.limits.rate_per_second()).unwrap_or(i32::MAX))
        .bind(endpoint.status.as_str())
        .bind(endpoint.updated_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected()
            > 0;
        if changed {
            insert_audit(&mut tx, audit).await?;
            tx.commit().await.map_err(store_error)?;
        }
        Ok(changed)
    }

    async fn delete_endpoint(
        &self,
        tenant: TenantId,
        id: EndpointId,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let removed = sqlx::query("DELETE FROM event_endpoints WHERE tenant_id = $1 AND id = $2")
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected()
            > 0;
        if removed {
            insert_audit(&mut tx, audit).await?;
            tx.commit().await.map_err(store_error)?;
        }
        Ok(removed)
    }

    async fn endpoint(
        &self,
        tenant: TenantId,
        id: EndpointId,
    ) -> Result<Option<EndpointRecord>, StoreError> {
        sqlx::query(select_endpoints!("WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|row| endpoint_from_row(&row))
            .transpose()
    }

    async fn endpoints(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<EndpointRecord>, StoreError> {
        sqlx::query(select_endpoints!(
            "WHERE tenant_id = $1 AND namespace_id = $2 ORDER BY slug"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(endpoint_from_row)
        .collect()
    }

    async fn endpoint_by_id(&self, id: EndpointId) -> Result<Option<EndpointRecord>, StoreError> {
        sqlx::query(select_endpoints!("WHERE id = $1"))
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|row| endpoint_from_row(&row))
            .transpose()
    }

    async fn signing_secrets(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        name: &SecretName,
        limit: u32,
    ) -> Result<Vec<StoredSecret>, StoreError> {
        let mut conn = self.pool.acquire().await.map_err(store_error)?;
        crate::secrets::newest_versions(&mut conn, tenant, namespace, name, limit).await
    }

    async fn admit_event(
        &self,
        event: &NewEvent,
        backlog_limit: u64,
        audit: Option<&AuditRecord>,
    ) -> Result<Admission, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let admission = admit_within(&mut tx, event, backlog_limit).await?;
        if admission == Admission::Accepted {
            if let Some(audit) = audit {
                insert_audit(&mut tx, audit).await?;
            }
            tx.commit().await.map_err(store_error)?;
        }
        // Anything else wrote nothing that matters: the transaction is rolled back.
        Ok(admission)
    }

    async fn event(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        key: &EventKey,
    ) -> Result<Option<StoredEvent>, StoreError> {
        let Some(row) = sqlx::query(
            "SELECT source, event_id, event_type, correlation_key, subject, occurred_at, payload, \
                    received_at, expires_at, status, attempts, last_error, dispatched_at \
             FROM inbox_events \
             WHERE tenant_id = $1 AND namespace_id = $2 AND source = $3 AND event_id = $4",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(key.source.as_str())
        .bind(key.event_id.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let deliveries = sqlx::query(
            "SELECT consumer, run_id, node, trigger_id, delivered_at FROM event_deliveries \
             WHERE tenant_id = $1 AND namespace_id = $2 AND source = $3 AND event_id = $4 \
             ORDER BY delivered_at, consumer",
        )
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .bind(key.source.as_str())
        .bind(key.event_id.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|d| {
            Ok(Delivery {
                consumer: col(d, "consumer")?,
                run_id: col::<Option<Uuid>>(d, "run_id")?.map(RunId::new),
                node: col(d, "node")?,
                trigger_id: col(d, "trigger_id")?,
                delivered_at: col(d, "delivered_at")?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
        let status: String = col(&row, "status")?;
        let attempts: i32 = col(&row, "attempts")?;
        Ok(Some(StoredEvent {
            tenant_id: tenant,
            namespace_id: namespace,
            envelope: envelope_from_row(&row)?,
            occurred_at: col(&row, "occurred_at")?,
            payload: col(&row, "payload")?,
            received_at: col(&row, "received_at")?,
            expires_at: col(&row, "expires_at")?,
            status: status
                .parse::<InboxStatus>()
                .map_err(|()| corrupt("inbox status"))?,
            attempts: u32::try_from(attempts).map_err(|_| corrupt("inbox attempts"))?,
            last_error: col(&row, "last_error")?,
            dispatched_at: col(&row, "dispatched_at")?,
            deliveries,
        }))
    }
}

/// Admits `event` in the caller's transaction: the deduplication, the backlog check and the insert,
/// without committing. The caller commits when the outcome is [`Admission::Accepted`] (or, when it
/// moves something else along with the event, also when it is [`Admission::Duplicate`]).
pub(crate) async fn admit_within(
    tx: &mut sqlx::PgConnection,
    event: &NewEvent,
    backlog_limit: u64,
) -> Result<Admission, StoreError> {
    let key = &event.envelope.key;
    // An id whose window has ended is a new event again; the purge job is not the only thing
    // that can notice. An event that was never dispatched is not forgotten with its window.
    sqlx::query(
        "DELETE FROM inbox_events \
         WHERE tenant_id = $1 AND namespace_id = $2 AND source = $3 AND event_id = $4 \
           AND expires_at <= clock_timestamp() AND status <> 'PENDING'",
    )
    .bind(event.tenant_id.as_uuid())
    .bind(event.namespace_id.as_uuid())
    .bind(key.source.as_str())
    .bind(key.event_id.as_str())
    .execute(&mut *tx)
    .await
    .map_err(store_error)?;

    if let Some(known) = remembered(&mut *tx, event).await? {
        return Ok(known);
    }

    // The backlog is counted up to its limit only: a full count of a deep backlog would make
    // the refusal as expensive as the work it protects.
    let pending: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT 1 FROM inbox_events \
                               WHERE tenant_id = $1 AND status = 'PENDING' LIMIT $2) AS p",
    )
    .bind(event.tenant_id.as_uuid())
    .bind(i64::try_from(backlog_limit).unwrap_or(i64::MAX))
    .fetch_one(&mut *tx)
    .await
    .map_err(store_error)?;
    if u64::try_from(pending).unwrap_or(u64::MAX) >= backlog_limit {
        return Ok(Admission::Backlogged);
    }

    let inserted = sqlx::query(
        "INSERT INTO inbox_events \
         (tenant_id, namespace_id, source, event_id, event_type, correlation_key, subject, \
          occurred_at, payload, payload_digest, received_at, expires_at, next_attempt_at, \
          trace_id, trace_parent_span) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, clock_timestamp(), \
                 clock_timestamp() + make_interval(secs => $11), clock_timestamp(), $12, $13) \
         ON CONFLICT (tenant_id, namespace_id, source, event_id) DO NOTHING",
    )
    .bind(event.tenant_id.as_uuid())
    .bind(event.namespace_id.as_uuid())
    .bind(key.source.as_str())
    .bind(key.event_id.as_str())
    .bind(event.envelope.event_type.as_str())
    .bind(
        event
            .envelope
            .correlation_key
            .as_ref()
            .map(CorrelationKey::as_str),
    )
    .bind(event.envelope.subject.as_deref())
    .bind(event.occurred_at)
    .bind(&event.payload)
    .bind(&event.digest)
    .bind(event.dedupe_ttl.as_secs_f64())
    .bind(event.trace.as_ref().map(|t| t.trace_id.as_str()))
    .bind(event.trace.as_ref().and_then(|t| t.parent_span.as_deref()))
    .execute(&mut *tx)
    .await
    .map_err(store_error)?
    .rows_affected()
        > 0;
    if inserted {
        return Ok(Admission::Accepted);
    }
    // Somebody admitted the same key between the look and the insert: they decide.
    remembered(&mut *tx, event)
        .await?
        .ok_or(StoreError::Internal {
            detail: "an inbox row vanished while it was being admitted".to_owned(),
        })
}

/// What the inbox already says about this event's key: a retry (same content) or a reuse of the id.
async fn remembered(
    conn: &mut sqlx::PgConnection,
    event: &NewEvent,
) -> Result<Option<Admission>, StoreError> {
    let key = &event.envelope.key;
    let existing: Option<Vec<u8>> = sqlx::query(
        "SELECT payload_digest FROM inbox_events \
         WHERE tenant_id = $1 AND namespace_id = $2 AND source = $3 AND event_id = $4",
    )
    .bind(event.tenant_id.as_uuid())
    .bind(event.namespace_id.as_uuid())
    .bind(key.source.as_str())
    .bind(key.event_id.as_str())
    .fetch_optional(conn)
    .await
    .map_err(store_error)?
    .map(|row| row.try_get("payload_digest").map_err(store_error))
    .transpose()?;
    Ok(existing.map(|digest| {
        if digest == event.digest {
            Admission::Duplicate
        } else {
            Admission::Reused
        }
    }))
}
