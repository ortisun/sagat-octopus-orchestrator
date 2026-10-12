//! Real broker proof for outbound subscriptions (`FR-004-005`, `T004-042`).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::StreamExt as _;
use rskafka::client::ClientBuilder;
use rskafka::client::partition::UnknownTopicHandling;
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::AppError;
use sagat_application::outbound::{
    BrokerFailure, BrokerPublisher, BrokerRequest, BrokerSubscriptionSettings, Health,
    OutboundStore, SubscriptionRecord,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::outbound::{
    AttemptOutcome, BrokerDestination, DeliveryId, DeliveryLimits, DeliveryStatus,
    OutboundDestination, OutboundKind, SubscriptionId, SubscriptionStatus,
};
use sagat_domain::secrets::SecretValue;
use sagat_infra_connectors::guard::{BrokerGuard, Unguarded};
use sagat_infra_connectors::publish::Publisher;
use sagat_infra_connectors::webhook::Clients;
use sagat_scheduler::{DeliverySettings, DeliveryWorker};

use crate::support::brokers;
use crate::support::engine::{World, ctx, world};
use crate::support::nats;
use crate::support::receiver::{LoopbackSender, OpenFactory};
use crate::support::{logs, now, slug};

#[derive(Debug, Clone, PartialEq, Eq)]
struct SeenBrokerRequest {
    destination: BrokerDestination,
    delivery_id: String,
    attempt: String,
}

#[derive(Clone, Default)]
struct ScriptedPublisher {
    state: Arc<Mutex<ScriptedPublisherState>>,
}

#[derive(Default)]
struct ScriptedPublisherState {
    results: VecDeque<Result<(), BrokerFailure>>,
    seen: Vec<SeenBrokerRequest>,
}

impl ScriptedPublisher {
    fn with_results(results: impl IntoIterator<Item = Result<(), BrokerFailure>>) -> Self {
        Self {
            state: Arc::new(Mutex::new(ScriptedPublisherState {
                results: results.into_iter().collect(),
                seen: Vec::new(),
            })),
        }
    }

    fn seen(&self) -> Vec<SeenBrokerRequest> {
        self.state.lock().expect("publisher state").seen.clone()
    }
}

impl BrokerPublisher for ScriptedPublisher {
    async fn publish(&self, request: &BrokerRequest) -> Result<(), BrokerFailure> {
        let header = |name: &str| {
            request
                .headers
                .iter()
                .find(|(candidate, _)| candidate == name)
                .map(|(_, value)| value.clone())
                .expect("worker header")
        };
        let mut state = self.state.lock().expect("publisher state");
        state.seen.push(SeenBrokerRequest {
            destination: request.destination.clone(),
            delivery_id: header("delivery-id"),
            attempt: header("attempt"),
        });
        state.results.pop_front().unwrap_or(Ok(()))
    }
}

struct ExpectedLastPublisher<P> {
    inner: P,
    expected: &'static str,
}

impl<P: BrokerPublisher> BrokerPublisher for ExpectedLastPublisher<P> {
    async fn publish(&self, request: &BrokerRequest) -> Result<(), BrokerFailure> {
        assert!(
            request.credentials.is_none(),
            "this test publisher does not copy secrets"
        );
        let mut headers = request.headers.clone();
        headers.push((
            "Nats-Expected-Last-Msg-Id".to_owned(),
            self.expected.to_owned(),
        ));
        self.inner
            .publish(&BrokerRequest {
                tenant_id: request.tenant_id,
                namespace_id: request.namespace_id,
                subscription_id: request.subscription_id,
                destination: request.destination.clone(),
                credentials: None,
                headers,
                body: request.body.clone(),
                timeout: request.timeout,
            })
            .await
    }
}

#[derive(Clone)]
struct SlowGuard {
    entered: Arc<AtomicBool>,
    completed: Arc<AtomicBool>,
}

impl BrokerGuard for SlowGuard {
    async fn permits(
        &self,
        _: sagat_domain::identity::TenantId,
        _: sagat_domain::identity::NamespaceId,
        _: &[sagat_domain::connectors::Server],
    ) -> bool {
        self.entered.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(250)).await;
        self.completed.store(true, Ordering::SeqCst);
        true
    }
}

