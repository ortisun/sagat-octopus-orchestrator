//! Transactional inbox and dead-letter value reads for feature 016.

use std::collections::BTreeMap;

use sagat_application::error::StoreError;
use sagat_application::event_payloads::{
    DeadLetterRead, EventPayloadAudits, EventPayloadEvaluator, EventPayloadFailure,
    EventPayloadRead, EventPayloadScope, EventPayloadStore, InboxEventRead, PreparedDeadLetter,
    PreparedInboxEvent, SelectedDeadLetter, SelectedInboxEvent,
};
use sagat_application::events::Delivery;
use sagat_application::payloads::StoredPayloadBytes;
use sagat_domain::connectors::{ConnectorId, DeadLetter};
use sagat_domain::events::{EventEnvelope, EventId, EventKey, InboxStatus};
use sagat_domain::execution::{CorrelationKey, RunId};
use sagat_domain::identity::{NamespaceId, Permission, Role, Slug, TenantId};
use sagat_domain::workflow::EventType;
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{best_effort_denial_audit, col, corrupt, insert_audit, store_error};
use crate::pool::BeginSafely;
use crate::reveal_grants::has_effective_payload_reveal_grant;

impl EventPayloadStore for PgRunStore {
    async fn event_scope(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        key: &EventKey,
    ) -> Result<Option<EventPayloadScope>, StoreError> {
        sqlx::query_scalar::<_,Uuid>("SELECT namespace_id FROM inbox_events WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4")
            .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(key.source.as_str()).bind(key.event_id.as_str())
            .fetch_optional(&self.pool).await.map_err(store_error).map(|v|v.map(|id|EventPayloadScope{namespace_id:NamespaceId::new(id)}))
    }
    async fn dead_letter_scope(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
        connector: ConnectorId,
        letter: Option<Uuid>,
    ) -> Result<Option<EventPayloadScope>, StoreError> {
        let id = if let Some(letter) = letter {
            sqlx::query_scalar::<_,Uuid>("SELECT d.namespace_id FROM connector_dead_letters d JOIN event_connectors c ON c.tenant_id=d.tenant_id AND c.id=d.connector_id WHERE d.tenant_id=$1 AND d.namespace_id=$2 AND d.connector_id=$3 AND d.id=$4")
                .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(connector.as_uuid()).bind(letter).fetch_optional(&self.pool).await.map_err(store_error)?
        } else {
            sqlx::query_scalar::<_,Uuid>("SELECT namespace_id FROM event_connectors WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3")
                .bind(tenant.as_uuid()).bind(namespace.as_uuid()).bind(connector.as_uuid()).fetch_optional(&self.pool).await.map_err(store_error)?
        };
        Ok(id.map(|id| EventPayloadScope {
            namespace_id: NamespaceId::new(id),
        }))
    }
    async fn read_event(
        &self,
        p: &PreparedInboxEvent,
        e: &dyn EventPayloadEvaluator,
        a: &EventPayloadAudits,
    ) -> Result<EventPayloadRead<InboxEventRead>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let Some(row) = select_event(&mut tx, p).await? else {
            return Ok(EventPayloadRead::Missing);
        };
        lock_authority(&mut tx, p.tenant_id(), p.namespace_id()).await?;
        let Some(role) = current_role(&mut tx, p.tenant_id(), p.subject().as_str()).await? else {
            return Ok(EventPayloadRead::Refused(
                EventPayloadFailure::EventReadNotGranted,
            ));
        };
        let grant = has_effective_payload_reveal_grant(
            &mut tx,
            p.tenant_id(),
            p.namespace_id(),
            p.subject(),
        )
        .await?;
        let selected = event_from_row(&mut tx, p, &row).await?;
        finish(
            tx,
            e.event(selected, role, grant),
            a,
            p.reveal() == sagat_application::event_payloads::EventReveal::Pii,
        )
        .await
    }
    async fn read_dead_letter(
        &self,
        p: &PreparedDeadLetter,
        e: &dyn EventPayloadEvaluator,
        a: &EventPayloadAudits,
    ) -> Result<EventPayloadRead<DeadLetterRead>, StoreError> {
        let Some(id) = p.letter_id() else {
            return Ok(EventPayloadRead::Missing);
        };
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // Dead-letter contents are immutable to the runtime role. Read the committed value without
        // a row lock, which would require granting UPDATE just to disclose this retained record.
        // Current role/grant checks below still hold their authority locks through the audit commit.
        let Some(row)=sqlx::query("SELECT id,connector_id,position,reason,headers,body,truncated,received_at FROM connector_dead_letters WHERE tenant_id=$1 AND namespace_id=$2 AND connector_id=$3 AND id=$4")
            .bind(p.tenant_id().as_uuid()).bind(p.namespace_id().as_uuid()).bind(p.connector_id().as_uuid()).bind(id).fetch_optional(&mut *tx).await.map_err(store_error)? else{return Ok(EventPayloadRead::Missing)};
        lock_authority(&mut tx, p.tenant_id(), p.namespace_id()).await?;
        let Some(role) = current_role(&mut tx, p.tenant_id(), p.subject().as_str()).await? else {
            return Ok(EventPayloadRead::Refused(
                EventPayloadFailure::EventReadNotGranted,
            ));
        };
        let grant = has_effective_payload_reveal_grant(
            &mut tx,
            p.tenant_id(),
            p.namespace_id(),
            p.subject(),
        )
        .await?;
        finish(
            tx,
            e.dead_letter(letter_from_row(&row)?, role, grant),
            a,
            p.reveal() == sagat_application::event_payloads::EventReveal::Pii,
        )
        .await
    }
    async fn list_dead_letters(
        &self,
        p: &PreparedDeadLetter,
        before: Option<Uuid>,
        limit: u32,
        e: &dyn EventPayloadEvaluator,
    ) -> Result<EventPayloadRead<Vec<DeadLetterRead>>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let exists=sqlx::query_scalar::<_,Uuid>("SELECT id FROM event_connectors WHERE tenant_id=$1 AND namespace_id=$2 AND id=$3 FOR SHARE")
            .bind(p.tenant_id().as_uuid()).bind(p.namespace_id().as_uuid()).bind(p.connector_id().as_uuid()).fetch_optional(&mut *tx).await.map_err(store_error)?;
        if exists.is_none() {
            return Ok(EventPayloadRead::Missing);
        }
        lock_authority(&mut tx, p.tenant_id(), p.namespace_id()).await?;
        let Some(role) = current_role(&mut tx, p.tenant_id(), p.subject().as_str()).await? else {
            return Ok(EventPayloadRead::Refused(
                EventPayloadFailure::EventReadNotGranted,
            ));
        };
        if !role.grants(Permission::EventRead) {
            return Ok(EventPayloadRead::Refused(
                EventPayloadFailure::EventReadNotGranted,
            ));
        }
        let grant = has_effective_payload_reveal_grant(
            &mut tx,
            p.tenant_id(),
            p.namespace_id(),
            p.subject(),
        )
        .await?;
        let rows=sqlx::query("SELECT id,connector_id,position,reason,headers,body,truncated,received_at FROM connector_dead_letters WHERE tenant_id=$1 AND namespace_id=$2 AND connector_id=$3 AND ($4::uuid IS NULL OR id<$4) ORDER BY id DESC LIMIT $5")
            .bind(p.tenant_id().as_uuid()).bind(p.namespace_id().as_uuid()).bind(p.connector_id().as_uuid()).bind(before).bind(i64::from(limit)).fetch_all(&mut *tx).await.map_err(store_error)?;
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            match e.dead_letter(letter_from_row(&row)?, role, grant) {
                Ok(v) => out.push(v),
                Err(f) => return Ok(EventPayloadRead::Refused(f)),
            }
        }
        tx.commit().await.map_err(store_error)?;
        Ok(EventPayloadRead::Ready(out))
    }
}

