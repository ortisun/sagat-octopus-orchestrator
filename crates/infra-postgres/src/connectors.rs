//! Connectors, their positions and their dead letters in PostgreSQL (`FR-004-005`, `FR-004-008`).
//!
//! What matters here is what happens in one transaction: a message becomes an event and the position
//! of the consumer moves (Kafka), or a message is set aside and the position moves. A consumer that
//! crashes between the two cannot have done one without the other.

use crate::pool::BeginSafely;
use std::collections::BTreeMap;
use std::time::Duration;

use sagat_application::connectors::{
    Checkpoint, ConnectorHealth, ConnectorRecord, ConnectorRuntime, ConnectorStore,
    DeadLetterRecord, MAX_DEAD_BODY, MAX_DEAD_LETTERS, Progress, RawMessage, SetAside,
    keep_headers,
};
use sagat_application::error::{ConflictKind, StoreError};
use sagat_application::events::{Admission, NewEvent};
use sagat_application::secrets::StoredSecret;
use sagat_domain::audit::AuditRecord;
use sagat_domain::connectors::{
    ConnectorId, ConnectorKind, ConnectorSettings, ConnectorStatus, DeadLetter, StartFrom,
};
use sagat_domain::identity::{NamespaceId, Slug, TenantId};
use sagat_domain::secrets::SecretName;
use sagat_domain::workflow::EventType;
use serde_json::{Value, json};
use sqlx::postgres::PgRow;
use uuid::Uuid;

use crate::events::admit_within;
use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::quotas::{ensure_room_for_connector, lock_connectors, lock_event_sources};

macro_rules! select_connectors {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, slug, kind, settings, secret_name, ",
            "default_event_type, rate_per_second, status, last_error, last_message_at, ",
            "consumed_total, dead_lettered_total, created_by, created_at, updated_at ",
            "FROM event_connectors ",
            $tail
        )
    };
}

/// The settings as the JSON that is stored.
fn settings_json(settings: &ConnectorSettings) -> Value {
    match settings {
        ConnectorSettings::Nats {
            servers,
            stream,
            consumer,
            filter_subject,
        } => json!({
            "servers": servers.iter().map(|s| format!("{}{}", if s.tls() { "tls://" } else { "" }, s.address())).collect::<Vec<_>>(),
            "stream": stream,
            "consumer": consumer,
            "filterSubject": filter_subject,
        }),
        ConnectorSettings::Kafka {
            servers,
            topic,
            start_from,
        } => json!({
            "servers": servers.iter().map(|s| format!("{}{}", if s.tls() { "tls://" } else { "" }, s.address())).collect::<Vec<_>>(),
            "topic": topic,
            "startFrom": match start_from { StartFrom::Earliest => "earliest", StartFrom::Latest => "latest" },
        }),
        ConnectorSettings::Sqs { queue_url, region } => {
            json!({ "queueUrl": queue_url, "region": region })
        }
    }
}