async fn store_broker(
    world: &World,
    name: &str,
    destination: BrokerDestination,
) -> SubscriptionRecord {
    let created = now();
    let subscription = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: world.admin.tenant_id(),
        namespace_id: world.namespace.id(),
        name: slug(name),
        destination: OutboundDestination::Broker {
            settings: destination,
            credentials_secret: None,
        },
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::default(),
        status: SubscriptionStatus::Active,
        health: Health::default(),
        created_by: "test".to_owned(),
        created_at: created,
        updated_at: created,
    };
    let audit = AuditRecord::provisioning(
        world.admin.tenant_id(),
        AuditAction::BrokerSubscriptionCreated,
        ResourceType::BrokerSubscription,
        subscription.id.to_string(),
        "test",
        created,
    );
    world
        .store
        .create_subscription(&subscription, 50, &audit)
        .await
        .expect("broker subscription");
    subscription
}

async fn queue_delivery(
    world: &World,
    subscription: &SubscriptionRecord,
    attempts: u32,
) -> DeliveryId {
    let id = DeliveryId::generate();
    sqlx::query(
        "INSERT INTO webhook_deliveries (id, tenant_id, namespace_id, subscription_id, event_id, \
         kind, payload, attempts, next_attempt_at) \
         VALUES ($1, $2, $3, $4, 'broker-event', 'run.finished', '{\"safe\":true}', $5, clock_timestamp())",
    )
    .bind(id.as_uuid())
    .bind(subscription.tenant_id.as_uuid())
    .bind(subscription.namespace_id.as_uuid())
    .bind(subscription.id.as_uuid())
    .bind(i32::try_from(attempts).expect("attempt count"))
    .execute(&world.db.pool)
    .await
    .expect("queue broker delivery");
    id
}

async fn make_due(world: &World, subscription: SubscriptionId) {
    sqlx::query(
        "UPDATE webhook_deliveries SET next_attempt_at = clock_timestamp() \
         WHERE subscription_id = $1 AND status = 'PENDING'",
    )
    .bind(subscription.as_uuid())
    .execute(&world.db.pool)
    .await
    .expect("make broker delivery due");
    sqlx::query("UPDATE webhook_subscriptions SET next_slot_at = '-infinity' WHERE id = $1")
        .bind(subscription.as_uuid())
        .execute(&world.db.pool)
        .await
        .expect("make broker subscription ready");
}

fn request(
    world: &crate::support::engine::World,
    destination: BrokerDestination,
    delivery_id: &str,
) -> BrokerRequest {
    BrokerRequest {
        tenant_id: world.admin.tenant_id(),
        namespace_id: world.namespace.id(),
        subscription_id: SubscriptionId::generate(),
        destination,
        credentials: None,
        headers: vec![
            ("content-type".to_owned(), "application/json".to_owned()),
            ("event-id".to_owned(), "event-real-1".to_owned()),
            ("event-type".to_owned(), "run.finished".to_owned()),
            ("delivery-id".to_owned(), delivery_id.to_owned()),
            ("attempt".to_owned(), "1".to_owned()),
            ("schema-version".to_owned(), "1".to_owned()),
        ],
        body: br#"{"type":"run.finished","data":{"runId":"real-1"}}"#.to_vec(),
        timeout: Duration::from_secs(20),
    }
}

#[tokio::test]
async fn rollback_refuses_to_delete_broker_subscriptions() {
    let world = world().await;
    let created = now();
    let subscription = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: world.admin.tenant_id(),
        namespace_id: world.namespace.id(),
        name: slug("rollback-proof"),
        destination: OutboundDestination::Broker {
            settings: BrokerDestination::kafka(&["kafka.example.com:9092".to_owned()], "runs")
                .expect("destination"),
            credentials_secret: None,
        },
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::default(),
        status: SubscriptionStatus::Active,
        health: Health::default(),
        created_by: "test".to_owned(),
        created_at: created,
        updated_at: created,
    };
    let audit = AuditRecord::provisioning(
        world.admin.tenant_id(),
        AuditAction::BrokerSubscriptionCreated,
        ResourceType::BrokerSubscription,
        subscription.id.to_string(),
        "test",
        created,
    );
    world
        .store
        .create_subscription(&subscription, 50, &audit)
        .await
        .expect("broker subscription");

    let error = sqlx::raw_sql(include_str!(
        "../../../crates/infra-postgres/migrations/20261021000000_outbound_brokers.down.sql"
    ))
    .execute(&world.db.pool)
    .await
    .expect_err("rollback must refuse broker customer data");
    assert!(
        error
            .to_string()
            .contains("cannot roll back outbound brokers"),
        "unexpected rollback error: {error}"
    );
    assert_eq!(
        world
            .store
            .subscription(world.admin.tenant_id(), subscription.id)
            .await
            .expect("read after refused rollback"),
        Some(subscription.clone())
    );

    sqlx::query("DELETE FROM webhook_subscriptions WHERE id = $1")
        .bind(subscription.id.as_uuid())
        .execute(&world.db.pool)
        .await
        .expect("remove broker row before rollback");
    sqlx::raw_sql(include_str!(
        "../../../crates/infra-postgres/migrations/20261021000000_outbound_brokers.down.sql"
    ))
    .execute(&world.db.pool)
    .await
    .expect("empty rollback");
    let destination_column: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'webhook_subscriptions' \
           AND column_name = 'destination_kind')",
    )
    .fetch_one(&world.db.pool)
    .await
    .expect("inspect rolled back schema");
    assert!(!destination_column, "rollback removes the additive column");

    sqlx::raw_sql(include_str!(
        "../../../crates/infra-postgres/migrations/20261021000000_outbound_brokers.up.sql"
    ))
    .execute(&world.db.pool)
    .await
    .expect("forward migration after rollback");
    let destination_column: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'webhook_subscriptions' \
           AND column_name = 'destination_kind')",
    )
    .fetch_one(&world.db.pool)
    .await
    .expect("inspect restored schema");
    assert!(destination_column, "forward migration restores the column");
}

