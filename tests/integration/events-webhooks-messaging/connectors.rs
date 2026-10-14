//! US03 without a broker: what a connector does with the messages it is given, how it is administered,
//! and how the position of a consumer moves with them (FR-004-005, FR-004-008).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use sagat_application::connectors::{
    Checkpoint, ConnectorChoices, ConnectorRecord, ConnectorRuntime, RawMessage,
};
use sagat_application::error::{AppError, ConflictKind};
use sagat_domain::connectors::{ConnectorSettings, ConnectorStatus, DeadLetter, StartFrom};
use sagat_domain::identity::Role;
use sagat_domain::quota::QuotaResource;
use sagat_domain::workflow::EventType;
use sagat_infra_connectors::ingest::{Ingest, Outcome};
use sagat_infra_ir::JsonEvaluator;

use crate::support::engine::{World, ctx, world};
use crate::support::slug;
use crate::{SECRET, SECRET_NAME, settings};

fn kafka(topic: &str) -> ConnectorSettings {
    ConnectorSettings::kafka(
        &["kafka.example.com:9092".to_owned()],
        topic,
        StartFrom::Earliest,
    )
    .expect("settings")
}

fn choices(settings: ConnectorSettings) -> ConnectorChoices {
    ConnectorChoices {
        settings,
        secret_name: None,
        default_event_type: None,
        rate_per_second: None,
        status: ConnectorStatus::Active,
    }
}

/// A world whose namespace may reach a plaintext Kafka broker at `kafka.example.com:9092`.
async fn allowing_kafka() -> World {
    let w = world().await;
    w.egress
        .set(
            &w.admin,
            w.namespace.id(),
            &["kafka.example.com".to_owned()],
            &[9092],
            true,
            &ctx(),
        )
        .await
        .expect("policy");
    w
}

async fn connector(w: &World, name: &str, default_type: Option<&str>) -> ConnectorRecord {
    let mut c = choices(kafka("orders"));
    c.default_event_type = default_type.map(|t| EventType::parse(t).expect("type"));
    w.connectors
        .create(&w.admin, w.namespace.id(), slug(name), c, &ctx())
        .await
        .expect("connector")
}

fn ingest(w: &World, connector: &ConnectorRecord) -> Ingest<sagat_infra_postgres::PgRunStore> {
    Ingest::new(
        Arc::new(w.store.clone()),
        connector.clone(),
        Arc::new(JsonEvaluator),
    )
}

fn message(position: &str, headers: &[(&str, &str)], body: &str) -> RawMessage {
    RawMessage {
        position: position.to_owned(),
        fallback_id: position.to_owned(),
        headers: headers
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect::<BTreeMap<_, _>>(),
        body: body.as_bytes().to_vec(),
    }
}

async fn inbox(w: &World) -> Vec<(String, String, String)> {
    sqlx::query_as("SELECT source, event_id, event_type FROM inbox_events ORDER BY event_id")
        .fetch_all(&w.db.pool)
        .await
        .expect("inbox")
}

async fn offset(w: &World, c: &ConnectorRecord, stream: &str) -> Option<i64> {
    w.store.checkpoint(c.id, stream).await.expect("offset")
}

