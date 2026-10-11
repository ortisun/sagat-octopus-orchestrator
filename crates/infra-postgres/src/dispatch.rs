//! The queries of the event dispatcher (`FR-004-001`, `FR-004-003`, `FR-004-008`).
//!
//! Every instant is the database's, so dispatchers on machines with different clocks agree on what
//! is due. A claim is a short lease on the row, not a lock held across the work: the dispatcher
//! then starts runs and wakes steps in transactions of their own, and the delivery rows those
//! transactions write make a repeated dispatch harmless.

use std::time::Duration;

use sagat_application::dispatch::{
    ClaimedEvent, EventDispatchStore, StoredTrigger, TriggerRegistration, WaitingStep,
};
use sagat_application::error::StoreError;
use sagat_application::identity::TraceLink;
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::events::{EventEnvelope, EventId, EventKey};
use sagat_domain::execution::{CorrelationKey, RunId};
use sagat_domain::identity::{NamespaceId, Slug, TenantId};
use sagat_domain::triggers::{CronSpec, TriggerKind};
use sagat_domain::workflow::{EventType, Expression, NodeId};
use sqlx::postgres::PgRow;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, store_error};

/// The columns of an inbox row the dispatcher reads.
macro_rules! select_events {
    ($tail:literal) => {
        concat!(
            "SELECT e.tenant_id, e.namespace_id, e.source, e.event_id, e.event_type, ",
            "e.correlation_key, e.subject, e.occurred_at, e.payload, e.received_at, e.attempts, ",
            "e.trace_id, e.trace_parent_span ",
            "FROM inbox_events e ",
            $tail
        )
    };
}

fn claimed_from_row(row: &PgRow) -> Result<ClaimedEvent, StoreError> {
    let source: String = col(row, "source")?;
    let id: String = col(row, "event_id")?;
    let kind: String = col(row, "event_type")?;
    let subject: Option<String> = col(row, "subject")?;
    let correlation: Option<String> = col(row, "correlation_key")?;
    let attempts: i32 = col(row, "attempts")?;
    Ok(ClaimedEvent {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        envelope: EventEnvelope::new(
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
        .map_err(|_| corrupt("event"))?,
        occurred_at: col(row, "occurred_at")?,
        payload: col(row, "payload")?,
        received_at: col(row, "received_at")?,
        attempts: u32::try_from(attempts).map_err(|_| corrupt("event attempts"))?,
        trace: col::<Option<String>>(row, "trace_id")?.map(|trace_id| TraceLink {
            trace_id,
            parent_span: col(row, "trace_parent_span").ok().flatten(),
        }),
    })
}

fn trigger_from_row(row: &PgRow) -> Result<StoredTrigger, StoreError> {
    let kind: String = col(row, "kind")?;
    let source: Option<String> = col(row, "match_source")?;
    let event_type: Option<String> = col(row, "match_type")?;
    let filter: Option<String> = col(row, "filter")?;
    let cron: Option<String> = col(row, "cron")?;
    let timezone: Option<String> = col(row, "timezone")?;
    Ok(StoredTrigger {
        registration: TriggerRegistration {
            id: col(row, "id")?,
            kind: kind
                .parse::<TriggerKind>()
                .map_err(|()| corrupt("trigger kind"))?,
            source: source
                .map(|s| Slug::parse(&s).map_err(|_| corrupt("trigger source")))
                .transpose()?,
            event_type: event_type
                .map(|t| EventType::parse(&t).map_err(|_| corrupt("trigger type")))
                .transpose()?,
            filter: filter
                .map(|f| Expression::parse(&f).map_err(|_| corrupt("trigger filter")))
                .transpose()?,
            schedule: match (cron, timezone) {
                (Some(expression), Some(zone)) => Some(
                    CronSpec::parse(&expression, &zone).map_err(|_| corrupt("trigger schedule"))?,
                ),
                _ => None,
            },
        },
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        workflow_id: WorkflowId::new(col::<Uuid>(row, "workflow_id")?),
        version_id: WorkflowVersionId::new(col::<Uuid>(row, "version_id")?),
        next_fire_at: col(row, "next_fire_at")?,
        db_now: col(row, "db_now")?,
    })
}

fn waiting_from_row(row: &PgRow) -> Result<WaitingStep, StoreError> {
    let node: String = col(row, "node")?;
    let kind: String = col(row, "event_type")?;
    let key: String = col(row, "correlation_key")?;
    Ok(WaitingStep {
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        run_id: RunId::new(col::<Uuid>(row, "run_id")?),
        node: NodeId::parse_stored(&node).map_err(|_| corrupt("step node"))?,
        event_type: EventType::parse(&kind).map_err(|_| corrupt("step event type"))?,
        correlation_key: CorrelationKey::parse(&key)
            .map_err(|_| corrupt("step correlation key"))?,
        since: col(row, "event_since")?,
    })
}

macro_rules! select_triggers {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, workflow_id, version_id, kind, match_source, ",
            "match_type, filter, cron, timezone, next_fire_at, clock_timestamp() AS db_now ",
            "FROM workflow_triggers ",
            $tail
        )
    };
}