#[tokio::test]
async fn ambiguous_broker_ack_retries_the_selected_destination_with_the_same_delivery_id() {
    let world = world().await;
    let destination =
        BrokerDestination::nats(&["nats.example.com:4222".to_owned()], "events.finished")
            .expect("destination");
    let subscription = store_broker(&world, "ambiguous-ack", destination.clone()).await;
    let delivery = queue_delivery(&world, &subscription, 0).await;
    let publisher = ScriptedPublisher::with_results([Err(BrokerFailure::Timeout), Ok(())]);
    let worker = DeliveryWorker::with_broker(
        world.store.clone(),
        LoopbackSender::new(),
        publisher.clone(),
        SystemEntropy,
        world.keys.cipher(),
        DeliverySettings::default(),
    );

    let first = worker.tick().await;
    assert_eq!((first.attempted, first.retried), (1, 1));
    drop(worker);
    make_due(&world, subscription.id).await;
    let restarted_worker = DeliveryWorker::with_broker(
        world.store.clone(),
        LoopbackSender::new(),
        publisher.clone(),
        SystemEntropy,
        world.keys.cipher(),
        DeliverySettings::default(),
    );
    let second = restarted_worker.tick().await;
    assert_eq!((second.attempted, second.delivered), (1, 1));

    let seen = publisher.seen();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].destination, destination);
    assert_eq!(seen[1].destination, destination);
    assert_eq!(seen[0].delivery_id, delivery.to_string());
    assert_eq!(seen[1].delivery_id, delivery.to_string());
    assert_eq!(seen[0].attempt, "1");
    assert_eq!(seen[1].attempt, "2");

    let (record, attempts) = world
        .outbound
        .broker_delivery(&world.admin, subscription.id, delivery)
        .await
        .expect("broker delivery");
    assert_eq!(record.status, DeliveryStatus::Delivered);
    assert_eq!(record.attempts, 2);
    assert_eq!(
        attempts
            .iter()
            .map(|attempt| attempt.outcome)
            .collect::<Vec<_>>(),
        vec![AttemptOutcome::Timeout, AttemptOutcome::Delivered]
    );
}