async fn finish<T>(
    mut tx: Transaction<'_, Postgres>,
    result: Result<T, EventPayloadFailure>,
    audits: &EventPayloadAudits,
    audit_success: bool,
) -> Result<EventPayloadRead<T>, StoreError> {
    let read = match result {
        Ok(value) => {
            if audit_success {
                insert_audit(&mut tx, &audits.revealed).await.map_err(|e| {
                    StoreError::Unavailable {
                        detail: format!("event payload audit could not be appended: {e}"),
                    }
                })?;
            }
            EventPayloadRead::Ready(value)
        }
        Err(EventPayloadFailure::PiiRevealNotGranted) => {
            best_effort_denial_audit(tx, &audits.denied).await;
            return Ok(EventPayloadRead::Refused(
                EventPayloadFailure::PiiRevealNotGranted,
            ));
        }
        Err(f) => EventPayloadRead::Refused(f),
    };
    tx.commit().await.map_err(store_error)?;
    Ok(read)
}

async fn select_event<'a>(
    tx: &mut Transaction<'a, Postgres>,
    p: &PreparedInboxEvent,
) -> Result<Option<PgRow>, StoreError> {
    sqlx::query("SELECT source,event_id,event_type,correlation_key,subject,occurred_at,payload,received_at,expires_at,status,attempts,last_error,dispatched_at FROM inbox_events WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4 FOR SHARE")
        .bind(p.tenant_id().as_uuid()).bind(p.namespace_id().as_uuid()).bind(p.key().source.as_str()).bind(p.key().event_id.as_str()).fetch_optional(&mut **tx).await.map_err(store_error)
}
async fn event_from_row(
    tx: &mut Transaction<'_, Postgres>,
    p: &PreparedInboxEvent,
    row: &PgRow,
) -> Result<SelectedInboxEvent, StoreError> {
    let deliveries=sqlx::query("SELECT consumer,run_id,node,trigger_id,delivered_at FROM event_deliveries WHERE tenant_id=$1 AND namespace_id=$2 AND source=$3 AND event_id=$4 ORDER BY delivered_at,consumer")
        .bind(p.tenant_id().as_uuid()).bind(p.namespace_id().as_uuid()).bind(p.key().source.as_str()).bind(p.key().event_id.as_str()).fetch_all(&mut **tx).await.map_err(store_error)?
        .iter().map(|d|Ok(Delivery{consumer:col(d,"consumer")?,run_id:col::<Option<Uuid>>(d,"run_id")?.map(RunId::new),node:col(d,"node")?,trigger_id:col(d,"trigger_id")?,delivered_at:col(d,"delivered_at")?})).collect::<Result<Vec<_>,StoreError>>()?;
    let source: String = col(row, "source")?;
    let event_id: String = col(row, "event_id")?;
    let event_type: String = col(row, "event_type")?;
    let correlation: Option<String> = col(row, "correlation_key")?;
    let status: String = col(row, "status")?;
    let attempts: i32 = col(row, "attempts")?;
    Ok(SelectedInboxEvent {
        envelope: EventEnvelope::new(
            EventKey {
                source: Slug::parse(&source).map_err(|_| corrupt("event source"))?,
                event_id: EventId::parse(&event_id).map_err(|_| corrupt("event id"))?,
            },
            EventType::parse(&event_type).map_err(|_| corrupt("event type"))?,
            col::<Option<String>>(row, "subject")?.as_deref(),
            correlation
                .map(|v| CorrelationKey::parse(&v).map_err(|_| corrupt("event correlation")))
                .transpose()?,
        )
        .map_err(|_| corrupt("event envelope"))?,
        occurred_at: col(row, "occurred_at")?,
        payload: StoredPayloadBytes::new(col(row, "payload")?),
        received_at: col(row, "received_at")?,
        expires_at: col(row, "expires_at")?,
        status: status
            .parse::<InboxStatus>()
            .map_err(|_| corrupt("event status"))?,
        attempts: u32::try_from(attempts).map_err(|_| corrupt("event attempts"))?,
        last_error: col(row, "last_error")?,
        dispatched_at: col(row, "dispatched_at")?,
        deliveries,
    })
}
fn letter_from_row(row: &PgRow) -> Result<SelectedDeadLetter, StoreError> {
    let reason: String = col(row, "reason")?;
    let headers: Value = col(row, "headers")?;
    let headers: BTreeMap<String, String> = headers
        .as_object()
        .map(|v| {
            v.iter()
                .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    Ok(SelectedDeadLetter {
        id: col(row, "id")?,
        connector_id: ConnectorId::new(col(row, "connector_id")?),
        position: col(row, "position")?,
        reason: reason
            .parse::<DeadLetter>()
            .map_err(|_| corrupt("dead letter reason"))?,
        headers,
        body: StoredPayloadBytes::new(col(row, "body")?),
        truncated: col(row, "truncated")?,
        received_at: col(row, "received_at")?,
    })
}
async fn lock_authority(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    namespace: NamespaceId,
) -> Result<(), StoreError> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT environment FROM public.sagat_lock_namespace($1,$2)")
            .bind(tenant.as_uuid())
            .bind(namespace.as_uuid())
            .fetch_optional(&mut **tx)
            .await
            .map_err(store_error)?;
    found.map(|_| ()).ok_or(StoreError::MissingReference)
}
async fn current_role(
    tx: &mut Transaction<'_, Postgres>,
    tenant: TenantId,
    subject: &str,
) -> Result<Option<Role>, StoreError> {
    let role: Option<String> =
        sqlx::query_scalar("SELECT role FROM public.sagat_lock_memberships($1,ARRAY[$2::text])")
            .bind(tenant.as_uuid())
            .bind(subject)
            .fetch_optional(&mut **tx)
            .await
            .map_err(store_error)?;
    role.map(|v| v.parse().map_err(|_| corrupt("membership role")))
        .transpose()
}