/// A delay as the interval the database adds to its clock.
fn seconds(delay: Duration) -> f64 {
    delay.as_secs_f64()
}

impl EventDispatchStore for PgRunStore {
    async fn claim_events(
        &self,
        limit: u32,
        lease: Duration,
    ) -> Result<Vec<ClaimedEvent>, StoreError> {
        let rows = sqlx::query(
            "WITH due AS ( \
                 SELECT tenant_id, namespace_id, source, event_id FROM inbox_events \
                 WHERE status = 'PENDING' AND next_attempt_at <= clock_timestamp() \
                 ORDER BY next_attempt_at LIMIT $1 FOR UPDATE SKIP LOCKED) \
             UPDATE inbox_events e \
             SET next_attempt_at = clock_timestamp() + make_interval(secs => $2) \
             FROM due \
             WHERE e.tenant_id = due.tenant_id AND e.namespace_id = due.namespace_id \
               AND e.source = due.source AND e.event_id = due.event_id \
             RETURNING e.tenant_id, e.namespace_id, e.source, e.event_id, e.event_type, \
                       e.correlation_key, e.subject, e.occurred_at, e.payload, e.received_at, \
                       e.attempts, e.trace_id, e.trace_parent_span",
        )
        .bind(i64::from(limit))
        .bind(seconds(lease))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?;
        let mut events: Vec<ClaimedEvent> = rows
            .iter()
            .map(claimed_from_row)
            .collect::<Result<_, _>>()?;
        events.sort_by_key(|e| e.received_at);
        Ok(events)
    }