#[tokio::test]
async fn a_message_becomes_an_event_under_the_slug_of_the_connector_and_a_redelivery_is_a_duplicate()
 {
    let w = allowing_kafka().await;
    let c = connector(&w, "orders", Some("order.created")).await;
    let ingest = ingest(&w, &c);

    let raw = message(
        "orders/0/41",
        &[
            ("eventId", "ord-7"),
            ("subject", "orders/7"),
            ("correlationKey", "order-7"),
        ],
        r#"{"amount":10}"#,
    );
    assert_eq!(ingest.ingest(&raw, None).await, Outcome::Admitted);
    assert_eq!(
        ingest.ingest(&raw, None).await,
        Outcome::Duplicate,
        "a redelivery is recognised"
    );
    assert_eq!(
        inbox(&w).await,
        vec![(
            "orders".to_owned(),
            "ord-7".to_owned(),
            "order.created".to_owned()
        )],
        "the connector's slug is the source, the default type is the type"
    );

    // The broker's own identity is the id when the message names none, and a type in the message wins.
    let plain = message(
        "orders/0/42",
        &[("eventType", "order.cancelled")],
        r#"{"n":1}"#,
    );
    assert_eq!(ingest.ingest(&plain, None).await, Outcome::Admitted);
    let row: (String, String) = sqlx::query_as(
        "SELECT event_id, event_type FROM inbox_events WHERE event_id = 'orders/0/42'",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("row");
    assert_eq!(
        row,
        ("orders/0/42".to_owned(), "order.cancelled".to_owned())
    );

    // What the connector read is shown on it.
    ingest.flush(true).await;
    let shown = w.connectors.get(&w.admin, c.id).await.expect("connector");
    assert_eq!(shown.health.consumed, 3, "two events and a repeat");
    assert!(shown.health.last_message_at.is_some());
}

#[tokio::test]
async fn a_message_that_cannot_be_an_event_is_set_aside_with_why_and_never_blocks_the_ones_behind_it()
 {
    let w = allowing_kafka().await;
    let c = connector(&w, "orders", None).await;
    let ingest = ingest(&w, &c);
    let stream = "orders/0";
    let mut next = 0;
    let feed = |raw: RawMessage, expected: i64| {
        let checkpoint = Checkpoint {
            stream: stream.to_owned(),
            next_offset: expected + 1,
        };
        let ingest = &ingest;
        async move { ingest.ingest(&raw, Some((&checkpoint, expected))).await }
    };

    let cases = [
        (message("orders/0/0", &[], r#"{"a":1}"#), DeadLetter::NoType),
        (
            message("orders/0/1", &[("eventType", "order.created")], "[1,2]"),
            DeadLetter::NotAnObject,
        ),
        (
            message(
                "orders/0/2",
                &[("eventType", "order.created")],
                "not json at all",
            ),
            DeadLetter::NotAnObject,
        ),
        (
            message("orders/0/3", &[("eventType", "Not A Type!")], "{}"),
            DeadLetter::BadEnvelope,
        ),
        (
            message(
                "orders/0/4",
                &[("eventType", "a.b"), ("eventId", "has space")],
                "{}",
            ),
            DeadLetter::BadEnvelope,
        ),
        (
            message(
                "orders/0/5",
                &[("eventType", "a.b")],
                &format!(r#"{{"pad":"{}"}}"#, "x".repeat(250_000)),
            ),
            DeadLetter::TooLarge,
        ),
    ];
    for (raw, reason) in cases {
        assert_eq!(
            feed(raw, next).await,
            Outcome::SetAside(reason),
            "{reason:?}"
        );
        next += 1;
    }
    // The one behind them goes through, and the position is past all of them.
    let good = message(
        "orders/0/6",
        &[("eventType", "order.created")],
        r#"{"ok":true}"#,
    );
    assert_eq!(feed(good, next).await, Outcome::Admitted);
    assert_eq!(offset(&w, &c, stream).await, Some(7));
    assert_eq!(inbox(&w).await.len(), 1);

    let letters = w
        .connectors
        .dead_letters(&w.admin, c.id, None, 50)
        .await
        .expect("letters");
    assert_eq!(letters.len(), 6);
    let reasons: Vec<_> = letters.iter().map(|l| l.reason).collect();
    assert!(reasons.contains(&DeadLetter::NoType) && reasons.contains(&DeadLetter::TooLarge));
    let big = letters
        .iter()
        .find(|l| l.reason == DeadLetter::TooLarge)
        .expect("big");
    assert!(
        big.truncated && big.body.len() <= 64 * 1024,
        "what is kept is bounded"
    );

    // The same message set aside twice (a redelivery) is one dead letter.
    let again = message("orders/0/0", &[], r#"{"a":1}"#);
    assert_eq!(
        ingest.ingest(&again, None).await,
        Outcome::Duplicate,
        "done with, like any message the inbox already had"
    );
    assert_eq!(
        w.connectors
            .dead_letters(&w.admin, c.id, None, 50)
            .await
            .expect("letters")
            .len(),
        6
    );
}

#[tokio::test]
async fn the_position_moves_with_the_event_and_a_consumer_that_lost_its_place_is_stopped() {
    let w = allowing_kafka().await;
    let c = connector(&w, "orders", Some("order.created")).await;
    let ingest = ingest(&w, &c);
    let at = |from: i64| Checkpoint {
        stream: "orders/1".to_owned(),
        next_offset: from + 1,
    };

    // The first message of a partition has no row to move from and creates it.
    assert_eq!(
        ingest
            .ingest(&message("orders/1/10", &[], "{}"), Some((&at(10), 10)))
            .await,
        Outcome::Admitted
    );
    assert_eq!(offset(&w, &c, "orders/1").await, Some(11));
    // A redelivery of the same message moves nothing backwards: it is a duplicate at the old place,
    // and a consumer that still thinks it is at 10 is no longer the one that reads this partition.
    assert_eq!(
        ingest
            .ingest(&message("orders/1/10", &[], "{}"), Some((&at(10), 10)))
            .await,
        Outcome::Superseded
    );
    assert_eq!(
        offset(&w, &c, "orders/1").await,
        Some(11),
        "and the position did not move"
    );
    // The consumer that is at the right place carries on.
    assert_eq!(
        ingest
            .ingest(&message("orders/1/11", &[], "{}"), Some((&at(11), 11)))
            .await,
        Outcome::Admitted
    );
    assert_eq!(offset(&w, &c, "orders/1").await, Some(12));

    // A stale consumer admits nothing: the event is not in the inbox either.
    let before = inbox(&w).await.len();
    assert_eq!(
        ingest
            .ingest(&message("orders/1/99", &[], "{}"), Some((&at(99), 3)))
            .await,
        Outcome::Superseded
    );
    assert_eq!(inbox(&w).await.len(), before);
}

#[tokio::test]
async fn a_full_backlog_leaves_the_message_with_the_broker_and_the_position_where_it_was() {
    let w = allowing_kafka().await;
    let c = connector(&w, "orders", Some("order.created")).await;
    let ingest = ingest(&w, &c);
    sqlx::query(
        "INSERT INTO inbox_events (tenant_id, namespace_id, source, event_id, event_type, payload, \
         payload_digest, received_at, expires_at, next_attempt_at) \
         SELECT $1, $2, 'filler-source', 'f-' || n, 'a.b', '\x7b7d'::bytea, decode(repeat('00', 32), 'hex'), \
                clock_timestamp(), clock_timestamp() + interval '1 hour', clock_timestamp() + interval '1 day' \
         FROM generate_series(1, 10000) AS n",
    )
    .bind(c.tenant_id.as_uuid())
    .bind(c.namespace_id.as_uuid())
    .execute(&w.db.pool)
    .await
    .expect("fill the backlog");

    let checkpoint = Checkpoint {
        stream: "orders/0".to_owned(),
        next_offset: 1,
    };
    let outcome = ingest
        .ingest(&message("orders/0/0", &[], "{}"), Some((&checkpoint, 0)))
        .await;
    assert_eq!(outcome, Outcome::Backlogged);
    assert!(
        !outcome.done(),
        "the broker is not told the message is done"
    );
    assert_eq!(
        offset(&w, &c, "orders/0").await,
        None,
        "and the position stays"
    );

    sqlx::query("UPDATE inbox_events SET status = 'DISPATCHED', dispatched_at = clock_timestamp()")
        .execute(&w.db.pool)
        .await
        .expect("the matcher catches up");
    assert_eq!(
        ingest
            .ingest(&message("orders/0/0", &[], "{}"), Some((&checkpoint, 0)))
            .await,
        Outcome::Admitted
    );
}

#[tokio::test]
async fn a_dead_letter_can_be_tried_again_when_the_connector_can_now_take_it_or_discarded() {
    let w = allowing_kafka().await;
    let c = connector(&w, "orders", None).await;
    let ingest = ingest(&w, &c);
    // No type in the message and none on the connector.
    assert_eq!(
        ingest
            .ingest(
                &message("orders/0/0", &[("correlationKey", "order-1")], r#"{"a":1}"#),
                None
            )
            .await,
        Outcome::SetAside(DeadLetter::NoType)
    );
    ingest
        .ingest(&message("orders/0/1", &[("eventType", "a.b")], "[]"), None)
        .await;
    let letters = w
        .connectors
        .dead_letters(&w.admin, c.id, None, 10)
        .await
        .expect("letters");
    assert_eq!(letters.len(), 2);
    let untyped = letters
        .iter()
        .find(|l| l.reason == DeadLetter::NoType)
        .expect("untyped")
        .id;
    let array = letters
        .iter()
        .find(|l| l.reason == DeadLetter::NotAnObject)
        .expect("array")
        .id;

    // Still not an event: it stays, with the reason it fails for now.
    assert_eq!(
        w.connectors
            .redrive(&w.admin, c.id, untyped, &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotAllowed(
            "the message names no event type and the connector has no default"
        ))
    );
    assert_eq!(
        w.connectors
            .redrive(&w.admin, c.id, array, &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotAllowed("the message is not a JSON object"))
    );

    // The administrator gives the connector a default type: now the first one can be taken.
    let mut with_default = choices(kafka("orders"));
    with_default.default_event_type = Some(EventType::parse("order.created").expect("type"));
    w.connectors
        .update(&w.admin, c.id, with_default, &ctx())
        .await
        .expect("update");
    let key = w
        .connectors
        .redrive(&w.admin, c.id, untyped, &ctx())
        .await
        .expect("redriven");
    assert_eq!(
        (key.source.as_str(), key.event_id.as_str()),
        ("orders", "orders/0/0")
    );
    assert_eq!(inbox(&w).await.len(), 1);
    assert_eq!(
        w.connectors
            .dead_letters(&w.admin, c.id, None, 10)
            .await
            .expect("letters")
            .len(),
        1
    );
    let kept: (String,) = sqlx::query_as("SELECT correlation_key FROM inbox_events")
        .fetch_one(&w.db.pool)
        .await
        .expect("key");
    assert_eq!(
        kept.0, "order-1",
        "what the message carried travels with it"
    );

    // The other is discarded by a person who may, and not by one who may only read.
    let viewer = w.member("viewer-1", Role::Viewer).await;
    assert!(matches!(
        w.connectors.discard(&viewer, c.id, array, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    assert!(matches!(
        w.connectors.redrive(&viewer, c.id, array, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    w.connectors
        .discard(&w.admin, c.id, array, &ctx())
        .await
        .expect("discarded");
    assert_eq!(
        w.connectors.discard(&w.admin, c.id, array, &ctx()).await,
        Err(AppError::NotFound)
    );
    assert!(
        w.connectors
            .dead_letters(&w.admin, c.id, None, 10)
            .await
            .expect("letters")
            .is_empty()
    );
}

#[tokio::test]
async fn a_connector_is_administered_within_the_egress_policy_the_quota_and_the_names_of_the_namespace()
 {
    let w = world().await;
    let create = |name: &str, c: ConnectorChoices| {
        let w = &w;
        let name = name.to_owned();
        async move {
            w.connectors
                .create(&w.admin, w.namespace.id(), slug(&name), c, &ctx())
                .await
        }
    };

    // Default-deny: a broker the policy does not name is out of reach, and a plaintext one needs the
    // policy to allow plain connections.
    assert_eq!(
        create("orders", choices(kafka("orders"))).await.map(|_| ()),
        Err(AppError::NotAllowed(
            "the egress policy of the namespace does not allow a broker of the connector"
        ))
    );
    w.egress
        .set(
            &w.admin,
            w.namespace.id(),
            &["kafka.example.com".to_owned()],
            &[9092],
            false,
            &ctx(),
        )
        .await
        .expect("policy");
    assert!(
        matches!(
            create("orders", choices(kafka("orders"))).await,
            Err(AppError::NotAllowed(_))
        ),
        "plain HTTP is not allowed, so a plaintext broker is not"
    );
    let encrypted = ConnectorSettings::kafka(
        &["tls://kafka.example.com:9092".to_owned()],
        "orders",
        StartFrom::Latest,
    )
    .expect("settings");
    let made = create("orders", choices(encrypted.clone()))
        .await
        .expect("an encrypted broker is allowed");
    assert_eq!(made.rate_per_second, 200, "the default ingest rate");

    // The slug is the source of its events: reserved names, repeats and the names of endpoints are out.
    assert!(matches!(
        create("api", choices(encrypted.clone())).await,
        Err(AppError::NotAllowed(_))
    ));
    assert_eq!(
        create("orders", choices(encrypted.clone()))
            .await
            .map(|_| ()),
        Err(AppError::Conflict(ConflictKind::ConnectorSlug))
    );
    w.put_secret(SECRET_NAME, SECRET).await;
    w.events
        .create_endpoint(
            &w.admin,
            w.namespace.id(),
            slug("billing"),
            settings(),
            &ctx(),
        )
        .await
        .expect("an endpoint");
    assert_eq!(
        create("billing", choices(encrypted.clone()))
            .await
            .map(|_| ()),
        Err(AppError::Conflict(ConflictKind::ConnectorSlug)),
        "an endpoint and a connector never share a source"
    );
    let c = create("shipping", choices(encrypted.clone()))
        .await
        .expect("another connector");
    assert_eq!(
        w.events
            .create_endpoint(
                &w.admin,
                w.namespace.id(),
                slug("shipping"),
                settings(),
                &ctx()
            )
            .await
            .map(|_| ()),
        Err(AppError::Conflict(ConflictKind::EventEndpointSlug)),
        "nor the other way round"
    );

    // A rate out of range, a kind that changes, a secret that is not usable.
    let mut fast = choices(encrypted.clone());
    fast.rate_per_second = Some(100_000);
    assert!(matches!(
        create("fast", fast).await,
        Err(AppError::NotAllowed(_))
    ));
    let mut nope = choices(encrypted.clone());
    nope.secret_name =
        Some(sagat_domain::secrets::SecretName::parse("no-such-secret").expect("name"));
    assert!(matches!(
        create("secretive", nope).await,
        Err(AppError::SecretUnavailable { .. })
    ));
    let sqs =
        ConnectorSettings::sqs("https://kafka.example.com/123/orders", "us-east-1").expect("sqs");
    assert_eq!(
        w.connectors
            .update(&w.admin, c.id, choices(sqs), &ctx())
            .await
            .map(|_| ()),
        Err(AppError::NotAllowed(
            "the kind of a connector does not change; make another"
        ))
    );

    // The quota counts connectors per namespace, and the roles decide who may do what.
    w.quotas
        .set(
            &w.admin,
            Some(w.namespace.id()),
            QuotaResource::Connectors,
            2,
            &ctx(),
        )
        .await
        .expect("limit");
    assert_eq!(
        create("third", choices(encrypted.clone()))
            .await
            .map(|_| ()),
        Err(AppError::QuotaExceeded(
            "message connectors in the namespace"
        ))
    );
    let viewer = w.member("viewer-2", Role::Viewer).await;
    assert!(w.connectors.get(&viewer, c.id).await.is_ok());
    assert!(matches!(
        w.connectors.delete(&viewer, c.id, &ctx()).await,
        Err(AppError::Forbidden { .. })
    ));
    // Another tenant sees none of it.
    assert_eq!(
        w.connectors.get(&w.other_admin, c.id).await.map(|_| ()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.connectors.delete(&w.other_admin, c.id, &ctx()).await,
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.connectors
            .list(&w.other_admin, w.namespace.id(), &ctx())
            .await
            .map(|v| v.len()),
        Err(AppError::NotFound)
    );
    assert_eq!(
        w.connectors
            .list(&w.admin, w.namespace.id(), &ctx())
            .await
            .map(|v| v.len()),
        Ok(2)
    );

    // Deleting takes the connector's positions and dead letters with it.
    let checkpoint = Checkpoint {
        stream: "orders/0".to_owned(),
        next_offset: 1,
    };
    let mut live = choices(encrypted);
    live.default_event_type = Some(EventType::parse("a.b").expect("type"));
    w.connectors
        .update(&w.admin, c.id, live, &ctx())
        .await
        .expect("update");
    let record = w.connectors.get(&w.admin, c.id).await.expect("get");
    let ingest = ingest(&w, &record);
    ingest
        .ingest(&message("orders/0/0", &[], "{}"), Some((&checkpoint, 0)))
        .await;
    ingest
        .ingest(&message("orders/0/1", &[], "nope"), None)
        .await;
    w.connectors
        .delete(&w.admin, c.id, &ctx())
        .await
        .expect("deleted");
    for table in ["connector_offsets", "connector_dead_letters"] {
        let left: i64 =
            sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
                .fetch_one(&w.db.pool)
                .await
                .expect("count");
        assert_eq!(left, 0, "{table}");
    }
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE action IN ('connector.created', 'connector.changed', 'connector.deleted')",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("audit");
    assert!(audited >= 4);
}

#[tokio::test]
async fn only_one_replica_holds_the_lease_on_a_partition_until_it_runs_out() {
    let w = allowing_kafka().await;
    let c = connector(&w, "orders", None).await;
    let ttl = Duration::from_secs(30);
    assert_eq!(
        w.store.lease(c.id, "orders/0", "replica-a", ttl).await,
        Ok(true)
    );
    assert_eq!(
        w.store.lease(c.id, "orders/0", "replica-b", ttl).await,
        Ok(false),
        "taken"
    );
    assert_eq!(
        w.store.lease(c.id, "orders/0", "replica-a", ttl).await,
        Ok(true),
        "renewed by its holder"
    );
    assert_eq!(
        w.store.lease(c.id, "orders/1", "replica-b", ttl).await,
        Ok(true),
        "another partition is free"
    );
    sqlx::query("UPDATE connector_leases SET expires_at = clock_timestamp() - interval '1 second'")
        .execute(&w.db.pool)
        .await
        .expect("the holder dies");
    assert_eq!(
        w.store.lease(c.id, "orders/0", "replica-b", ttl).await,
        Ok(true),
        "taken over"
    );
    assert_eq!(
        w.store.lease(c.id, "orders/0", "replica-a", ttl).await,
        Ok(false)
    );
}

/// A reader of an SQS queue that nobody listens at.
async fn sqs_reader_of_nothing(
    w: &World,
    timing: sagat_infra_connectors::sqs::Timing,
) -> sagat_infra_connectors::nats::Stopped {
    use crate::support::brokers::{Wanted, store_connector};
    use crate::support::receiver::OpenFactory;
    use sagat_infra_connectors::credentials::Credentials;
    use sagat_infra_connectors::webhook::ClientFactory as _;

    // A port nobody listens on: every call to the queue fails.
    let closed = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        drop(listener);
        format!("http://127.0.0.1:{port}/queue")
    };
    let record = store_connector(
        &w.store,
        w.admin.tenant_id(),
        w.namespace.id(),
        Wanted {
            slug: "orders",
            settings: ConnectorSettings::sqs(&closed, "elasticmq").expect("settings"),
            secret: None,
            default_type: Some("order.created"),
            rate: 100,
        },
    )
    .await;
    let ingest = Arc::new(Ingest::new(
        Arc::new(w.store.clone()),
        record,
        Arc::new(JsonEvaluator),
    ));
    let client = Arc::new(
        OpenFactory
            .client(&sagat_domain::egress::EgressPolicy::deny_all())
            .expect("client"),
    );
    let (_keep, stop) = tokio::sync::watch::channel(false);
    tokio::time::timeout(
        Duration::from_secs(20),
        sagat_infra_connectors::sqs::run_with(ingest, client, Credentials::none(), stop, timing),
    )
    .await
    .expect("the reader gives up instead of trying for ever")
}

#[tokio::test]
async fn a_queue_reader_that_keeps_failing_gives_up_so_that_it_is_started_again_with_fresh_credentials()
 {
    let w = world().await;
    let stopped = sqs_reader_of_nothing(
        &w,
        sagat_infra_connectors::sqs::Timing {
            pause: Duration::from_millis(20),
            restart_after: 3,
        },
    )
    .await;
    // (Credentials and the policy of the namespace are read when a reader starts; one that is told
    // the key has expired or the policy has changed only learns it by starting again.)
    assert!(
        matches!(stopped, sagat_infra_connectors::nats::Stopped::Failed(_)),
        "{stopped:?}"
    );
}

#[tokio::test]
async fn the_dead_letters_kept_are_the_first_ones_and_a_redelivered_one_is_not_counted_again() {
    let w = allowing_kafka().await;
    let c = connector(&w, "orders", None).await;
    let ingest = ingest(&w, &c);
    let poison = |n: usize| message(&format!("orders/0/{n}"), &[], r#"{"a":1}"#);

    // One poison message, delivered twice by a broker that lost the acknowledgement.
    assert_eq!(
        ingest.ingest(&poison(0), None).await,
        Outcome::SetAside(DeadLetter::NoType)
    );
    assert_eq!(
        ingest.ingest(&poison(0), None).await,
        Outcome::Duplicate,
        "it is already set aside; the broker may be told it is done with it"
    );
    ingest.flush(true).await;
    let shown = w.connectors.get(&w.admin, c.id).await.expect("connector");
    assert_eq!(shown.health.dead_lettered, 1, "counted once");

    // A flood: the first thousand are what stays (the first ones show what went wrong first), the
    // rest are acknowledged and counted but not kept.
    for n in 1..1_010 {
        assert!(matches!(
            ingest.ingest(&poison(n), None).await,
            Outcome::SetAside(_)
        ));
    }
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM connector_dead_letters")
        .fetch_one(&w.db.pool)
        .await
        .expect("count");
    assert_eq!(kept, 1_000);
    let positions: Vec<String> = sqlx::query_scalar(
        "SELECT position FROM connector_dead_letters WHERE position IN \
         ('orders/0/0', 'orders/0/999', 'orders/0/1000', 'orders/0/1009') ORDER BY position",
    )
    .fetch_all(&w.db.pool)
    .await
    .expect("positions");
    assert_eq!(
        positions,
        ["orders/0/0", "orders/0/999"],
        "the oldest are kept, the ones past the bound are not"
    );
    ingest.flush(true).await;
    let shown = w.connectors.get(&w.admin, c.id).await.expect("connector");
    assert_eq!(
        shown.health.dead_lettered, 1_010,
        "every message set aside counted once, kept or not"
    );
}

/// A connector of `settings` for the readers that are tried directly.
async fn stored_connector(w: &World, name: &str, settings: ConnectorSettings) -> ConnectorRecord {
    use crate::support::brokers::{Wanted, store_connector};
    store_connector(
        &w.store,
        w.admin.tenant_id(),
        w.namespace.id(),
        Wanted {
            slug: name,
            settings,
            secret: None,
            default_type: Some("order.created"),
            rate: 100,
        },
    )
    .await
}

#[tokio::test]
async fn a_password_is_not_sent_over_a_connection_without_tls() {
    use sagat_infra_connectors::credentials::Credentials;
    use sagat_infra_connectors::nats::Stopped;
    let w = world().await;
    let (_keep, stop) = tokio::sync::watch::channel(false);

    // Kafka, with SASL, to a plaintext broker. (Nothing listens there: the reader must refuse to go.)
    let kafka = stored_connector(
        &w,
        "kafka-orders",
        ConnectorSettings::kafka(&["127.0.0.1:1".to_owned()], "orders", StartFrom::Earliest)
            .expect("settings"),
    )
    .await;
    let sasl = Credentials::parse(br#"{"username":"u","password":"p"}"#).expect("credentials");
    let stopped = tokio::time::timeout(
        Duration::from_secs(10),
        sagat_infra_connectors::kafka::run(
            Arc::new(ingest(&w, &kafka)),
            Arc::new(w.store.clone()),
            sasl,
            "replica-a".to_owned(),
            stop.clone(),
        ),
    )
    .await
    .expect("the reader refuses at once; it does not try to connect with a password");
    assert!(
        matches!(stopped, Stopped::Failed(why) if why.contains("TLS")),
        "{stopped:?}"
    );

    // NATS, with a user and a password, to a plaintext server.
    let nats = stored_connector(
        &w,
        "nats-orders",
        ConnectorSettings::nats(&["127.0.0.1:1".to_owned()], "ORDERS", "sagat", None)
            .expect("settings"),
    )
    .await;
    for document in [
        br#"{"user":"u","password":"p"}"#.as_slice(),
        br#"{"token":"t"}"#.as_slice(),
    ] {
        let stopped = tokio::time::timeout(
            Duration::from_secs(10),
            sagat_infra_connectors::nats::run(
                Arc::new(ingest(&w, &nats)),
                Credentials::parse(document).expect("credentials"),
                stop.clone(),
            ),
        )
        .await
        .expect("the reader refuses at once; it does not try to connect with a password");
        assert!(
            matches!(stopped, Stopped::Failed(why) if why.contains("TLS")),
            "{stopped:?}"
        );
    }
}

#[tokio::test]
async fn durable_source_rate_refusal_neither_debits_namespace_nor_acknowledges_or_moves_offset() {
    let w = allowing_kafka().await;
    let mut c = connector(&w, "orders", Some("order.created")).await;
    c.rate_per_second = 1;
    sqlx::query("UPDATE event_connectors SET rate_per_second=1 WHERE id=$1")
        .bind(c.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("source rate");
    let ingest = ingest(&w, &c);
    assert_eq!(
        ingest.ingest(&message("prime", &[], "{}"), None).await,
        Outcome::Admitted
    );
    // Simulate a bucket whose database clock was ahead: a backwards clock must not mint tokens.
    sqlx::query("UPDATE quota_rate_buckets SET microtokens=0, updated_at=clock_timestamp()+interval '1 day' WHERE source_kind='CONNECTOR'")
        .execute(&w.db.pool).await.expect("deplete source bucket");
    let before: (i64, time::OffsetDateTime) = sqlx::query_as(
        "SELECT microtokens,updated_at FROM quota_rate_buckets WHERE resource='events_per_second' AND source_kind='NAMESPACE'",
    ).fetch_one(&w.db.pool).await.expect("namespace bucket");
    let checkpoint = Checkpoint {
        stream: "orders/0".into(),
        next_offset: 1,
    };
    let raw = message("orders/0/0", &[], "{}");
    let outcome = ingest.ingest(&raw, Some((&checkpoint, 0))).await;
    assert_eq!(outcome, Outcome::Backlogged);
    assert!(
        !outcome.done(),
        "no broker acknowledgement before durable admission"
    );
    assert_eq!(offset(&w, &c, "orders/0").await, None);
    let after: (i64, time::OffsetDateTime) = sqlx::query_as(
        "SELECT microtokens,updated_at FROM quota_rate_buckets WHERE resource='events_per_second' AND source_kind='NAMESPACE'",
    ).fetch_one(&w.db.pool).await.expect("namespace bucket");
    assert_eq!(
        before, after,
        "source refusal does not debit the namespace bucket"
    );
    sqlx::query("UPDATE quota_rate_buckets SET updated_at=clock_timestamp()-interval '2 seconds' WHERE source_kind='CONNECTOR'")
        .execute(&w.db.pool).await.expect("refill source");
    assert_eq!(
        ingest.ingest(&raw, Some((&checkpoint, 0))).await,
        Outcome::Admitted
    );
    assert_eq!(offset(&w, &c, "orders/0").await, Some(1));
}

#[tokio::test]
async fn a_full_log_budget_preserves_connector_progress_and_explicit_error_truncation() {
    use sagat_application::connectors::Progress;
    let w = allowing_kafka().await;
    let connector = connector(&w, "log-budget", None).await;
    w.quotas
        .set(&w.admin, None, QuotaResource::TenantLogBytes, 0, &ctx())
        .await
        .expect("no logs");
    sqlx::query("UPDATE quota_accounting_control SET log_enforcement_enabled=true")
        .execute(&w.db.pool)
        .await
        .expect("enable log guard");
    w.store
        .progress(
            connector.id,
            &Progress {
                consumed: 1,
                dead_lettered: 0,
                last_message_at: None,
                error: Some(Some("échec diagnostic".into())),
            },
        )
        .await
        .expect("progress");
    let row: (i64, String, bool) = sqlx::query_as(
        "SELECT consumed_total,last_error,last_error_truncated FROM event_connectors WHERE id=$1",
    )
    .bind(connector.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("connector");
    assert_eq!(row, (1, String::new(), true));
    w.store
        .progress(
            connector.id,
            &Progress {
                consumed: 1,
                dead_lettered: 0,
                last_message_at: None,
                error: Some(None),
            },
        )
        .await
        .expect("clear error");
    let row: (i64, Option<String>, bool) = sqlx::query_as(
        "SELECT consumed_total,last_error,last_error_truncated FROM event_connectors WHERE id=$1",
    )
    .bind(connector.id.as_uuid())
    .fetch_one(&w.db.pool)
    .await
    .expect("connector");
    assert_eq!(row, (2, None, false));
}
