//! Callback fixtures bypass loopback egress only through the test-only guard; production denial
//! and namespace management are exercised separately. All delivery state uses real PostgreSQL.
use super::{read, redis, support};
use sagat_application::entropy::SystemEntropy;
use sagat_application::outbound::{Health, OutboundStore, SubscriptionRecord};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::outbound::{
    BrokerDestination, DeliveryLimits, DeliveryStatus, OutboundDestination, OutboundKind,
    SubscriptionId, SubscriptionStatus,
};
use sagat_infra_connectors::{guard::Unguarded, publish::Publisher, webhook::Clients};
use sagat_scheduler::{DeliverySettings, DeliveryWorker};
use std::time::Duration;
use support::engine::{World, activity, document};
use support::receiver::{LoopbackSender, OpenFactory};

pub(super) async fn subscribe(w: &World, url: &str) -> SubscriptionRecord {
    let created = support::now();
    let record = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        name: support::slug("redis-callback"),
        destination: OutboundDestination::Broker {
            settings: BrokerDestination::redis(url.trim_end_matches("/0"), "test:callbacks", 0)
                .expect("destination"),
            credentials_secret: None,
        },
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::new(100, Duration::from_secs(2)).expect("limits"),
        status: SubscriptionStatus::Active,
        health: Health::default(),
        created_by: "test".to_owned(),
        created_at: created,
        updated_at: created,
    };
    let audit = AuditRecord::provisioning(
        record.tenant_id,
        AuditAction::BrokerSubscriptionCreated,
        ResourceType::BrokerSubscription,
        record.id.to_string(),
        "test",
        created,
    );
    w.store
        .create_subscription(&record, 100, &audit)
        .await
        .expect("store owned fixture");
    record
}
pub(super) async fn finish(w: &World) {
    let (workflow, _) = w
        .publish(document("redis-run", "work", vec![activity("work")]))
        .await;
    w.start(workflow, &serde_json::json!({})).await;
    let session = w.session(&["act.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &serde_json::json!({"ok":true}))
        .await
        .expect("complete real run");
}
pub(super) fn worker(
    w: &World,
) -> DeliveryWorker<
    sagat_infra_postgres::PgRunStore,
    LoopbackSender,
    SystemEntropy,
    Publisher<Unguarded, sagat_infra_postgres::PgRunStore, OpenFactory>,
> {
    DeliveryWorker::with_broker(
        w.store.clone(),
        LoopbackSender::new(),
        Publisher::with_parts(
            Unguarded,
            Clients::with_factory(w.store.clone(), OpenFactory),
        ),
        SystemEntropy,
        w.keys.cipher(),
        DeliverySettings::default(),
    )
}

#[tokio::test]
async fn a_finished_run_reaches_redis_through_the_existing_durable_delivery_worker() {
    let (_redis, url, client) = redis().await;
    let w = support::engine::world().await;
    let subscription = subscribe(&w, &url).await;
    finish(&w).await;
    let round = worker(&w).tick().await;
    assert_eq!(
        (round.attempted, round.delivered),
        (1, 1),
        "the real worker metadata must be accepted"
    );
    let deliveries = w
        .store
        .deliveries(w.admin.tenant_id(), subscription.id, None, None, 10)
        .await
        .expect("deliveries");
    assert_eq!(deliveries.len(), 1);
    let delivery = &deliveries[0];
    assert_eq!(delivery.status, DeliveryStatus::Delivered);
    let entries = read(&client, "test:callbacks").await.ids;
    assert_eq!(entries.len(), 1);
    let entry = &entries[0];
    assert_eq!(
        entry.get::<Vec<u8>>("body").expect("body"),
        delivery.payload
    );
    assert_eq!(
        entry.get::<String>("event-id").expect("event"),
        delivery.event_id
    );
    assert_eq!(
        entry.get::<String>("delivery-id").expect("delivery"),
        delivery.id.to_string()
    );
    assert_eq!(
        entry.get::<String>("event-type").expect("kind"),
        "run.finished"
    );
    assert_eq!(entry.get::<String>("attempt").expect("attempt"), "1");
}

async fn due(w: &World, subscription: SubscriptionId) {
    sqlx::query("UPDATE webhook_deliveries SET next_attempt_at = clock_timestamp() WHERE subscription_id = $1 AND status = 'PENDING'")
        .bind(subscription.as_uuid()).execute(&w.db.pool).await.expect("age retry");
    sqlx::query("UPDATE webhook_subscriptions SET next_slot_at = '-infinity' WHERE id = $1")
        .bind(subscription.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("age rate slot");
}

#[tokio::test]
async fn redis_outage_exhaustion_and_redrive_retain_the_same_delivery() {
    let (redis, url, client) = redis().await;
    let w = support::engine::world().await;
    let subscription = subscribe(&w, &url).await;
    finish(&w).await;
    // Set the existing durable attempt counter to the last permitted attempt; no clock sleeps.
    sqlx::query("UPDATE webhook_deliveries SET attempts = 11 WHERE subscription_id = $1")
        .bind(subscription.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("last attempt");
    redis.pause().await.expect("pause owned broker");
    let round = worker(&w).tick().await;
    assert_eq!((round.attempted, round.dead), (1, 1));
    let dead = w
        .store
        .deliveries(
            w.admin.tenant_id(),
            subscription.id,
            Some(DeliveryStatus::Dead),
            None,
            10,
        )
        .await
        .expect("DLQ");
    assert_eq!(dead.len(), 1);
    let id = dead[0].id;
    redis.unpause().await.expect("recover owned broker");
    assert_eq!(
        w.outbound
            .redrive_broker(
                &w.admin,
                subscription.id,
                Some(id),
                1,
                &support::engine::ctx()
            )
            .await
            .expect("redrive"),
        1
    );
    due(&w, subscription.id).await;
    let recovered = worker(&w).tick().await;
    assert_eq!((recovered.attempted, recovered.delivered), (1, 1));
    let entries = read(&client, "test:callbacks").await.ids;
    // The timed-out write may have been processed after unpause, so physical duplicates are valid.
    assert!(!entries.is_empty());
    for entry in entries {
        assert_eq!(
            entry.get::<String>("delivery-id").expect("id"),
            id.to_string()
        );
    }
    let (delivery, attempts) = w
        .store
        .delivery(w.admin.tenant_id(), subscription.id, id)
        .await
        .expect("state")
        .expect("retained");
    assert_eq!(delivery.status, DeliveryStatus::Delivered);
    assert_eq!(attempts.len(), 2);
}

#[tokio::test]
async fn wrong_type_stream_retries_then_recovers_without_trimming_customer_entries() {
    let (_redis, url, client) = redis().await;
    let w = support::engine::world().await;
    let subscription = subscribe(&w, &url).await;
    finish(&w).await;
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .expect("fixture");
    redis::cmd("SET")
        .arg("test:callbacks")
        .arg("wrong-type")
        .query_async::<()>(&mut connection)
        .await
        .expect("wrong type");
    let rejected = worker(&w).tick().await;
    assert_eq!((rejected.attempted, rejected.retried), (1, 1));
    redis::cmd("DEL")
        .arg("test:callbacks")
        .query_async::<i64>(&mut connection)
        .await
        .expect("repair");
    redis::cmd("XADD")
        .arg("test:callbacks")
        .arg("*")
        .arg("customer")
        .arg("retain-me")
        .query_async::<String>(&mut connection)
        .await
        .expect("customer entry");
    due(&w, subscription.id).await;
    assert_eq!(worker(&w).tick().await.delivered, 1);
    let entries = read(&client, "test:callbacks").await.ids;
    assert_eq!(entries.len(), 2);
    assert_eq!(
        entries[0]
            .get::<String>("customer")
            .expect("customer retained"),
        "retain-me"
    );
    assert_eq!(entries[1].get::<String>("attempt").expect("attempt"), "2");
}

#[tokio::test]
async fn redis_callback_creation_is_disabled_until_explicit_activation_and_stays_tenant_scoped() {
    use sagat_application::{
        clock::SystemClock,
        egress::EgressService,
        error::AppError,
        outbound::{BrokerSubscriptionSettings, OutboundService},
    };
    let w = support::engine::world().await;
    let settings = || BrokerSubscriptionSettings {
        destination: BrokerDestination::redis("redis://127.0.0.1:6379", "callbacks", 0)
            .expect("destination"),
        credentials_secret: None,
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::default(),
        status: SubscriptionStatus::Active,
    };
    assert_eq!(
        w.outbound
            .create_broker(
                &w.admin,
                w.namespace.id(),
                support::slug("redis-disabled"),
                settings(),
                &support::engine::ctx()
            )
            .await,
        Err(AppError::NotAllowed(
            "Redis callbacks require the completed rollout activation"
        ))
    );
    let enabled = OutboundService::new(
        w.store.clone(),
        SystemClock,
        EgressService::new(w.store.clone(), SystemClock),
    )
    .with_redis_callbacks(true);
    assert_eq!(
        enabled
            .create_broker(
                &w.admin,
                w.namespace.id(),
                support::slug("redis-denied"),
                settings(),
                &support::engine::ctx()
            )
            .await,
        Err(AppError::NotAllowed(
            "the egress policy of the namespace does not allow that broker"
        ))
    );
    let stored = subscribe(&w, "redis://127.0.0.1:6379/0").await;
    assert_eq!(
        enabled.get_broker(&w.other_admin, stored.id).await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        enabled
            .get_broker(&w.admin, stored.id)
            .await
            .expect("owner")
            .destination,
        stored.destination
    );
}

struct AfterAppend<P> {
    inner: P,
    accepted: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
    release: tokio::sync::Mutex<tokio::sync::oneshot::Receiver<()>>,
}
impl<P: sagat_application::outbound::BrokerPublisher> sagat_application::outbound::BrokerPublisher
    for AfterAppend<P>
{
    async fn publish(
        &self,
        request: &sagat_application::outbound::BrokerRequest,
    ) -> Result<(), sagat_application::outbound::BrokerFailure> {
        self.inner.publish(request).await?;
        self.accepted
            .lock()
            .expect("barrier")
            .take()
            .expect("one append")
            .send(())
            .expect("observer");
        let _ = (&mut *self.release.lock().await).await;
        Err(sagat_application::outbound::BrokerFailure::Network)
    }
}
async fn uncertain_append(crash: bool) {
    let (_redis, url, client) = redis().await;
    let w = support::engine::world().await;
    let subscription = subscribe(&w, &url).await;
    finish(&w).await;
    let (accepted, observed) = tokio::sync::oneshot::channel();
    let (release, released) = tokio::sync::oneshot::channel();
    let first = DeliveryWorker::with_broker(
        w.store.clone(),
        LoopbackSender::new(),
        AfterAppend {
            inner: Publisher::with_parts(
                Unguarded,
                Clients::with_factory(w.store.clone(), OpenFactory),
            ),
            accepted: std::sync::Mutex::new(Some(accepted)),
            release: tokio::sync::Mutex::new(released),
        },
        SystemEntropy,
        w.keys.cipher(),
        DeliverySettings::default(),
    );
    let first = tokio::spawn(async move { first.tick().await });
    support::bounded::within(
        "actual append before PG acknowledgement",
        Duration::from_secs(10),
        observed,
    )
    .await
    .expect("append observed");
    assert_eq!(read(&client, "test:callbacks").await.ids.len(), 1);
    if crash {
        first.abort();
        assert!(
            support::bounded::within("join crashed worker", Duration::from_secs(10), first)
                .await
                .expect_err("cancelled")
                .is_cancelled()
        );
        due(&w, subscription.id).await;
        assert_eq!(worker(&w).tick().await.delivered, 1);
        drop(release);
    } else {
        due(&w, subscription.id).await;
        assert_eq!(worker(&w).tick().await.delivered, 1);
        release.send(()).expect("release stale worker");
        let stale =
            support::bounded::within("join superseded worker", Duration::from_secs(10), first)
                .await
                .expect("worker");
        assert_eq!(stale.attempted, 1); // Durable state/attempt assertions below prove the fence.
    }
    let entries = read(&client, "test:callbacks").await.ids;
    assert_eq!(
        entries.len(),
        2,
        "uncertain external effect can be physically repeated"
    );
    assert_ne!(entries[0].id, entries[1].id);
    assert_eq!(
        entries[0].get::<String>("delivery-id"),
        entries[1].get::<String>("delivery-id")
    );
    assert_eq!(
        entries[0].get::<Vec<u8>>("body"),
        entries[1].get::<Vec<u8>>("body")
    );
    let delivered = w
        .store
        .deliveries(w.admin.tenant_id(), subscription.id, None, None, 10)
        .await
        .expect("deliveries");
    assert_eq!(delivered.len(), 1);
    assert_eq!(delivered[0].status, DeliveryStatus::Delivered);
    let (_, attempts) = w
        .store
        .delivery(w.admin.tenant_id(), subscription.id, delivered[0].id)
        .await
        .expect("state")
        .expect("delivery");
    assert_eq!(
        attempts.len(),
        1,
        "a crashed or superseded publisher cannot add durable settlement"
    );
}
#[tokio::test]
async fn crash_after_redis_append_replays_the_same_application_identity() {
    uncertain_append(true).await;
}
#[tokio::test]
async fn a_stale_redis_publisher_cannot_overwrite_the_new_workers_settlement() {
    uncertain_append(false).await;
}