/// Reads the settings back through the constructors of the domain, so what is stored is judged by
/// the rules of today.
fn settings_from(kind: ConnectorKind, value: &Value) -> Result<ConnectorSettings, StoreError> {
    let texts = |key: &str| -> Vec<String> {
        value[key]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let text = |key: &str| value[key].as_str().unwrap_or_default().to_owned();
    match kind {
        ConnectorKind::Nats => ConnectorSettings::nats(
            &texts("servers"),
            &text("stream"),
            &text("consumer"),
            value["filterSubject"].as_str(),
        ),
        ConnectorKind::Kafka => ConnectorSettings::kafka(
            &texts("servers"),
            &text("topic"),
            if value["startFrom"] == "latest" {
                StartFrom::Latest
            } else {
                StartFrom::Earliest
            },
        ),
        ConnectorKind::Sqs => ConnectorSettings::sqs(&text("queueUrl"), &text("region")),
    }
    .map_err(|_| corrupt("connector settings"))
}

fn connector_from_row(row: &PgRow) -> Result<ConnectorRecord, StoreError> {
    let slug: String = col(row, "slug")?;
    let kind: String = col(row, "kind")?;
    let kind = kind
        .parse::<ConnectorKind>()
        .map_err(|()| corrupt("connector kind"))?;
    let settings: Value = col(row, "settings")?;
    let secret: Option<String> = col(row, "secret_name")?;
    let default_type: Option<String> = col(row, "default_event_type")?;
    let rate: i32 = col(row, "rate_per_second")?;
    let status: String = col(row, "status")?;
    let consumed: i64 = col(row, "consumed_total")?;
    let dead: i64 = col(row, "dead_lettered_total")?;
    Ok(ConnectorRecord {
        id: ConnectorId::new(col::<Uuid>(row, "id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        slug: Slug::parse(&slug).map_err(|_| corrupt("connector slug"))?,
        settings: settings_from(kind, &settings)?,
        secret_name: secret
            .map(|s| SecretName::parse(&s).map_err(|_| corrupt("connector secret")))
            .transpose()?,
        default_event_type: default_type
            .map(|t| EventType::parse(&t).map_err(|_| corrupt("connector event type")))
            .transpose()?,
        rate_per_second: u32::try_from(rate).map_err(|_| corrupt("connector rate"))?,
        status: status
            .parse::<ConnectorStatus>()
            .map_err(|()| corrupt("connector status"))?,
        health: ConnectorHealth {
            last_error: col(row, "last_error")?,
            last_message_at: col(row, "last_message_at")?,
            consumed: u64::try_from(consumed).unwrap_or(0),
            dead_lettered: u64::try_from(dead).unwrap_or(0),
        },
        created_by: col(row, "created_by")?,
        created_at: col(row, "created_at")?,
        updated_at: col(row, "updated_at")?,
    })
}

fn dead_letter_from_row(row: &PgRow) -> Result<DeadLetterRecord, StoreError> {
    let reason: String = col(row, "reason")?;
    let headers: Value = col(row, "headers")?;
    Ok(DeadLetterRecord {
        id: col(row, "id")?,
        connector_id: ConnectorId::new(col::<Uuid>(row, "connector_id")?),
        position: col(row, "position")?,
        reason: reason
            .parse::<DeadLetter>()
            .map_err(|()| corrupt("dead letter reason"))?,
        headers: headers
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
                    .collect()
            })
            .unwrap_or_default(),
        body: col(row, "body")?,
        truncated: col(row, "truncated")?,
        received_at: col(row, "received_at")?,
    })
}

impl ConnectorStore for PgRunStore {
    async fn create_connector(
        &self,
        connector: &ConnectorRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_connectors(&mut tx, connector.tenant_id, connector.namespace_id).await?;
        lock_event_sources(&mut tx, connector.tenant_id, connector.namespace_id).await?;
        // A slug names one source of events in a namespace: an endpoint or a connector.
        let taken: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM event_endpoints \
                            WHERE tenant_id = $1 AND namespace_id = $2 AND slug = $3)",
        )
        .bind(connector.tenant_id.as_uuid())
        .bind(connector.namespace_id.as_uuid())
        .bind(connector.slug.as_str())
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        if taken {
            return Err(StoreError::Conflict(ConflictKind::ConnectorSlug));
        }
        ensure_room_for_connector(&mut tx, connector.tenant_id, connector.namespace_id, limit)
            .await?;
        sqlx::query(
            "INSERT INTO event_connectors \
             (id, tenant_id, namespace_id, slug, kind, settings, secret_name, default_event_type, \
              rate_per_second, status, created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)",
        )
        .bind(connector.id.as_uuid())
        .bind(connector.tenant_id.as_uuid())
        .bind(connector.namespace_id.as_uuid())
        .bind(connector.slug.as_str())
        .bind(connector.settings.kind().as_str())
        .bind(settings_json(&connector.settings))
        .bind(connector.secret_name.as_ref().map(SecretName::as_str))
        .bind(connector.default_event_type.as_ref().map(EventType::as_str))
        .bind(i32::try_from(connector.rate_per_second).unwrap_or(i32::MAX))
        .bind(connector.status.as_str())
        .bind(&connector.created_by)
        .bind(connector.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn update_connector(
        &self,
        connector: &ConnectorRecord,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let changed = sqlx::query(
            "UPDATE event_connectors SET settings = $3, secret_name = $4, default_event_type = $5, \
                 rate_per_second = $6, status = $7, updated_at = $8 \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(connector.tenant_id.as_uuid())
        .bind(connector.id.as_uuid())
        .bind(settings_json(&connector.settings))
        .bind(connector.secret_name.as_ref().map(SecretName::as_str))
        .bind(connector.default_event_type.as_ref().map(EventType::as_str))
        .bind(i32::try_from(connector.rate_per_second).unwrap_or(i32::MAX))
        .bind(connector.status.as_str())
        .bind(connector.updated_at)
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

    async fn delete_connector(
        &self,
        tenant: TenantId,
        id: ConnectorId,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let removed = sqlx::query("DELETE FROM event_connectors WHERE tenant_id = $1 AND id = $2")
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

    async fn connector(
        &self,
        tenant: TenantId,
        id: ConnectorId,
    ) -> Result<Option<ConnectorRecord>, StoreError> {
        sqlx::query(select_connectors!("WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|row| connector_from_row(&row))
            .transpose()
    }

    async fn connectors(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<ConnectorRecord>, StoreError> {
        sqlx::query(select_connectors!(
            "WHERE tenant_id = $1 AND namespace_id = $2 ORDER BY slug"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(connector_from_row)
        .collect()
    }

    async fn dead_letters(
        &self,
        tenant: TenantId,
        connector: ConnectorId,
        before: Option<Uuid>,
        limit: u32,
    ) -> Result<Vec<DeadLetterRecord>, StoreError> {
        sqlx::query(
            "SELECT id, connector_id, position, reason, headers, body, truncated, received_at \
             FROM connector_dead_letters \
             WHERE tenant_id = $1 AND connector_id = $2 AND ($3::uuid IS NULL OR id < $3) \
             ORDER BY id DESC LIMIT $4",
        )
        .bind(tenant.as_uuid())
        .bind(connector.as_uuid())
        .bind(before)
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(dead_letter_from_row)
        .collect()
    }

    async fn dead_letter(
        &self,
        tenant: TenantId,
        connector: ConnectorId,
        id: Uuid,
    ) -> Result<Option<DeadLetterRecord>, StoreError> {
        sqlx::query(
            "SELECT id, connector_id, position, reason, headers, body, truncated, received_at \
             FROM connector_dead_letters WHERE tenant_id = $1 AND connector_id = $2 AND id = $3",
        )
        .bind(tenant.as_uuid())
        .bind(connector.as_uuid())
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| dead_letter_from_row(&row))
        .transpose()
    }

    async fn discard_dead_letter(
        &self,
        tenant: TenantId,
        connector: ConnectorId,
        id: Uuid,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let removed = sqlx::query(
            "DELETE FROM connector_dead_letters \
             WHERE tenant_id = $1 AND connector_id = $2 AND id = $3",
        )
        .bind(tenant.as_uuid())
        .bind(connector.as_uuid())
        .bind(id)
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

    async fn admit_dead_letter(
        &self,
        connector: &ConnectorRecord,
        dead_letter: Uuid,
        event: &NewEvent,
        backlog_limit: u64,
        audit: &AuditRecord,
    ) -> Result<Admission, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let admission = admit_within(
            &mut tx,
            event,
            backlog_limit,
            Some(crate::content_usage::EventRateSource::Connector(
                connector.id,
            )),
        )
        .await?;
        if matches!(admission, Admission::Accepted | Admission::Duplicate) {
            sqlx::query("DELETE FROM connector_dead_letters WHERE connector_id = $1 AND id = $2")
                .bind(connector.id.as_uuid())
                .bind(dead_letter)
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
            insert_audit(&mut tx, audit).await?;
            tx.commit().await.map_err(store_error)?;
        }
        Ok(admission)
    }
}

/// Moves the checkpoint of a stream from `expected` to the new offset, in the caller's transaction.
/// The first message of a stream has no row to compare with and creates it; after that, a consumer
/// that finds the position somewhere other than where it left it has been replaced, and stops.
async fn move_checkpoint(
    tx: &mut sqlx::PgConnection,
    connector: &ConnectorRecord,
    checkpoint: &Checkpoint,
    expected: i64,
) -> Result<(), StoreError> {
    let moved = sqlx::query(
        "INSERT INTO connector_offsets (connector_id, tenant_id, stream, next_offset) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (connector_id, stream) DO UPDATE \
             SET next_offset = EXCLUDED.next_offset, updated_at = clock_timestamp() \
             WHERE connector_offsets.next_offset = $5",
    )
    .bind(connector.id.as_uuid())
    .bind(connector.tenant_id.as_uuid())
    .bind(&checkpoint.stream)
    .bind(checkpoint.next_offset)
    .bind(expected)
    .execute(tx)
    .await
    .map_err(store_error)?
    .rows_affected();
    if moved == 0 {
        return Err(StoreError::RevisionConflict { current: 0 });
    }
    Ok(())
}

impl ConnectorRuntime for PgRunStore {
    async fn active_connectors(&self) -> Result<Vec<ConnectorRecord>, StoreError> {
        sqlx::query(select_connectors!("WHERE status = 'ACTIVE' ORDER BY id"))
            .fetch_all(&self.pool)
            .await
            .map_err(store_error)?
            .iter()
            .map(connector_from_row)
            .collect()
    }

    async fn credentials(
        &self,
        connector: &ConnectorRecord,
    ) -> Result<Option<StoredSecret>, StoreError> {
        let Some(name) = &connector.secret_name else {
            return Ok(None);
        };
        let mut conn = self.pool.acquire().await.map_err(store_error)?;
        let mut versions = crate::secrets::newest_versions(
            &mut conn,
            connector.tenant_id,
            connector.namespace_id,
            name,
            1,
        )
        .await?;
        Ok(versions.pop())
    }

    async fn admit(
        &self,
        connector: &ConnectorRecord,
        event: &NewEvent,
        backlog_limit: u64,
        checkpoint: Option<(&Checkpoint, i64)>,
    ) -> Result<Admission, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let admission = admit_within(
            &mut tx,
            event,
            backlog_limit,
            Some(crate::content_usage::EventRateSource::Connector(
                connector.id,
            )),
        )
        .await?;
        // The position moves with the event, and only when the event is in (or was in before).
        if matches!(admission, Admission::Accepted | Admission::Duplicate) {
            if let Some((checkpoint, expected)) = checkpoint {
                move_checkpoint(&mut tx, connector, checkpoint, expected).await?;
            }
            tx.commit().await.map_err(store_error)?;
        }
        Ok(admission)
    }

    async fn set_aside(
        &self,
        connector: &ConnectorRecord,
        message: &RawMessage,
        reason: DeadLetter,
        checkpoint: Option<(&Checkpoint, i64)>,
    ) -> Result<SetAside, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let truncated = message.body.len() > MAX_DEAD_BODY;
        let body: &[u8] = &message.body[..message.body.len().min(MAX_DEAD_BODY)];
        let headers: BTreeMap<String, String> = keep_headers(&message.headers);
        let position: String = message.position.chars().take(512).collect();
        // What is kept is bounded, and what is kept is the first: they show what went wrong first,
        // and a flood of poison messages cannot wipe out the evidence that came before it. A message
        // that is already a dead letter is not added (nor counted) again.
        let known: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM connector_dead_letters \
                            WHERE connector_id = $1 AND position = $2)",
        )
        .bind(connector.id.as_uuid())
        .bind(&position)
        .fetch_one(&mut *tx)
        .await
        .map_err(store_error)?;
        let result = if known {
            SetAside::AlreadyKept
        } else {
            let kept = sqlx::query(
                "INSERT INTO connector_dead_letters \
                 (id, tenant_id, namespace_id, connector_id, position, reason, headers, body, truncated) \
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9 \
                 WHERE (SELECT count(*) FROM (SELECT 1 FROM connector_dead_letters \
                                              WHERE connector_id = $4 LIMIT $10) AS kept) < $10 \
                 ON CONFLICT (connector_id, position) DO NOTHING",
            )
            .bind(Uuid::now_v7())
            .bind(connector.tenant_id.as_uuid())
            .bind(connector.namespace_id.as_uuid())
            .bind(connector.id.as_uuid())
            .bind(&position)
            .bind(reason.as_str())
            .bind(json!(headers))
            .bind(body)
            .bind(truncated)
            .bind(i64::try_from(MAX_DEAD_LETTERS).unwrap_or(i64::MAX))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?
            .rows_affected();
            if kept > 0 {
                SetAside::Kept
            } else {
                SetAside::Full
            }
        };
        if let Some((checkpoint, expected)) = checkpoint {
            move_checkpoint(&mut tx, connector, checkpoint, expected).await?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(result)
    }

    async fn checkpoint(
        &self,
        connector: ConnectorId,
        stream: &str,
    ) -> Result<Option<i64>, StoreError> {
        sqlx::query_scalar(
            "SELECT next_offset FROM connector_offsets WHERE connector_id = $1 AND stream = $2",
        )
        .bind(connector.as_uuid())
        .bind(stream)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)
    }

    async fn lease(
        &self,
        connector: ConnectorId,
        stream: &str,
        holder: &str,
        ttl: Duration,
    ) -> Result<bool, StoreError> {
        let held: Option<i32> = sqlx::query_scalar(
            "INSERT INTO connector_leases (connector_id, stream, holder, expires_at) \
             VALUES ($1, $2, $3, clock_timestamp() + make_interval(secs => $4)) \
             ON CONFLICT (connector_id, stream) DO UPDATE \
                 SET holder = EXCLUDED.holder, expires_at = EXCLUDED.expires_at \
                 WHERE connector_leases.holder = EXCLUDED.holder \
                    OR connector_leases.expires_at < clock_timestamp() \
             RETURNING 1",
        )
        .bind(connector.as_uuid())
        .bind(stream)
        .bind(holder)
        .bind(ttl.as_secs_f64())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(held.is_some())
    }

    async fn progress(
        &self,
        connector: ConnectorId,
        progress: &Progress,
    ) -> Result<(), StoreError> {
        let (set_error, error): (bool, Option<&str>) = match &progress.error {
            None => (false, None),
            Some(value) => (true, value.as_deref()),
        };
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let prior: Option<(Uuid, i64)> = sqlx::query_as(
            "SELECT tenant_id,coalesce(octet_length(last_error),0)::bigint FROM event_connectors WHERE id=$1 FOR UPDATE")
            .bind(connector.as_uuid()).fetch_optional(&mut *tx).await.map_err(store_error)?;
        let Some((tenant, prior)) = prior else {
            return Ok(());
        };
        let allocated = if let Some(error) = error {
            Some(
                crate::content_usage::take_log_prefix(
                    &mut tx,
                    TenantId::new(tenant),
                    u64::try_from(prior).map_err(|_| corrupt("connector error bytes"))?,
                    error,
                )
                .await?,
            )
        } else {
            None
        };
        sqlx::query(
            "UPDATE event_connectors SET \
                 consumed_total = consumed_total + $2, \
                 dead_lettered_total = dead_lettered_total + $3, \
                 last_message_at = COALESCE($4, last_message_at), \
                 last_error = CASE WHEN $5 THEN $6 ELSE last_error END, \
                 last_error_truncated = CASE WHEN $5 THEN $7 ELSE last_error_truncated END \
             WHERE id = $1",
        )
        .bind(connector.as_uuid())
        .bind(i64::try_from(progress.consumed).unwrap_or(i64::MAX))
        .bind(i64::try_from(progress.dead_lettered).unwrap_or(i64::MAX))
        .bind(progress.last_message_at)
        .bind(set_error)
        .bind(allocated.as_ref().map(|log| log.text.as_str()))
        .bind(allocated.as_ref().is_some_and(|log| log.truncated))
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        tx.commit().await.map_err(store_error)
    }
}