#[tokio::test]
async fn broker_dead_letter_redrive_keeps_the_destination_and_delivery_identity() {
    let world = world().await;
    let destination =
        BrokerDestination::kafka(&["kafka.example.com:9092".to_owned()], "run-finished")
            .expect("destination");
    let subscription = store_broker(&world, "broker-redrive", destination.clone()).await;
    let delivery = queue_delivery(&world, &subscription, 11).await;
    let publisher = ScriptedPublisher::with_results([Err(BrokerFailure::Rejected), Ok(())]);
    let worker = DeliveryWorker::with_broker(
        world.store.clone(),
        LoopbackSender::new(),
        publisher.clone(),
        SystemEntropy,
        world.keys.cipher(),
        DeliverySettings::default(),
    );

    let dead = worker.tick().await;
    assert_eq!((dead.attempted, dead.dead), (1, 1));
    let (record, attempts) = world
        .outbound
        .broker_delivery(&world.admin, subscription.id, delivery)
        .await
        .expect("dead broker delivery");
    assert_eq!(record.status, DeliveryStatus::Dead);
    assert_eq!(
        attempts.last().map(|attempt| attempt.outcome),
        Some(AttemptOutcome::Rejected)
    );

    assert_eq!(
        world
            .outbound
            .redrive_broker(&world.admin, subscription.id, Some(delivery), 1, &ctx(),)
            .await
            .expect("redrive broker delivery"),
        1
    );
    make_due(&world, subscription.id).await;
    let delivered = worker.tick().await;
    assert_eq!((delivered.attempted, delivered.delivered), (1, 1));

    let seen = publisher.seen();
    assert_eq!(seen.len(), 2);
    assert!(
        seen.iter()
            .all(|request| request.destination == destination)
    );
    assert!(
        seen.iter()
            .all(|request| request.delivery_id == delivery.to_string())
    );
    assert_eq!(seen[0].attempt, "12");
    assert_eq!(seen[1].attempt, "13");
    let (record, attempts) = world
        .outbound
        .broker_delivery(&world.admin, subscription.id, delivery)
        .await
        .expect("redriven broker delivery");
    assert_eq!(record.status, DeliveryStatus::Delivered);
    assert_eq!(record.attempts, 13);
    assert_eq!(attempts.len(), 2, "redrive preserves attempt history");
}

