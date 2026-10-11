//! Subscriptions to outbound events and their deliveries in PostgreSQL (`FR-004-006`).
//!
//! A delivery row is written in the transaction that records the event it carries (see
//! [`fan_out`]), so an event is owed to every subscription that wanted it or was never committed.
//! The worker claims due deliveries with a short lease and records each attempt, the new state of the
//! delivery and the health of the subscription in one transaction. The rate of a subscription is
//! enforced here, across replicas: a claim moves the subscription's `next_slot_at`, and a
//! subscription whose slot is in the future is not offered to anybody.

use crate::pool::BeginSafely;
use std::time::Duration;

use sagat_application::error::StoreError;
use sagat_application::outbound::{
    AttemptRecord, DeliveryRecord, DeliveryStore, DueDelivery, Health, Next, OutboundStore,
    SubscriptionRecord,
};
use sagat_application::secrets::StoredSecret;
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::identity::{NamespaceId, Slug, TenantId};
use sagat_domain::outbound::{
    AttemptOutcome, DISABLE_AFTER_DEAD, DeliveryId, DeliveryLimits, DeliveryStatus,
    MAX_PENDING_PER_SUBSCRIPTION, OutboundKind, SubscriptionId, SubscriptionStatus, TargetUrl,
};
use sagat_domain::secrets::SecretName;
use sqlx::postgres::PgRow;
use sqlx::{PgConnection, Row};
use uuid::Uuid;

use crate::execution::PgRunStore;
use crate::identity::{col, corrupt, insert_audit, store_error};
use crate::quotas::{ensure_room_for_subscription, lock_subscriptions};

/// Most deliveries one subscription is offered per claim, however fast its rate: a subscription
/// with a long queue must not take the whole claim from the others.
const MAX_PER_SUBSCRIPTION: i64 = 25;

macro_rules! select_subscriptions {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, name, url, secret_name, kinds, rate_per_second, ",
            "timeout_ms, status, consecutive_dead, last_success_at, last_failure_at, ",
            "disabled_reason, created_by, created_at, updated_at FROM webhook_subscriptions ",
            $tail
        )
    };
}

macro_rules! select_deliveries {
    ($tail:literal) => {
        concat!(
            "SELECT id, tenant_id, namespace_id, subscription_id, event_id, kind, payload, status, ",
            "attempts, attempts_before_redrive, next_attempt_at, last_outcome, last_http_status, ",
            "last_error, created_at, delivered_at FROM webhook_deliveries ",
            $tail
        )
    };
}

