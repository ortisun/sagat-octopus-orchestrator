//! US03 — integrate Kafka and SQS (and NATS): messages are redelivered, arrive out of order, and
//! some are not events at all; the inbox deduplicates, the ones that cannot be events are set aside,
//! and the position survives a lost commit (US03-AC01).

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use sagat_domain::connectors::{ConnectorSettings, StartFrom};
use serde_json::{Value, json};

use crate::support::brokers::{self, Running, Wanted, record, store_connector, supervisor};
use crate::support::nats;
use crate::support::worker::{Platform, platform, single_ir};
use crate::{engine, runs_of, within};

fn on_order(name: &str, connector: &str) -> Value {
    let mut ir = single_ir(name, None);
    ir["triggers"] =
        json!([{"kind": "message", "connector": connector, "eventType": "order.created"}]);
    ir
}

async fn scalar(p: &Platform, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_owned()))
        .fetch_one(&p.api.db.pool)
        .await
        .expect("count")
}

async fn namespace_of(p: &Platform) -> sagat_domain::identity::NamespaceId {
    p.namespace_id().await.parse().expect("namespace id")
}

#[tokio::test]
async fn us03_kafka_redelivery_poison_messages_and_a_lost_offset_commit_leave_one_run_per_event() {
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let kafka = brokers::kafka().await;
    kafka.create_topic("orders", 2).await;
    let workflow = p.publish("on-order", &on_order("on-order", "orders")).await;
    let namespace = namespace_of(&p).await;
    let connector = store_connector(
        &p.api.executions,
        p.tenant,
        namespace,
        Wanted {
            slug: "orders",
            settings: ConnectorSettings::kafka(
                std::slice::from_ref(&kafka.bootstrap),
                "orders",
                StartFrom::Earliest,
            )
            .expect("settings"),
            secret: None,
            default_type: None,
            rate: 1000,
        },
    )
    .await;

    // Three orders, one of them delivered twice (a producer retry), one message that is not JSON and
    // one that says nothing about its type: spread over the two partitions, in no useful order.
    let ok = |id: &str, order: &str| {
        record(
            &[
                ("eventType", "order.created"),
                ("eventId", id),
                ("correlationKey", order),
            ],
            &format!(r#"{{"order":"{order}"}}"#),
        )
    };
    kafka
        .produce(
            "orders",
            0,
            vec![
                ok("o-2", "ord-2"),
                ok("o-1", "ord-1"),
                record(&[("eventType", "order.created")], "this is not json"),
            ],
        )
        .await;
    kafka
        .produce(
            "orders",
            1,
            vec![
                ok("o-3", "ord-3"),
                ok("o-2", "ord-2"),
                record(&[], r#"{"no":"type"}"#),
            ],
        )
        .await;

    let mut running = Running::start(supervisor(
        &p.api.executions,
        p.api.keys.cipher(),
        "replica-a",
    ));
    within("three runs, one per order", 60, || async {
        (runs_of(&p, &workflow).await.len() == 3).then_some(())
    })
    .await;
    within("the poison messages to be set aside", 30, || async {
        (scalar(&p, "SELECT count(*) FROM connector_dead_letters").await == 2).then_some(())
    })
    .await;
    assert_eq!(
        scalar(
            &p,
            "SELECT count(*) FROM inbox_events WHERE source = 'orders'"
        )
        .await,
        3,
        "one event per order id"
    );
    let positions: Vec<(String, i64)> =
        sqlx::query_as("SELECT stream, next_offset FROM connector_offsets ORDER BY stream")
            .fetch_all(&p.api.db.pool)
            .await
            .expect("offsets");
    assert_eq!(
        positions,
        vec![("orders/0".to_owned(), 3), ("orders/1".to_owned(), 3)],
        "every message is behind the position, the poison ones too"
    );
    let reasons: Vec<String> =
        sqlx::query_scalar("SELECT reason FROM connector_dead_letters ORDER BY reason")
            .fetch_all(&p.api.db.pool)
            .await
            .expect("reasons");
    assert_eq!(reasons, ["NOT_A_JSON_OBJECT", "NO_EVENT_TYPE"]);

    // The connector says what it has read.
    let ns = p.namespace_id().await;
    let shown = within("the connector to show what it read", 20, || async {
        let (status, shown) = p
            .rest(
                Method::GET,
                &format!("/namespaces/{ns}/connectors/{}", connector.id),
                None,
            )
            .await;
        assert_eq!(status, 200, "{shown}");
        (shown["health"]["deadLettered"] == 2 && shown["health"]["consumed"].as_u64() >= Some(4))
            .then_some(shown)
    })
    .await;
    assert_eq!(shown["slug"], "orders");
    assert!(shown["health"]["lastMessageAt"].is_string());

    // "The offset commit was lost": the position goes back to the start while the consumer runs. It
    // reads everything again, and nothing happens twice.
    sqlx::query("UPDATE connector_offsets SET next_offset = 0")
        .execute(&p.api.db.pool)
        .await
        .expect("lose the commit");
    kafka.produce("orders", 0, vec![ok("o-4", "ord-4")]).await;
    within("the fourth order", 60, || async {
        (runs_of(&p, &workflow).await.len() == 4).then_some(())
    })
    .await;
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        runs_of(&p, &workflow).await.len(),
        4,
        "the replayed messages started nothing"
    );
    assert_eq!(
        scalar(
            &p,
            "SELECT count(*) FROM inbox_events WHERE source = 'orders'"
        )
        .await,
        4
    );
    assert_eq!(
        scalar(&p, "SELECT count(*) FROM connector_dead_letters").await,
        2,
        "and set nothing aside twice"
    );

    // A restart reads on from where the database says, and a second replica reads nothing the first
    // is reading.
    running.stop().await;
    kafka.produce("orders", 1, vec![ok("o-5", "ord-5")]).await;
    let mut a = Running::start(supervisor(
        &p.api.executions,
        p.api.keys.cipher(),
        "replica-a",
    ));
    let mut b = Running::start(supervisor(
        &p.api.executions,
        p.api.keys.cipher(),
        "replica-b",
    ));
    within("the fifth order", 60, || async {
        (runs_of(&p, &workflow).await.len() == 5).then_some(())
    })
    .await;
    let holders: i64 = scalar(
        &p,
        "SELECT count(DISTINCT holder) FROM connector_leases WHERE stream LIKE 'orders/%'",
    )
    .await;
    assert!((1..=2).contains(&holders));
    let streams: i64 = scalar(&p, "SELECT count(*) FROM connector_leases").await;
    assert_eq!(streams, 2, "one lease per partition, whoever holds it");
    a.stop().await;
    b.stop().await;
    assert_eq!(runs_of(&p, &workflow).await.len(), 5);
}

#[tokio::test]
async fn us03_sqs_messages_are_deleted_only_after_they_are_in_the_inbox() {
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let sqs = brokers::sqs().await;
    let queue = sqs.create_queue("orders").await;
    let workflow = p.publish("on-order", &on_order("on-order", "orders")).await;
    let namespace = namespace_of(&p).await;
    let ns = p.namespace_id().await;
    // The credentials of the queue are a secret of the namespace.
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/queue-credentials"),
            Some(&json!({"value": json!({"accessKeyId": "AKIDEXAMPLE", "secretAccessKey": "secret-key"}).to_string()})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    store_connector(
        &p.api.executions,
        p.tenant,
        namespace,
        Wanted {
            slug: "orders",
            settings: ConnectorSettings::sqs(&queue, "elasticmq").expect("settings"),
            secret: Some("queue-credentials"),
            default_type: None,
            rate: 1000,
        },
    )
    .await;

    fn attributes(id: &str) -> [(&str, &str); 2] {
        [("eventType", "order.created"), ("eventId", id)]
    }
    sqs.send(&queue, r#"{"order":"ord-1"}"#, &attributes("o-1"))
        .await;
    sqs.send(&queue, r#"{"order":"ord-1"}"#, &attributes("o-1"))
        .await; // delivered twice by the sender
    sqs.send(&queue, r#"{"order":"ord-2"}"#, &attributes("o-2"))
        .await;
    sqs.send(&queue, "this is not json", &attributes("o-3"))
        .await;

    let mut running = Running::start(supervisor(
        &p.api.executions,
        p.api.keys.cipher(),
        "replica-a",
    ));
    within("two runs", 60, || async {
        (runs_of(&p, &workflow).await.len() == 2).then_some(())
    })
    .await;
    within("the queue to be empty", 30, || async {
        (sqs.depth(&queue).await == (0, 0)).then_some(())
    })
    .await;
    assert_eq!(
        scalar(
            &p,
            "SELECT count(*) FROM inbox_events WHERE source = 'orders'"
        )
        .await,
        2
    );
    assert_eq!(
        scalar(&p, "SELECT count(*) FROM connector_dead_letters").await,
        1,
        "the message that is not JSON"
    );
    running.stop().await;

    // While the tenant's backlog is full nothing is taken off the queue: the messages stay with SQS.
    sqlx::query(
        "INSERT INTO inbox_events (tenant_id, namespace_id, source, event_id, event_type, payload, \
         payload_digest, received_at, expires_at, next_attempt_at) \
         SELECT n.tenant_id, n.id, 'filler-source', 'f-' || g, 'a.b', '\\x7b7d'::bytea, \
                decode(repeat('00', 32), 'hex'), clock_timestamp(), clock_timestamp() + interval '1 hour', \
                clock_timestamp() + interval '1 day' \
         FROM namespaces n, generate_series(1, 10000) AS g WHERE n.slug = 'payments' AND n.tenant_id = $1",
    )
    .bind(p.tenant.as_uuid())
    .execute(&p.api.db.pool)
    .await
    .expect("fill the backlog");
    sqs.send(&queue, r#"{"order":"ord-4"}"#, &attributes("o-4"))
        .await;
    let mut again = Running::start(supervisor(
        &p.api.executions,
        p.api.keys.cipher(),
        "replica-a",
    ));
    tokio::time::sleep(Duration::from_secs(4)).await;
    let (visible, in_flight) = sqs.depth(&queue).await;
    assert_eq!(visible + in_flight, 1, "the message is still in the queue");
    assert_eq!(
        scalar(
            &p,
            "SELECT count(*) FROM inbox_events WHERE event_id = 'o-4'"
        )
        .await,
        0
    );
    again.stop().await;
}

#[tokio::test]
async fn us03_nats_messages_are_acknowledged_after_the_event_is_in_the_inbox() {
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let broker = nats::start().await;
    let client = async_nats::connect(broker.url()).await.expect("connect");
    let jetstream = async_nats::jetstream::new(client);
    jetstream
        .create_stream(async_nats::jetstream::stream::Config {
            name: "ORDERS".to_owned(),
            subjects: vec!["orders.>".to_owned()],
            ..Default::default()
        })
        .await
        .expect("stream");
    let workflow = p.publish("on-order", &on_order("on-order", "orders")).await;
    let namespace = namespace_of(&p).await;
    store_connector(
        &p.api.executions,
        p.tenant,
        namespace,
        Wanted {
            slug: "orders",
            settings: ConnectorSettings::nats(
                &[broker.url()],
                "ORDERS",
                "sagat-orders",
                Some("orders.>"),
            )
            .expect("settings"),
            secret: None,
            default_type: None,
            rate: 1000,
        },
    )
    .await;

    for (id, order) in [("o-1", "ord-1"), ("o-2", "ord-2")] {
        let mut headers = async_nats::HeaderMap::new();
        headers.insert("eventType", "order.created");
        headers.insert("Nats-Msg-Id", id);
        jetstream
            .publish_with_headers(
                "orders.created",
                headers,
                format!(r#"{{"order":"{order}"}}"#).into(),
            )
            .await
            .expect("publish")
            .await
            .expect("stored");
    }
    let mut running = Running::start(supervisor(
        &p.api.executions,
        p.api.keys.cipher(),
        "replica-a",
    ));
    within("two runs", 60, || async {
        (runs_of(&p, &workflow).await.len() == 2).then_some(())
    })
    .await;
    // Nothing is left unacknowledged at the server.
    let stream = jetstream.get_stream("ORDERS").await.expect("stream");
    let mut consumer = stream
        .get_consumer::<async_nats::jetstream::consumer::pull::Config>("sagat-orders")
        .await
        .expect("consumer");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let info = consumer.info().await.expect("consumer info");
        if info.num_pending == 0 && info.num_ack_pending == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "messages stayed unacknowledged: {} pending, {} awaiting acknowledgement",
            info.num_pending,
            info.num_ack_pending
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    running.stop().await;
    assert_eq!(
        scalar(
            &p,
            "SELECT count(*) FROM inbox_events WHERE source = 'orders'"
        )
        .await,
        2
    );
}

#[tokio::test]
async fn us03_sqs_batches_far_larger_than_a_webhook_answer_are_read_and_not_a_wedge() {
    let p = platform().await;
    let _engine = engine(&p, &[]).await;
    let sqs = brokers::sqs().await;
    let queue = sqs.create_queue("big-orders").await;
    let workflow = p
        .publish("on-big-order", &on_order("on-big-order", "big-orders"))
        .await;
    let namespace = namespace_of(&p).await;
    let ns = p.namespace_id().await;
    let (status, body) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/secrets/queue-credentials"),
            Some(&json!({"value": json!({"accessKeyId": "AKIDEXAMPLE", "secretAccessKey": "secret-key"}).to_string()})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    store_connector(
        &p.api.executions,
        p.tenant,
        namespace,
        Wanted {
            slug: "big-orders",
            settings: ConnectorSettings::sqs(&queue, "elasticmq").expect("settings"),
            secret: Some("queue-credentials"),
            default_type: None,
            rate: 1000,
        },
    )
    .await;
    // Ten messages of 20 KiB: one receive carries over 200 KiB, and every quotation mark is six
    // characters in the XML it comes in.
    for i in 0..10 {
        let filler = "\"".repeat(20_000);
        sqs.send(
            &queue,
            &format!(
                r#"{{"order":"ord-{i}","filler":"{}"}}"#,
                filler.replace('"', "\\\"")
            ),
            &[
                ("eventType", "order.created"),
                ("eventId", &format!("big-{i}")),
            ],
        )
        .await;
    }
    let mut running = Running::start(supervisor(
        &p.api.executions,
        p.api.keys.cipher(),
        "replica-a",
    ));
    within("ten runs", 60, || async {
        (runs_of(&p, &workflow).await.len() == 10).then_some(())
    })
    .await;
    within("the queue to be empty", 30, || async {
        (sqs.depth(&queue).await == (0, 0)).then_some(())
    })
    .await;
    running.stop().await;
}

#[tokio::test]
async fn us03_kafka_a_paced_batch_longer_than_the_lease_keeps_renewing_it() {
    use sagat_infra_connectors::credentials::Credentials;
    let p = platform().await;
    let kafka = brokers::kafka().await;
    kafka.create_topic("slow-orders", 1).await;
    let namespace = namespace_of(&p).await;
    let connector = store_connector(
        &p.api.executions,
        p.tenant,
        namespace,
        Wanted {
            slug: "slow-orders",
            settings: ConnectorSettings::kafka(
                std::slice::from_ref(&kafka.bootstrap),
                "slow-orders",
                StartFrom::Earliest,
            )
            .expect("settings"),
            secret: None,
            default_type: Some("order.created"),
            // Eight messages a second: forty of them take five seconds, and the lease lasts three.
            rate: 8,
        },
    )
    .await;
    kafka
        .produce(
            "slow-orders",
            0,
            (0..40)
                .map(|n| {
                    record(
                        &[("eventId", &format!("slow-{n}"))],
                        &format!(r#"{{"n":{n}}}"#),
                    )
                })
                .collect(),
        )
        .await;

    let ingest = Arc::new(sagat_infra_connectors::ingest::Ingest::new(
        Arc::new(p.api.executions.clone()),
        connector,
        Arc::new(sagat_infra_ir::JsonEvaluator),
    ));
    let (quit, stop) = tokio::sync::watch::channel(false);
    let reader = tokio::spawn(sagat_infra_connectors::kafka::run_with(
        ingest,
        Arc::new(p.api.executions.clone()),
        Credentials::none(),
        "replica-a".to_owned(),
        stop,
        sagat_infra_connectors::kafka::Timing {
            lease: Duration::from_secs(3),
        },
    ));

    // While the batch is being taken in, the lease keeps moving forward (a lease that was taken once
    // and left would be over after three seconds; at five seconds the batch is not).
    within("the batch to start", 30, || async {
        (scalar(&p, "SELECT count(*) FROM inbox_events").await > 3).then_some(())
    })
    .await;
    let early: time::OffsetDateTime = sqlx::query_scalar("SELECT expires_at FROM connector_leases")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("lease");
    tokio::time::sleep(Duration::from_millis(2500)).await;
    let later: time::OffsetDateTime = sqlx::query_scalar("SELECT expires_at FROM connector_leases")
        .fetch_one(&p.api.db.pool)
        .await
        .expect("lease");
    assert!(
        later > early,
        "the lease was renewed in the middle of the batch"
    );
    within("all forty", 30, || async {
        (scalar(&p, "SELECT count(*) FROM inbox_events").await == 40).then_some(())
    })
    .await;
    let _ = quit.send(true);
    let _ = reader.await;
}