#[tokio::test]
async fn broker_management_is_tenant_scoped_denies_loopback_and_redacts_credentials() {
    let world = world().await;
    let destination =
        BrokerDestination::nats(&["nats.example.com:4222".to_owned()], "events.finished")
            .expect("destination");
    let subscription = store_broker(&world, "tenant-private", destination.clone()).await;
    assert_eq!(
        world
            .outbound
            .get_broker(&world.other_admin, subscription.id)
            .await,
        Err(AppError::NotFound)
    );

    let loopback = BrokerSubscriptionSettings {
        destination: BrokerDestination::nats(&["127.0.0.1:4222".to_owned()], "private.subject")
            .expect("loopback destination"),
        credentials_secret: None,
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::default(),
        status: SubscriptionStatus::Active,
    };
    assert_eq!(
        world
            .outbound
            .create_broker(
                &world.admin,
                world.namespace.id(),
                slug("loopback-denied"),
                loopback,
                &ctx(),
            )
            .await,
        Err(AppError::NotAllowed(
            "the egress policy of the namespace does not allow that broker"
        ))
    );

    let mut sensitive = request(&world, destination, "delivery-secret-proof");
    sensitive.credentials =
        Some(SecretValue::new(br#"{"token":"do-not-log-this"}"#.to_vec()).expect("secret value"));
    let debug = format!("{sensitive:?}");
    assert!(
        debug.contains("<redacted>"),
        "credentials are visibly redacted"
    );
    assert!(
        !debug.contains("do-not-log-this"),
        "credentials leaked: {debug}"
    );
}

#[tokio::test]
async fn nats_puback_details_never_reach_logs_or_durable_failure_state() {
    const SENTINEL: &str = "puback-must-not-leak-7d34b2";
    const WRONG_EXPECTATION: &str = "a-different-last-message-id";

    let world = world().await;
    let nats = nats::start().await;
    let client = async_nats::connect(nats.url()).await.expect("NATS client");
    let jetstream = async_nats::jetstream::new(client);
    jetstream
        .create_stream(async_nats::jetstream::stream::Config {
            name: "OUTBOUND_REJECT".to_owned(),
            subjects: vec!["events.rejected".to_owned()],
            ..Default::default()
        })
        .await
        .expect("NATS stream");
    let mut seed_headers = async_nats::HeaderMap::new();
    seed_headers.insert("Nats-Msg-Id", SENTINEL);
    jetstream
        .publish_with_headers("events.rejected", seed_headers, "seed".into())
        .await
        .expect("publish seed")
        .await
        .expect("seed PubAck");
    let mut reject_headers = async_nats::HeaderMap::new();
    reject_headers.insert("Nats-Expected-Last-Msg-Id", WRONG_EXPECTATION);
    let rejection = jetstream
        .publish_with_headers("events.rejected", reject_headers, "probe".into())
        .await
        .expect("send rejected publish")
        .await
        .expect_err("conditional publish must be rejected");
    assert!(
        rejection.to_string().contains(SENTINEL),
        "the real PubAck error must contain the sentinel for this redaction proof: {rejection}"
    );
    let destination =
        BrokerDestination::nats(&[nats.url()], "events.rejected").expect("NATS destination");
    let subscription = store_broker(&world, "nats-redacted-rejection", destination).await;
    let delivery = queue_delivery(&world, &subscription, 0).await;
    let publisher = ExpectedLastPublisher {
        inner: Publisher::with_parts(
            Unguarded,
            Clients::with_factory(world.store.clone(), OpenFactory),
        ),
        expected: WRONG_EXPECTATION,
    };
    let worker = DeliveryWorker::with_broker(
        world.store.clone(),
        LoopbackSender::new(),
        publisher,
        SystemEntropy,
        world.keys.cipher(),
        DeliverySettings::default(),
    );

    let (_guard, captured) = logs::capture();
    let result = worker.tick().await;
    assert_eq!((result.attempted, result.retried), (1, 1));
    let log = captured.text();
    assert!(
        log.contains("NATS JetStream rejected an outbound publish"),
        "the rejection remains observable: {log}"
    );
    assert!(!log.contains(SENTINEL), "PubAck detail leaked: {log}");

    let (record, attempts) = world
        .outbound
        .broker_delivery(&world.admin, subscription.id, delivery)
        .await
        .expect("rejected delivery");
    assert_eq!(record.status, DeliveryStatus::Pending);
    assert_eq!(record.last_outcome, Some(AttemptOutcome::Rejected));
    assert_eq!(
        record.last_error.as_deref(),
        Some("the receiver refused the event")
    );
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Rejected);
    assert_eq!(
        attempts[0].error.as_deref(),
        Some("the receiver refused the event")
    );
    assert!(
        !record
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains(SENTINEL)
            && !attempts[0]
                .error
                .as_deref()
                .unwrap_or_default()
                .contains(SENTINEL),
        "PubAck detail reached durable state"
    );
}

#[tokio::test]
async fn an_sqs_http_success_without_a_send_result_is_retried() {
    let world = world().await;
    let receiver = crate::support::receiver::Receiver::start().await;
    receiver.script([crate::support::receiver::Reply::StatusBody(
        200,
        "<html>accepted-ish</html>",
    )]);
    let destination = BrokerDestination::sqs(&receiver.url, "us-east-1").expect("SQS destination");
    let subscription = store_broker(&world, "sqs-malformed-success", destination).await;
    let delivery = queue_delivery(&world, &subscription, 0).await;
    let worker = DeliveryWorker::with_broker(
        world.store.clone(),
        LoopbackSender::new(),
        Publisher::with_parts(
            Unguarded,
            Clients::with_factory(world.store.clone(), OpenFactory),
        ),
        SystemEntropy,
        world.keys.cipher(),
        DeliverySettings::default(),
    );

    let result = worker.tick().await;
    assert_eq!(
        (result.attempted, result.retried, result.delivered),
        (1, 1, 0)
    );
    let (record, attempts) = world
        .outbound
        .broker_delivery(&world.admin, subscription.id, delivery)
        .await
        .expect("malformed SQS response delivery");
    assert_eq!(record.status, DeliveryStatus::Pending);
    assert_eq!(record.last_outcome, Some(AttemptOutcome::Rejected));
    assert_eq!(attempts.len(), 1);
    assert_eq!(attempts[0].outcome, AttemptOutcome::Rejected);
}

#[tokio::test]
async fn broker_timeout_covers_the_guard_and_cancels_it_before_effects() {
    let world = world().await;
    let entered = Arc::new(AtomicBool::new(false));
    let completed = Arc::new(AtomicBool::new(false));
    let publisher = Publisher::with_parts(
        SlowGuard {
            entered: Arc::clone(&entered),
            completed: Arc::clone(&completed),
        },
        Clients::with_factory(world.store.clone(), OpenFactory),
    );
    let mut outbound = request(
        &world,
        BrokerDestination::nats(&["127.0.0.1:1".to_owned()], "events.timeout")
            .expect("NATS destination"),
        "deadline-covers-guard",
    );
    outbound.timeout = Duration::from_millis(25);

    assert_eq!(
        publisher.publish(&outbound).await,
        Err(BrokerFailure::Timeout)
    );
    assert!(entered.load(Ordering::SeqCst), "guard was entered");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !completed.load(Ordering::SeqCst),
        "cancelled guard must not resume and perform its later effect"
    );
}