fn subscription_from_row(row: &PgRow) -> Result<SubscriptionRecord, StoreError> {
    let name: String = col(row, "name")?;
    let url: String = col(row, "url")?;
    let secret: String = col(row, "secret_name")?;
    let kinds: Vec<String> = col(row, "kinds")?;
    let rate: i32 = col(row, "rate_per_second")?;
    let timeout: i32 = col(row, "timeout_ms")?;
    let status: String = col(row, "status")?;
    let dead: i32 = col(row, "consecutive_dead")?;
    Ok(SubscriptionRecord {
        id: SubscriptionId::new(col::<Uuid>(row, "id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        name: Slug::parse(&name).map_err(|_| corrupt("subscription name"))?,
        url: TargetUrl::parse(&url).map_err(|_| corrupt("subscription url"))?,
        secret_name: SecretName::parse(&secret).map_err(|_| corrupt("subscription secret"))?,
        kinds: kinds
            .iter()
            .map(|k| {
                k.parse::<OutboundKind>()
                    .map_err(|()| corrupt("subscription kind"))
            })
            .collect::<Result<_, _>>()?,
        limits: DeliveryLimits::new(
            u32::try_from(rate).map_err(|_| corrupt("subscription rate"))?,
            Duration::from_millis(
                u64::try_from(timeout).map_err(|_| corrupt("subscription timeout"))?,
            ),
        )
        .map_err(|_| corrupt("subscription limits"))?,
        status: status
            .parse::<SubscriptionStatus>()
            .map_err(|()| corrupt("subscription status"))?,
        health: Health {
            consecutive_dead: u32::try_from(dead).map_err(|_| corrupt("subscription failures"))?,
            last_success_at: col(row, "last_success_at")?,
            last_failure_at: col(row, "last_failure_at")?,
            disabled_reason: col(row, "disabled_reason")?,
        },
        created_by: col(row, "created_by")?,
        created_at: col(row, "created_at")?,
        updated_at: col(row, "updated_at")?,
    })
}

fn delivery_from_row(row: &PgRow) -> Result<DeliveryRecord, StoreError> {
    let kind: String = col(row, "kind")?;
    let status: String = col(row, "status")?;
    let attempts: i32 = col(row, "attempts")?;
    let before_redrive: i32 = col(row, "attempts_before_redrive")?;
    let outcome: Option<String> = col(row, "last_outcome")?;
    let http: Option<i32> = col(row, "last_http_status")?;
    Ok(DeliveryRecord {
        id: DeliveryId::new(col::<Uuid>(row, "id")?),
        tenant_id: TenantId::new(col::<Uuid>(row, "tenant_id")?),
        namespace_id: NamespaceId::new(col::<Uuid>(row, "namespace_id")?),
        subscription_id: SubscriptionId::new(col::<Uuid>(row, "subscription_id")?),
        event_id: col(row, "event_id")?,
        kind: kind
            .parse::<OutboundKind>()
            .map_err(|()| corrupt("delivery kind"))?,
        payload: col(row, "payload")?,
        status: status
            .parse::<DeliveryStatus>()
            .map_err(|()| corrupt("delivery status"))?,
        attempts: u32::try_from(attempts).map_err(|_| corrupt("delivery attempts"))?,
        attempts_before_redrive: u32::try_from(before_redrive)
            .map_err(|_| corrupt("delivery attempts"))?,
        next_attempt_at: col(row, "next_attempt_at")?,
        last_outcome: outcome
            .map(|o| {
                o.parse::<AttemptOutcome>()
                    .map_err(|()| corrupt("delivery outcome"))
            })
            .transpose()?,
        last_http_status: http
            .map(|s| u16::try_from(s).map_err(|_| corrupt("delivery http status")))
            .transpose()?,
        last_error: col(row, "last_error")?,
        created_at: col(row, "created_at")?,
        delivered_at: col(row, "delivered_at")?,
    })
}

fn kind_texts(kinds: &[OutboundKind]) -> Vec<&'static str> {
    kinds.iter().map(|k| k.as_str()).collect()
}

fn millis(duration: Duration) -> i32 {
    i32::try_from(duration.as_millis()).unwrap_or(i32::MAX)
}

/// Queues `payload` for every active subscription of the namespace that wants `kind`, in the
/// caller's transaction. One row per subscription, keyed by the event, so queueing the same event
/// twice queues it once. The ids are time-ordered, so a listing by id is a listing by age.
///
/// A subscription that already has [`MAX_PENDING_PER_SUBSCRIPTION`] deliveries waiting is not given
/// another: it is switched off, with its reason and an audit record, in the same transaction.
pub(crate) async fn fan_out(
    conn: &mut PgConnection,
    tenant: TenantId,
    namespace: NamespaceId,
    kind: OutboundKind,
    event_id: &str,
    payload: &[u8],
    now: time::OffsetDateTime,
) -> Result<u64, StoreError> {
    // No lock on the subscriptions: counting them is a look, and two commits that look together may
    // both add one past the limit, which is as good as the limit.
    let candidates = sqlx::query(
        "SELECT s.id, \
                (SELECT count(*) FROM (SELECT 1 FROM webhook_deliveries d \
                                       WHERE d.subscription_id = s.id AND d.status = 'PENDING' \
                                       LIMIT $4) AS p) AS pending \
         FROM webhook_subscriptions s \
         WHERE s.tenant_id = $1 AND s.namespace_id = $2 AND s.status = 'ACTIVE' \
           AND $3 = ANY(s.kinds)",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(kind.as_str())
    .bind(i64::try_from(MAX_PENDING_PER_SUBSCRIPTION).unwrap_or(i64::MAX))
    .fetch_all(&mut *conn)
    .await
    .map_err(store_error)?;
    let (mut wanted, mut full) = (Vec::new(), Vec::new());
    for row in &candidates {
        let id: Uuid = col(row, "id")?;
        let pending: i64 = col(row, "pending")?;
        if u64::try_from(pending).unwrap_or(u64::MAX) >= MAX_PENDING_PER_SUBSCRIPTION {
            full.push(id);
        } else {
            wanted.push(id);
        }
    }
    if !full.is_empty() {
        let switched = sqlx::query_scalar::<_, Uuid>(
            "UPDATE webhook_subscriptions SET status = 'DISABLED', \
                 disabled_reason = 'too many deliveries were waiting' \
             WHERE id = ANY($1) AND status = 'ACTIVE' RETURNING id",
        )
        .bind(&full)
        .fetch_all(&mut *conn)
        .await
        .map_err(store_error)?;
        for id in switched {
            tracing::warn!(subscription = %id, "a subscription was switched off: too many deliveries were waiting");
            metrics::counter!("webhook_subscriptions_switched_off_total").increment(1);
            let audit = AuditRecord::engine(
                tenant,
                AuditAction::WebhookSubscriptionDisabled,
                ResourceType::WebhookSubscription,
                id.to_string(),
                "too many deliveries were waiting",
                None,
                now,
            );
            insert_audit(&mut *conn, &audit).await?;
        }
    }
    if wanted.is_empty() {
        return Ok(0);
    }
    let ids: Vec<Uuid> = wanted.iter().map(|_| Uuid::now_v7()).collect();
    Ok(sqlx::query(
        "INSERT INTO webhook_deliveries \
         (id, tenant_id, namespace_id, subscription_id, event_id, kind, payload, next_attempt_at) \
         SELECT t.id, $1, $2, t.subscription_id, $4, $3, $5, clock_timestamp() \
         FROM unnest($6::uuid[], $7::uuid[]) AS t(id, subscription_id) \
         ON CONFLICT (subscription_id, event_id) DO NOTHING",
    )
    .bind(tenant.as_uuid())
    .bind(namespace.as_uuid())
    .bind(kind.as_str())
    .bind(event_id)
    .bind(payload)
    .bind(&ids)
    .bind(&wanted)
    .execute(conn)
    .await
    .map_err(store_error)?
    .rows_affected())
}

impl OutboundStore for PgRunStore {
    async fn create_subscription(
        &self,
        subscription: &SubscriptionRecord,
        limit: u64,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        lock_subscriptions(&mut tx, subscription.tenant_id, subscription.namespace_id).await?;
        ensure_room_for_subscription(
            &mut tx,
            subscription.tenant_id,
            subscription.namespace_id,
            limit,
        )
        .await?;
        sqlx::query(
            "INSERT INTO webhook_subscriptions \
             (id, tenant_id, namespace_id, name, url, secret_name, kinds, rate_per_second, \
              timeout_ms, status, created_by, created_at, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $12)",
        )
        .bind(subscription.id.as_uuid())
        .bind(subscription.tenant_id.as_uuid())
        .bind(subscription.namespace_id.as_uuid())
        .bind(subscription.name.as_str())
        .bind(subscription.url.as_str())
        .bind(subscription.secret_name.as_str())
        .bind(kind_texts(&subscription.kinds))
        .bind(i32::try_from(subscription.limits.rate_per_second()).unwrap_or(i32::MAX))
        .bind(millis(subscription.limits.timeout()))
        .bind(subscription.status.as_str())
        .bind(&subscription.created_by)
        .bind(subscription.created_at)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)
    }

    async fn update_subscription(
        &self,
        subscription: &SubscriptionRecord,
        audit: &AuditRecord,
    ) -> Result<Option<u64>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // The status it had is what decides whether this is a subscription being turned back on.
        let before = sqlx::query_scalar::<_, String>(
            "SELECT status FROM webhook_subscriptions WHERE tenant_id = $1 AND id = $2 FOR UPDATE",
        )
        .bind(subscription.tenant_id.as_uuid())
        .bind(subscription.id.as_uuid())
        .fetch_optional(&mut *tx)
        .await
        .map_err(store_error)?;
        let Some(before) = before else {
            return Ok(None);
        };
        // Only a subscription the platform switched off is "turned back on": that is the
        // administrator saying the receiver is back. A resume after a pause is not that.
        let turned_on = before == "DISABLED" && subscription.status == SubscriptionStatus::Active;
        sqlx::query(
            "UPDATE webhook_subscriptions SET url = $3, secret_name = $4, kinds = $5, \
                 rate_per_second = $6, timeout_ms = $7, status = $8, updated_at = $9, \
                 consecutive_dead = CASE WHEN $10 THEN 0 ELSE consecutive_dead END, \
                 disabled_reason = CASE WHEN $10 THEN NULL ELSE disabled_reason END, \
                 next_slot_at = CASE WHEN $10 THEN '-infinity' ELSE next_slot_at END \
             WHERE tenant_id = $1 AND id = $2",
        )
        .bind(subscription.tenant_id.as_uuid())
        .bind(subscription.id.as_uuid())
        .bind(subscription.url.as_str())
        .bind(subscription.secret_name.as_str())
        .bind(kind_texts(&subscription.kinds))
        .bind(i32::try_from(subscription.limits.rate_per_second()).unwrap_or(i32::MAX))
        .bind(millis(subscription.limits.timeout()))
        .bind(subscription.status.as_str())
        .bind(subscription.updated_at)
        .bind(turned_on)
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;
        // Turning back on a subscription the platform switched off is the administrator saying the
        // receiver is back: what was set aside as dead is delivered again, up to a thousand (the
        // rest wait for an explicit redrive, and the response says how many came back). Changing
        // the settings of one that is running, or resuming one that was paused, touches nothing
        // that was set aside.
        let redriven = if turned_on {
            requeue_dead(&mut tx, subscription.tenant_id, subscription.id, None, 1000).await?
        } else {
            0
        };
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(Some(redriven))
    }

    async fn delete_subscription(
        &self,
        tenant: TenantId,
        id: SubscriptionId,
        audit: &AuditRecord,
    ) -> Result<bool, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let removed =
            sqlx::query("DELETE FROM webhook_subscriptions WHERE tenant_id = $1 AND id = $2")
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

    async fn subscription(
        &self,
        tenant: TenantId,
        id: SubscriptionId,
    ) -> Result<Option<SubscriptionRecord>, StoreError> {
        sqlx::query(select_subscriptions!("WHERE tenant_id = $1 AND id = $2"))
            .bind(tenant.as_uuid())
            .bind(id.as_uuid())
            .fetch_optional(&self.pool)
            .await
            .map_err(store_error)?
            .map(|row| subscription_from_row(&row))
            .transpose()
    }

    async fn subscriptions(
        &self,
        tenant: TenantId,
        namespace: NamespaceId,
    ) -> Result<Vec<SubscriptionRecord>, StoreError> {
        sqlx::query(select_subscriptions!(
            "WHERE tenant_id = $1 AND namespace_id = $2 ORDER BY name"
        ))
        .bind(tenant.as_uuid())
        .bind(namespace.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(subscription_from_row)
        .collect()
    }

    async fn deliveries(
        &self,
        tenant: TenantId,
        subscription: SubscriptionId,
        status: Option<DeliveryStatus>,
        before: Option<DeliveryId>,
        limit: u32,
    ) -> Result<Vec<DeliveryRecord>, StoreError> {
        sqlx::query(select_deliveries!(
            "WHERE tenant_id = $1 AND subscription_id = $2 \
               AND ($3::text IS NULL OR status = $3) \
               AND ($4::uuid IS NULL OR id < $4) \
             ORDER BY id DESC LIMIT $5"
        ))
        .bind(tenant.as_uuid())
        .bind(subscription.as_uuid())
        .bind(status.map(DeliveryStatus::as_str))
        .bind(before.map(DeliveryId::as_uuid))
        .bind(i64::from(limit))
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(delivery_from_row)
        .collect()
    }

    async fn delivery(
        &self,
        tenant: TenantId,
        subscription: SubscriptionId,
        id: DeliveryId,
    ) -> Result<Option<(DeliveryRecord, Vec<AttemptRecord>)>, StoreError> {
        let Some(row) = sqlx::query(select_deliveries!(
            "WHERE tenant_id = $1 AND subscription_id = $2 AND id = $3"
        ))
        .bind(tenant.as_uuid())
        .bind(subscription.as_uuid())
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(store_error)?
        else {
            return Ok(None);
        };
        let attempts = sqlx::query(
            "SELECT attempt, started_at, duration_ms, outcome, http_status, error \
             FROM webhook_attempts WHERE tenant_id = $1 AND delivery_id = $2 ORDER BY attempt",
        )
        .bind(tenant.as_uuid())
        .bind(id.as_uuid())
        .fetch_all(&self.pool)
        .await
        .map_err(store_error)?
        .iter()
        .map(|a| {
            let attempt: i32 = col(a, "attempt")?;
            let duration: i32 = col(a, "duration_ms")?;
            let outcome: String = col(a, "outcome")?;
            let http: Option<i32> = col(a, "http_status")?;
            Ok(AttemptRecord {
                attempt: u32::try_from(attempt).map_err(|_| corrupt("attempt number"))?,
                started_at: col(a, "started_at")?,
                duration_ms: u32::try_from(duration).map_err(|_| corrupt("attempt duration"))?,
                outcome: outcome
                    .parse::<AttemptOutcome>()
                    .map_err(|()| corrupt("attempt outcome"))?,
                http_status: http
                    .map(|s| u16::try_from(s).map_err(|_| corrupt("attempt http status")))
                    .transpose()?,
                error: col(a, "error")?,
            })
        })
        .collect::<Result<Vec<_>, StoreError>>()?;
        Ok(Some((delivery_from_row(&row)?, attempts)))
    }

    async fn redrive(
        &self,
        tenant: TenantId,
        subscription: SubscriptionId,
        delivery: Option<DeliveryId>,
        limit: u32,
        audit: &AuditRecord,
    ) -> Result<u64, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let count = requeue_dead(&mut tx, tenant, subscription, delivery, limit).await?;
        insert_audit(&mut tx, audit).await?;
        tx.commit().await.map_err(store_error)?;
        Ok(count)
    }
}