    async fn finish_event(&self, event: &ClaimedEvent) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE inbox_events SET status = 'DISPATCHED', dispatched_at = clock_timestamp(), \
                 last_error = NULL \
             WHERE tenant_id = $1 AND namespace_id = $2 AND source = $3 AND event_id = $4 \
               AND status = 'PENDING'",
        )
        .bind(event.tenant_id.as_uuid())
        .bind(event.namespace_id.as_uuid())
        .bind(event.envelope.key.source.as_str())
        .bind(event.envelope.key.event_id.as_str())
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn retry_event(
        &self,
        event: &ClaimedEvent,
        error: &str,
        delay: Duration,
        dead: bool,
        counted: bool,
    ) -> Result<(), StoreError> {
        // What a dispatch says about its failure is ours to write, never a payload: keep it short.
        let error: String = error.chars().take(500).collect();
        sqlx::query(
            "UPDATE inbox_events SET attempts = attempts + CASE WHEN $8 THEN 1 ELSE 0 END, \
                 last_error = $5, \
                 next_attempt_at = clock_timestamp() + make_interval(secs => $6), \
                 status = CASE WHEN $7 THEN 'DEAD' ELSE 'PENDING' END \
             WHERE tenant_id = $1 AND namespace_id = $2 AND source = $3 AND event_id = $4 \
               AND status = 'PENDING'",
        )
        .bind(event.tenant_id.as_uuid())
        .bind(event.namespace_id.as_uuid())
        .bind(event.envelope.key.source.as_str())
        .bind(event.envelope.key.event_id.as_str())
        .bind(error)
        .bind(seconds(delay))
        .bind(dead)
        .bind(counted)
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn triggers_matching(
        &self,
        event: &ClaimedEvent,
    ) -> Result<Vec<StoredTrigger>, StoreError> {
        sqlx::query(select_triggers!(
            "WHERE tenant_id = $1 AND namespace_id = $2 AND kind <> 'cron' \
               AND (match_source IS NULL OR match_source = $3) \
               AND (match_type IS NULL OR match_type = $4) \
             ORDER BY id"
        ))
        .bind(event.tenant_id.as_uuid())
        .bind(event.namespace_id.as_uuid())
        .bind(event.envelope.key.source.as_str())
        .bind(event.envelope.event_type.as_str())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(trigger_from_row)
        .collect()
    }

    async fn waits_for_event(
        &self,
        event: &ClaimedEvent,
        after: Option<(RunId, &NodeId)>,
        limit: u32,
    ) -> Result<Vec<WaitingStep>, StoreError> {
        let Some(key) = &event.envelope.correlation_key else {
            return Ok(Vec::new());
        };
        sqlx::query(
            "SELECT tenant_id, namespace_id, run_id, node, event_type, correlation_key, event_since \
             FROM steps \
             WHERE tenant_id = $1 AND namespace_id = $2 AND status = 'WAITING' \
               AND waiting_kind = 'EVENT' AND event_type = $3 AND correlation_key = $4 \
               AND event_since <= $5 \
               AND ($7::uuid IS NULL OR (run_id, node) > ($7::uuid, $8::text)) \
             ORDER BY run_id, node LIMIT $6",
        )
        .bind(event.tenant_id.as_uuid())
        .bind(event.namespace_id.as_uuid())
        .bind(event.envelope.event_type.as_str())
        .bind(key.as_str())
        .bind(event.received_at)
        .bind(i64::from(limit))
        .bind(after.map(|(run, _)| run.as_uuid()))
        .bind(after.map(|(_, node)| node.as_str()))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(waiting_from_row)
        .collect()
    }

    async fn unscanned_waits(&self, limit: u32) -> Result<Vec<WaitingStep>, StoreError> {
        sqlx::query(
            "SELECT tenant_id, namespace_id, run_id, node, event_type, correlation_key, event_since \
             FROM steps \
             WHERE status = 'WAITING' AND waiting_kind = 'EVENT' AND event_scanned_at IS NULL \
               AND event_since IS NOT NULL \
             ORDER BY updated_at LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(waiting_from_row)
        .collect()
    }

    async fn undelivered_event_for(
        &self,
        step: &WaitingStep,
    ) -> Result<Option<ClaimedEvent>, StoreError> {
        let consumer = format!("wait:{}/{}", step.run_id, step.node);
        sqlx::query(select_events!(
            "WHERE e.tenant_id = $1 AND e.namespace_id = $2 AND e.event_type = $3 \
               AND e.correlation_key = $4 AND e.received_at >= $5 \
               AND NOT EXISTS (SELECT 1 FROM event_deliveries d \
                               WHERE d.tenant_id = e.tenant_id AND d.namespace_id = e.namespace_id \
                                 AND d.source = e.source AND d.event_id = e.event_id \
                                 AND d.consumer = $6) \
             ORDER BY e.received_at LIMIT 1"
        ))
        .bind(step.tenant_id.as_uuid())
        .bind(step.namespace_id.as_uuid())
        .bind(step.event_type.as_str())
        .bind(step.correlation_key.as_str())
        .bind(step.since)
        .bind(consumer)
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        .map(|row| claimed_from_row(&row))
        .transpose()
    }

    async fn mark_wait_scanned(&self, step: &WaitingStep) -> Result<(), StoreError> {
        sqlx::query(
            "UPDATE steps SET event_scanned_at = clock_timestamp() \
             WHERE tenant_id = $1 AND run_id = $2 AND node = $3 AND status = 'WAITING' \
               AND waiting_kind = 'EVENT' AND event_scanned_at IS NULL",
        )
        .bind(step.tenant_id.as_uuid())
        .bind(step.run_id.as_uuid())
        .bind(step.node.as_str())
        .execute(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(())
    }

    async fn purge_events(&self, limit: u32) -> Result<u64, StoreError> {
        // An event still waiting to be dispatched outlives its deduplication window: dropping it
        // would lose it.
        Ok(sqlx::query(
            "DELETE FROM inbox_events WHERE ctid IN ( \
                 SELECT ctid FROM inbox_events \
                 WHERE expires_at <= clock_timestamp() AND status <> 'PENDING' \
                 ORDER BY expires_at LIMIT $1)",
        )
        .bind(i64::from(limit))
        .execute(&self.pool)
        .await
        .map_err(store_error)?
        .rows_affected())
    }

    async fn due_cron_triggers(&self, limit: u32) -> Result<Vec<StoredTrigger>, StoreError> {
        sqlx::query(select_triggers!(
            "WHERE kind = 'cron' \
               AND (next_fire_at IS NULL \
                    OR (next_fire_at <= clock_timestamp() \
                        AND (retry_at IS NULL OR retry_at <= clock_timestamp()))) \
             ORDER BY next_fire_at NULLS FIRST LIMIT $1"
        ))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(trigger_from_row)
        .collect()
    }

    async fn advance_cron_trigger(
        &self,
        id: Uuid,
        expected: Option<OffsetDateTime>,
        next: OffsetDateTime,
    ) -> Result<bool, StoreError> {
        Ok(sqlx::query(
            "UPDATE workflow_triggers \
             SET next_fire_at = $3, retry_at = NULL, last_fire_at = COALESCE($2, last_fire_at) \
             WHERE id = $1 AND next_fire_at IS NOT DISTINCT FROM $2",
        )
        .bind(id)
        .bind(expected)
        .bind(next)
        .execute(&self.pool)
        .await
        .map_err(store_error)?
        .rows_affected()
            > 0)
    }

    async fn delay_cron_trigger(
        &self,
        id: Uuid,
        scheduled: OffsetDateTime,
        retry_at: OffsetDateTime,
    ) -> Result<bool, StoreError> {
        Ok(sqlx::query(
            "UPDATE workflow_triggers SET retry_at = $3 WHERE id = $1 AND next_fire_at = $2",
        )
        .bind(id)
        .bind(scheduled)
        .bind(retry_at)
        .execute(&self.pool)
        .await
        .map_err(store_error)?
        .rows_affected()
            > 0)
    }

    async fn pending_events(&self) -> Result<u64, StoreError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (SELECT 1 FROM inbox_events WHERE status = 'PENDING' \
                                   LIMIT 100000) AS p",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