#[tokio::test]
async fn nats_kafka_and_sqs_acknowledge_real_publishes_with_stable_delivery_metadata() {
    let world = world().await;
    let publisher = Publisher::with_parts(
        Unguarded,
        Clients::with_factory(world.store.clone(), OpenFactory),
    );

    // JetStream: publishing the same durable delivery twice with Nats-Msg-Id stores it once.
    let nats = nats::start().await;
    let client = async_nats::connect(nats.url()).await.expect("NATS client");
    let js = async_nats::jetstream::new(client);
    js.create_stream(async_nats::jetstream::stream::Config {
        name: "OUTBOUND".to_owned(),
        subjects: vec!["events.finished".to_owned()],
        ..Default::default()
    })
    .await
    .expect("NATS stream");
    let nats_request = request(
        &world,
        BrokerDestination::nats(&[nats.url()], "events.finished").expect("NATS destination"),
        "0199d6f3-9ec3-7f10-b50a-31e7801370a1",
    );
    publisher.publish(&nats_request).await.expect("NATS PubAck");
    publisher
        .publish(&nats_request)
        .await
        .expect("repeated NATS PubAck");
    let mut stream = js.get_stream("OUTBOUND").await.expect("NATS stream");
    assert_eq!(stream.info().await.expect("stream info").state.messages, 1);
    let consumer = stream
        .create_consumer(async_nats::jetstream::consumer::pull::Config::default())
        .await
        .expect("NATS consumer");
    let mut messages = consumer
        .fetch()
        .max_messages(1)
        .messages()
        .await
        .expect("NATS messages");
    let message = messages
        .next()
        .await
        .expect("one message")
        .expect("message");
    assert_eq!(
        message
            .headers
            .as_ref()
            .and_then(|headers| headers.get("Nats-Msg-Id"))
            .map(ToString::to_string)
            .as_deref(),
        Some("0199d6f3-9ec3-7f10-b50a-31e7801370a1")
    );
    assert_eq!(message.payload.as_ref(), nats_request.body.as_slice());
    drop(nats);

    // Kafka: the durable delivery id is the record key and header; the broker's produce response
    // is the acknowledgement returned to the worker.
    let kafka = brokers::kafka().await;
    kafka.create_topic("outbound-runs", 3).await;
    let kafka_request = request(
        &world,
        BrokerDestination::kafka(std::slice::from_ref(&kafka.bootstrap), "outbound-runs")
            .expect("Kafka destination"),
        "0199d6f3-9ec3-7f10-b50a-31e7801370a2",
    );
    publisher
        .publish(&kafka_request)
        .await
        .expect("Kafka produce acknowledgement");
    let client = ClientBuilder::new(vec![kafka.bootstrap.clone()])
        .build()
        .await
        .expect("Kafka client");
    let mut found = None;
    for partition in 0..3 {
        let partition = client
            .partition_client(
                "outbound-runs".to_owned(),
                partition,
                UnknownTopicHandling::Retry,
            )
            .await
            .expect("partition");
        let (records, _) = partition
            .fetch_records(0, 1..1_000_000, 1_000)
            .await
            .expect("fetch");
        if let Some(record) = records.into_iter().next() {
            found = Some(record.record);
            break;
        }
    }
    let record = found.expect("one Kafka record");
    assert_eq!(
        record.key.as_deref(),
        Some(b"0199d6f3-9ec3-7f10-b50a-31e7801370a2".as_slice())
    );
    assert_eq!(
        record.headers.get("delivery-id").map(Vec::as_slice),
        Some(b"0199d6f3-9ec3-7f10-b50a-31e7801370a2".as_slice())
    );
    assert_eq!(record.value.as_deref(), Some(kafka_request.body.as_slice()));
    drop(kafka);

    // SQS FIFO: the request is accepted and carries the stable id as a message attribute as well
    // as the protocol-native deduplication id.
    let sqs = brokers::sqs().await;
    let queue = sqs.create_queue_with("outbound-runs.fifo", true).await;
    let sqs_request = request(
        &world,
        BrokerDestination::sqs(&queue, "us-east-1").expect("SQS destination"),
        "0199d6f3-9ec3-7f10-b50a-31e7801370a3",
    );
    publisher
        .publish(&sqs_request)
        .await
        .expect("SQS SendMessage acknowledgement");
    assert_eq!(sqs.depth(&queue).await.0, 1);
    let message = sqs.receive_raw(&queue).await;
    assert!(
        message.contains("delivery-id"),
        "missing delivery-id: {message}"
    );
    assert!(
        message.contains("0199d6f3-9ec3-7f10-b50a-31e7801370a3"),
        "missing stable delivery id: {message}"
    );
    assert!(
        message.contains("real-1"),
        "missing published body: {message}"
    );
}