/// Puts dead deliveries back: one, or the oldest `limit`. The attempts made stay in the record; the
/// delivery gets a full set again.
async fn requeue_dead(
    conn: &mut PgConnection,
    tenant: TenantId,
    subscription: SubscriptionId,
    delivery: Option<DeliveryId>,
    limit: u32,
) -> Result<u64, StoreError> {
    Ok(sqlx::query(
        "UPDATE webhook_deliveries SET status = 'PENDING', attempts_before_redrive = attempts, \
             next_attempt_at = clock_timestamp(), last_error = NULL \
         WHERE id IN ( \
             SELECT id FROM webhook_deliveries \
             WHERE tenant_id = $1 AND subscription_id = $2 AND status = 'DEAD' \
               AND ($3::uuid IS NULL OR id = $3) \
             ORDER BY created_at LIMIT $4 FOR UPDATE SKIP LOCKED)",
    )
    .bind(tenant.as_uuid())
    .bind(subscription.as_uuid())
    .bind(delivery.map(DeliveryId::as_uuid))
    .bind(i64::from(limit))
    .execute(conn)
    .await
    .map_err(store_error)?
    .rows_affected())
}

impl DeliveryStore for PgRunStore {
    async fn claim_deliveries(
        &self,
        limit: u32,
        lease: Duration,
    ) -> Result<Vec<DueDelivery>, StoreError> {
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        // Subscriptions whose slot has come and that have something due; taking the slot is what
        // limits the rate, across replicas, because nobody else is offered the subscription.
        let slots = sqlx::query(
            "SELECT id, tenant_id, rate_per_second FROM webhook_subscriptions s \
             WHERE s.status = 'ACTIVE' AND s.next_slot_at <= clock_timestamp() \
               AND EXISTS (SELECT 1 FROM webhook_deliveries d \
                           WHERE d.subscription_id = s.id AND d.status = 'PENDING' \
                             AND d.next_attempt_at <= clock_timestamp()) \
             ORDER BY s.next_slot_at LIMIT $1 FOR UPDATE OF s SKIP LOCKED",
        )
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await
        .map_err(store_error)?;

        let mut due = Vec::new();
        for slot in &slots {
            let id: Uuid = col(slot, "id")?;
            let rate: i32 = col(slot, "rate_per_second")?;
            let per_slot = (i64::from(rate) / 4).clamp(1, MAX_PER_SUBSCRIPTION);
            let rows = sqlx::query(
                "UPDATE webhook_deliveries d \
                 SET next_attempt_at = clock_timestamp() + make_interval(secs => $3) \
                 WHERE d.id IN ( \
                     SELECT id FROM webhook_deliveries \
                     WHERE subscription_id = $1 AND status = 'PENDING' \
                       AND next_attempt_at <= clock_timestamp() \
                     ORDER BY next_attempt_at LIMIT $2 FOR UPDATE SKIP LOCKED) \
                 RETURNING d.id, d.tenant_id, d.namespace_id, d.subscription_id, d.event_id, \
                           d.kind, d.payload, d.status, d.attempts, d.attempts_before_redrive, \
                           d.next_attempt_at, d.last_outcome, d.last_http_status, d.last_error, \
                           d.created_at, d.delivered_at",
            )
            .bind(id)
            .bind(per_slot)
            .bind(lease.as_secs_f64())
            .fetch_all(&mut *tx)
            .await
            .map_err(store_error)?;
            let taken = i64::try_from(rows.len()).unwrap_or(i64::MAX);
            // The slot moves by what was taken, so a subscription with little to send is offered
            // again soon, and one that was given a full share waits its turn.
            sqlx::query(
                "UPDATE webhook_subscriptions \
                 SET next_slot_at = clock_timestamp() + make_interval(secs => $2::float8 / $3) \
                 WHERE id = $1",
            )
            .bind(id)
            .bind(taken as f64)
            .bind(f64::from(rate))
            .execute(&mut *tx)
            .await
            .map_err(store_error)?;
            if rows.is_empty() {
                continue;
            }
            let sub = sqlx::query(select_subscriptions!("WHERE id = $1"))
                .bind(id)
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
            let sub = subscription_from_row(&sub)?;
            for row in &rows {
                due.push(DueDelivery {
                    delivery: delivery_from_row(row)?,
                    url: sub.url.clone(),
                    secret_name: sub.secret_name.clone(),
                    limits: sub.limits,
                });
            }
        }
        tx.commit().await.map_err(store_error)?;
        due.sort_by_key(|d| d.delivery.created_at);
        Ok(due)
    }

