//! Durable event admission policies: replay costs nothing and refused work never enters the inbox.
use std::time::Duration;

use sagat_application::error::AppError;
use sagat_application::events::PublishRequest;
use sagat_domain::events::EventId;
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::EventType;

use crate::support::engine::{World, ctx, world};

fn request(w: &World, id: &str, ttl: u64) -> PublishRequest {
    PublishRequest {
        namespace: w.namespace.slug().as_str().to_owned(),
        event_id: Some(EventId::parse(id).expect("event id")),
        event_type: EventType::parse("order.created").expect("type"),
        subject: None,
        correlation_key: None,
        occurred_at: None,
        data: b"{}".to_vec(),
        dedupe_ttl: Some(Duration::from_secs(ttl)),
    }
}

#[tokio::test]
async fn disabling_event_admission_preserves_duplicate_acknowledgement() {
    let w = world().await;
    w.events
        .publish(&w.admin, request(&w, "already-accepted", 600), &ctx())
        .await
        .expect("first admission");
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::EventsPerSecond,
            0,
            &ctx(),
        )
        .await
        .expect("disable new events");
    let replay = w
        .events
        .publish(&w.admin, request(&w, "already-accepted", 600), &ctx())
        .await
        .expect("a committed event remains acknowledged");
    assert!(replay.duplicate);
    assert!(matches!(
        w.events
            .publish(&w.admin, request(&w, "new-event", 600), &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM inbox_events")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn retention_lowering_refuses_new_choices_without_rewriting_prior_expiry() {
    let w = world().await;
    w.events
        .publish(&w.admin, request(&w, "retained", 3600), &ctx())
        .await
        .expect("first admission");
    let before: time::OffsetDateTime = sqlx::query_scalar("SELECT expires_at FROM inbox_events")
        .fetch_one(&w.db.pool)
        .await
        .expect("expiry");
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::EventRetentionSeconds,
            600,
            &ctx(),
        )
        .await
        .expect("lower retention ceiling");
    assert!(matches!(
        w.events
            .publish(&w.admin, request(&w, "too-long", 601), &ctx())
            .await,
        Err(AppError::QuotaExceeded(_))
    ));
    w.events
        .publish(&w.admin, request(&w, "within-policy", 600), &ctx())
        .await
        .expect("new bounded retention");
    assert!(
        w.events
            .publish(&w.admin, request(&w, "retained", 3600), &ctx())
            .await
            .expect("original replay")
            .duplicate
    );
    let after: time::OffsetDateTime =
        sqlx::query_scalar("SELECT expires_at FROM inbox_events WHERE event_id='retained'")
            .fetch_one(&w.db.pool)
            .await
            .expect("expiry");
    assert_eq!(before, after);
}

#[tokio::test]
async fn a_failed_audit_rolls_back_the_event_and_its_durable_rate_debit() {
    let w = world().await;
    w.events
        .publish(&w.admin, request(&w, "before-failure", 600), &ctx())
        .await
        .expect("prime bucket");
    let before: (i64, time::OffsetDateTime) = sqlx::query_as(
        "SELECT microtokens, updated_at FROM quota_rate_buckets WHERE resource='events_per_second'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("bucket");
    sqlx::query("ALTER TABLE audit_records ADD CONSTRAINT test_reject_event_audit CHECK (action <> 'event.published') NOT VALID")
        .execute(&w.db.pool).await.expect("inject audit failure");
    assert!(
        w.events
            .publish(&w.admin, request(&w, "failed-audit", 600), &ctx())
            .await
            .is_err()
    );
    let after: (i64, time::OffsetDateTime) = sqlx::query_as(
        "SELECT microtokens, updated_at FROM quota_rate_buckets WHERE resource='events_per_second'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("bucket");
    assert_eq!(
        before, after,
        "a refused transaction consumes no token or refill time"
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM inbox_events WHERE event_id='failed-audit'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(count, 0);
    sqlx::query("ALTER TABLE audit_records DROP CONSTRAINT test_reject_event_audit")
        .execute(&w.db.pool)
        .await
        .expect("restore audit");
    assert!(
        !w.events
            .publish(&w.admin, request(&w, "failed-audit", 600), &ctx())
            .await
            .expect("retry admits once")
            .duplicate
    );
}

fn replica(w: &World) -> crate::support::engine::Events {
    sagat_application::events::EventService::new(
        sagat_infra_postgres::PgRunStore::new(w.db.pool.clone(), w.cache.clone()),
        sagat_application::clock::SystemClock,
        w.keys.cipher(),
        std::sync::Arc::new(sagat_infra_ir::JsonEvaluator),
    )
}

async fn leave_one_token(w: &World) {
    w.events
        .publish(&w.admin, request(w, "prime", 600), &ctx())
        .await
        .expect("prime");
    // Freeze refill even if the test machine is slow, simulating a backward wall-clock step.
    sqlx::query("UPDATE quota_rate_buckets SET microtokens=1000000, updated_at=clock_timestamp()+interval '1 day' WHERE resource='events_per_second'")
        .execute(&w.db.pool).await.expect("last token");
}

#[tokio::test]
async fn independent_stores_race_for_exactly_one_remaining_event_token() {
    let w = world().await;
    leave_one_token(&w).await;
    let other = replica(&w);
    let context = ctx();
    let (a, b) = tokio::join!(
        w.events
            .publish(&w.admin, request(&w, "racer-a", 600), &context),
        other.publish(&w.admin, request(&w, "racer-b", 600), &context),
    );
    assert_eq!(usize::from(a.is_ok()) + usize::from(b.is_ok()), 1);
    let refused = if a.is_err() { a } else { b };
    assert!(matches!(refused, Err(AppError::Busy { .. })));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM inbox_events")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(count, 2);
}

#[tokio::test]
async fn concurrent_identical_events_consume_the_last_token_only_once() {
    let w = world().await;
    leave_one_token(&w).await;
    let other = replica(&w);
    let context = ctx();
    let (a, b) = tokio::join!(
        w.events
            .publish(&w.admin, request(&w, "same-event", 600), &context),
        other.publish(&w.admin, request(&w, "same-event", 600), &context),
    );
    let a = a.expect("first delivery");
    let b = b.expect("second delivery");
    assert_ne!(a.duplicate, b.duplicate);
    let tokens: i64 = sqlx::query_scalar(
        "SELECT microtokens FROM quota_rate_buckets WHERE resource='events_per_second'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("bucket");
    assert_eq!(tokens, 0);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM inbox_events WHERE event_id='same-event'")
            .fetch_one(&w.db.pool)
            .await
            .expect("count");
    assert_eq!(count, 1);
}

#[tokio::test]
async fn zero_log_capacity_does_not_block_event_retry_or_dead_state() {
    use sagat_application::dispatch::EventDispatchStore;
    let w = world().await;
    w.events
        .publish(&w.admin, request(&w, "retry-no-log", 600), &ctx())
        .await
        .expect("publish");
    let event = w
        .store
        .claim_events(1, Duration::from_secs(60))
        .await
        .expect("claim")
        .pop()
        .expect("event");
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantLogBytes, 0, &ctx())
        .await
        .expect("no logs");
    sqlx::query("UPDATE quota_accounting_control SET log_enforcement_enabled=true")
        .execute(&w.db.pool)
        .await
        .expect("enable log guard");
    w.store
        .retry_event(
            &event,
            "échec diagnostic",
            Duration::from_secs(1),
            true,
            true,
        )
        .await
        .expect("terminal retry state");
    let row: (String, String, bool, i32) = sqlx::query_as("SELECT status,last_error,last_error_truncated,attempts FROM inbox_events WHERE event_id='retry-no-log'")
        .fetch_one(&w.db.pool).await.expect("event");
    assert_eq!(row, ("DEAD".into(), String::new(), true, 1));
}