    async fn record_attempt(
        &self,
        due: &DueDelivery,
        attempt: &AttemptRecord,
        next: &Next,
        switched_off: Option<&sagat_domain::audit::AuditRecord>,
    ) -> Result<bool, StoreError> {
        let d = &due.delivery;
        let mut tx = self.pool.begin_safely().await.map_err(store_error)?;
        let (status, delay) = match next {
            Next::Delivered => ("DELIVERED", 0.0),
            Next::Retry { delay } => ("PENDING", delay.as_secs_f64()),
            Next::Dead => ("DEAD", 0.0),
        };
        // Fenced by the attempt number: a worker whose lease ran out while it was sending, and whose
        // delivery another worker has since taken further, finds the delivery no longer where it
        // left it and records nothing (neither the attempt, nor a state, nor the subscription's
        // health). What the other worker recorded is the delivery's story.
        let moved = sqlx::query(
            "UPDATE webhook_deliveries SET attempts = $2, status = $3, \
                 next_attempt_at = clock_timestamp() + make_interval(secs => $4), \
                 last_outcome = $5, last_http_status = $6, last_error = $7, \
                 delivered_at = CASE WHEN $3 = 'DELIVERED' THEN clock_timestamp() END \
             WHERE id = $1 AND status = 'PENDING' AND attempts = $2 - 1",
        )
        .bind(d.id.as_uuid())
        .bind(i32::try_from(attempt.attempt).unwrap_or(i32::MAX))
        .bind(status)
        .bind(delay)
        .bind(attempt.outcome.as_str())
        .bind(attempt.http_status.map(i32::from))
        .bind(attempt.error.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?
        .rows_affected();
        if moved == 0 {
            tx.rollback().await.map_err(store_error)?;
            metrics::counter!("webhook_attempts_superseded_total").increment(1);
            return Ok(false);
        }
        sqlx::query(
            "INSERT INTO webhook_attempts \
             (delivery_id, attempt, tenant_id, started_at, duration_ms, outcome, http_status, error) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT DO NOTHING",
        )
        .bind(d.id.as_uuid())
        .bind(i32::try_from(attempt.attempt).unwrap_or(i32::MAX))
        .bind(d.tenant_id.as_uuid())
        .bind(attempt.started_at)
        .bind(i32::try_from(attempt.duration_ms).unwrap_or(i32::MAX))
        .bind(attempt.outcome.as_str())
        .bind(attempt.http_status.map(i32::from))
        .bind(attempt.error.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(store_error)?;

        // The health of the subscription follows what ended: a delivery that got through clears the
        // run of failures, one that was set aside adds to it, and a run that reaches the limit
        // switches the subscription off so it is not tried on every new event for ever.
        let off = match next {
            Next::Delivered => {
                sqlx::query(
                    "UPDATE webhook_subscriptions SET consecutive_dead = 0, \
                         last_success_at = clock_timestamp() WHERE id = $1",
                )
                .bind(d.subscription_id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                false
            }
            Next::Retry { .. } => {
                sqlx::query(
                    "UPDATE webhook_subscriptions SET last_failure_at = clock_timestamp() \
                     WHERE id = $1",
                )
                .bind(d.subscription_id.as_uuid())
                .execute(&mut *tx)
                .await
                .map_err(store_error)?;
                false
            }
            Next::Dead => {
                // `switched` is true only for the delivery that crossed the limit: a subscription
                // that is already off stays off without being switched off again.
                let row = sqlx::query(
                    "WITH before AS ( \
                         SELECT id, status, consecutive_dead FROM webhook_subscriptions \
                         WHERE id = $1 FOR UPDATE) \
                     UPDATE webhook_subscriptions s SET consecutive_dead = s.consecutive_dead + 1, \
                         last_failure_at = clock_timestamp(), \
                         status = CASE WHEN before.status = 'ACTIVE' \
                                        AND before.consecutive_dead + 1 >= $2 \
                                       THEN 'DISABLED' ELSE s.status END, \
                         disabled_reason = CASE WHEN before.status = 'ACTIVE' \
                                                 AND before.consecutive_dead + 1 >= $2 \
                                                THEN 'too many deliveries in a row were set aside' \
                                                ELSE s.disabled_reason END \
                     FROM before WHERE s.id = before.id \
                     RETURNING (before.status = 'ACTIVE' \
                                AND before.consecutive_dead + 1 >= $2) AS switched",
                )
                .bind(d.subscription_id.as_uuid())
                .bind(i32::try_from(DISABLE_AFTER_DEAD).unwrap_or(i32::MAX))
                .fetch_one(&mut *tx)
                .await
                .map_err(store_error)?;
                row.try_get::<bool, _>("switched").map_err(store_error)?
            }
        };
        if off && let Some(audit) = switched_off {
            insert_audit(&mut tx, audit).await?;
        }
        tx.commit().await.map_err(store_error)?;
        Ok(off)
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

    async fn purge_deliveries(&self, retention: Duration, limit: u32) -> Result<u64, StoreError> {
        Ok(sqlx::query(
            "DELETE FROM webhook_deliveries WHERE ctid IN ( \
                 SELECT ctid FROM webhook_deliveries \
                 WHERE status <> 'PENDING' \
                   AND created_at < clock_timestamp() - make_interval(secs => $1) \
                 ORDER BY created_at LIMIT $2)",
        )
        .bind(retention.as_secs_f64())
        .bind(i64::from(limit))
        .execute(&self.pool)
        .await
        .map_err(store_error)?
        .rows_affected())
    }

    async fn pending_deliveries(&self) -> Result<u64, StoreError> {
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM (SELECT 1 FROM webhook_deliveries WHERE status = 'PENDING' \
                                   LIMIT 100000) AS p",
        )
        .fetch_one(&self.pool)
        .await
        .map_err(store_error)?;
        Ok(u64::try_from(count).unwrap_or(0))
    }
}
