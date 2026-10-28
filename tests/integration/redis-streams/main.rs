//! Real disposable Redis proof for selectable internal notifications.
#![allow(clippy::expect_used, clippy::panic)]
mod api;
mod bootstrap;
mod callbacks;
#[path = "../../support/mod.rs"]
mod support;
mod tls;
use sagat_application::entropy::SystemEntropy;
use sagat_application::maintenance::{EventPublisher, OutboxStore};
use sagat_infra_connectors::redis::{RedisConfig, RedisPublisher};
use std::time::Duration;
use testcontainers_modules::testcontainers::{
    ContainerAsync, GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

async fn redis() -> (ContainerAsync<GenericImage>, String, redis::Client) {
    let image = GenericImage::new("redis", "8.6.8-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_cmd([
            "redis-server",
            "--appendonly",
            "yes",
            "--appendfsync",
            "always",
        ])
        .with_host_config_modifier(|config| {
            config.memory = Some(128 * 1024 * 1024);
            config.pids_limit = Some(128);
            if let Some(bindings) = config.port_bindings.as_mut() {
                for entries in bindings.values_mut().flatten() {
                    for binding in entries {
                        binding.host_ip = Some("127.0.0.1".to_owned());
                    }
                }
            }
        });
    let container =
        support::bounded::within("start owned Redis", Duration::from_secs(90), image.start())
            .await
            .expect("Redis fixture");
    let port = container
        .get_host_port_ipv4(6379)
        .await
        .expect("mapped Redis");
    let url = format!("redis://127.0.0.1:{port}/0");
    let client = redis::Client::open(url.as_str()).expect("fixture client");
    (container, url, client)
}
async fn read(client: &redis::Client, stream: &str) -> redis::streams::StreamRangeReply {
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .expect("fixture read");
    redis::cmd("XRANGE")
        .arg(stream)
        .arg("-")
        .arg("+")
        .query_async(&mut connection)
        .await
        .expect("stream entries")
}

#[tokio::test]
async fn internal_notifications_preserve_raw_payload_and_stable_application_identity() {
    let (_redis, url, client) = redis().await;
    let publisher = RedisPublisher::new(
        RedisConfig::new(
            &url,
            "test:notifications",
            100,
            Duration::from_secs(3),
            false,
        )
        .expect("config"),
    );
    let payload = b"{ \"task\": 7 }";
    for _ in 0..2 {
        publisher
            .publish(
                "sagat.v1.task.ready.tenant.activity",
                "stable-outbox-id",
                payload,
            )
            .await
            .expect("acknowledged XADD");
    }
    let entries = read(&client, "test:notifications").await.ids;
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].id, entries[1].id);
    for row in entries {
        assert_eq!(
            row.get::<String>("message-id").expect("id"),
            "stable-outbox-id"
        );
        assert_eq!(
            row.get::<String>("subject").expect("subject"),
            "sagat.v1.task.ready.tenant.activity"
        );
        assert_eq!(row.get::<Vec<u8>>("payload").expect("bytes"), payload);
    }
}

#[tokio::test]
async fn internal_maxlen_is_exact_and_aof_survives_a_fixture_restart() {
    let (container, url, client) = redis().await;
    let publisher = RedisPublisher::new(
        RedisConfig::new(&url, "test:bounded", 2, Duration::from_secs(3), false).expect("config"),
    );
    for id in ["one", "two", "three"] {
        publisher
            .publish("sagat.v1.task.ready.t.a", id, b"{}")
            .await
            .expect("publish");
    }
    let before = read(&client, "test:bounded").await;
    assert_eq!(before.ids.len(), 2);
    container.stop().await.expect("stop own Redis");
    assert!(
        publisher
            .publish("sagat.v1.task.ready.t.a", "offline", b"{}")
            .await
            .is_err()
    );
    container.start().await.expect("restart own Redis");
    let restarted_port = container
        .get_host_port_ipv4(6379)
        .await
        .expect("current mapped Redis port");
    let restarted_url = format!("redis://127.0.0.1:{restarted_port}/0");
    eprintln!("owned Redis endpoint before={url}, after={restarted_url}");
    // Docker may allocate a new published port after stop/start; rediscovered fixture routing
    // must be used for the restart check, rather than retrying a stale test endpoint.
    let client = redis::Client::open(restarted_url.as_str()).expect("rediscovered fixture client");
    let publisher = RedisPublisher::new(
        RedisConfig::new(
            &restarted_url,
            "test:bounded",
            2,
            Duration::from_secs(3),
            false,
        )
        .expect("rediscovered endpoint"),
    );

    support::bounded::within("Redis restart readiness", Duration::from_secs(30), async {
        loop {
            if let Ok(mut connection) = client.get_multiplexed_async_connection().await {
                let ping: redis::RedisResult<String> =
                    redis::cmd("PING").query_async(&mut connection).await;
                if ping.is_ok() {
                    break;
                }
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    let after = read(&client, "test:bounded").await;
    assert_eq!(
        before.ids.iter().map(|r| &r.id).collect::<Vec<_>>(),
        after.ids.iter().map(|r| &r.id).collect::<Vec<_>>()
    );
    publisher
        .publish("sagat.v1.task.ready.t.a", "back", b"{}")
        .await
        .expect("fresh bounded attempt reconnects");
}

#[test]
fn configuration_bounds_tls_and_debug_redaction_are_fail_closed() {
    for (url, key, max, tls) in [
        ("redis://example:6379/0", "valid", 1, true),
        ("rediss://example:6379/16", "valid", 1, true),
        ("rediss://example:6379/0", "bad key", 1, true),
        ("rediss://example:6379/0", "valid", 0, true),
    ] {
        assert!(RedisConfig::new(url, key, max, Duration::from_secs(1), tls).is_err());
    }
    let config = RedisConfig::new(
        "rediss://user:private-canary@example:6379/0",
        "valid",
        1,
        Duration::from_secs(1),
        true,
    )
    .expect("TLS config");
    let shown = format!("{config:?}");
    assert!(!shown.contains("private-canary") && !shown.contains("example"));
}

#[tokio::test]
async fn committed_postgres_outbox_is_published_and_confirmed_through_redis() {
    use support::engine::{activity, document, world};
    let (_redis, url, client) = redis().await;
    let world = world().await;
    let (workflow, _) = world
        .publish(document("redis-outbox", "a", vec![activity("a")]))
        .await;
    world.start(workflow, &serde_json::json!({})).await;
    assert!(world.store.outbox_backlog().await.expect("before") > 0);
    let publisher = RedisPublisher::new(
        RedisConfig::new(&url, "test:outbox", 100, Duration::from_secs(3), false).expect("config"),
    );
    let dispatcher = sagat_scheduler::OutboxDispatcher::new(
        world.store.clone(),
        publisher,
        SystemEntropy,
        sagat_scheduler::DispatchSettings::default(),
    );
    let result = dispatcher.run_once().await;
    assert!(result.published > 0);
    assert_eq!(result.failed, 0);
    assert_eq!(world.store.outbox_backlog().await.expect("after"), 0);
    let entries = read(&client, "test:outbox").await;
    assert!(entries.ids.iter().any(|row| {
        row.get::<String>("subject")
            .is_some_and(|s| s.starts_with("sagat.v1.task.ready."))
    }));
}

#[tokio::test]
async fn a_noncanonical_stream_acknowledgement_is_never_success() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("owned protocol peer");
    let address = listener.local_addr().expect("peer address");
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("publisher connects");
        let mut received = Vec::new();
        loop {
            let mut buffer = [0; 1024];
            let count = socket.read(&mut buffer).await.expect("read command");
            assert!(count > 0, "publisher closed before XADD");
            received.extend_from_slice(&buffer[..count]);
            if received.windows(4).any(|s| s == b"XADD") {
                break;
            }
            assert!(received.len() < 4096, "unexpected handshake");
        }
        socket
            .write_all(b"++1-0\r\n")
            .await
            .expect("malformed stream id reply");
    });
    let config = RedisConfig::new(
        &format!("redis://{address}/0"),
        "test:ack",
        10,
        Duration::from_secs(2),
        false,
    )
    .expect("config");
    let result = RedisPublisher::new(config)
        .publish("sagat.v1.task.ready.t.a", "id", b"{}")
        .await;
    server.await.expect("actual XADD reached fake peer");
    assert!(
        result.is_err(),
        "a plus-prefixed numeric id is not a Redis stream acknowledgement"
    );
}

#[tokio::test]
async fn a_stalled_ack_is_bounded_and_drops_its_connection() {
    use tokio::io::AsyncReadExt as _;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("owned stalled peer");
    let address = listener.local_addr().expect("peer address");
    let (sent, received) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("client connects");
        let mut bytes = [0; 4096];
        let count = socket.read(&mut bytes).await.expect("read XADD");
        assert!(count > 0);
        sent.send(()).expect("record held request");
        support::bounded::within(
            "timed-out client closes its connection",
            Duration::from_secs(10),
            async {
                loop {
                    if socket.read(&mut bytes).await.expect("peer closure") == 0 {
                        break;
                    }
                }
            },
        )
        .await;
    });
    let config = RedisConfig::new(
        &format!("redis://{address}/0"),
        "test:timeout",
        10,
        Duration::from_secs(2),
        false,
    )
    .expect("config");
    let operation = tokio::spawn(async move {
        RedisPublisher::new(config)
            .publish("sagat.v1.task.ready.t.a", "id", b"{}")
            .await
    });
    support::bounded::within(
        "stalled server holds actual command",
        Duration::from_secs(5),
        received,
    )
    .await
    .expect("request observed");
    assert!(
        operation
            .await
            .expect("bounded publisher returned")
            .is_err()
    );
    server.await.expect("no detached connection retained");
}

#[tokio::test]
async fn the_actual_runtime_engine_selects_redis_and_starts_while_the_broker_is_paused() {
    use support::engine::{activity, document, world};
    use support::worker::{RunningEngine, engine_env};
    let (container, url, client) = redis().await;
    let world = world().await;
    let (workflow, _) = world
        .publish(document("redis-real-engine", "a", vec![activity("a")]))
        .await;
    let run = world.start(workflow, &serde_json::json!({})).await;
    let expected:uuid::Uuid=sqlx::query_scalar("SELECT id FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready'")
        .bind(world.admin.tenant_id().as_uuid()).bind(run.as_uuid()).fetch_one(&world.db.pool).await.expect("exact committed hint");
    let idp = support::oidc::TestIdp::start().await;
    container
        .pause()
        .await
        .expect("pause own broker before startup");
    let runtime = world
        .db
        .url_for(support::RUNTIME_ROLE, support::RUNTIME_ROLE_PASSWORD);
    let env = engine_env(
        &world.db,
        &idp,
        &[
            ("DATABASE_URL", runtime.as_str()),
            ("SAGAT_EVENT_TRANSPORT", "redis"),
            ("SAGAT_REDIS_URL", url.as_str()),
            ("SAGAT_REDIS_STREAM", "test:engine"),
            ("SAGAT_NATS_URL", "nats://127.0.0.1:1"),
        ],
    );
    let mut engine = support::bounded::within(
        "actual Redis engine startup",
        Duration::from_secs(30),
        RunningEngine::start(env),
    )
    .await;
    assert_eq!(engine.status("/livez").await, Some(200));
    assert!(
        world
            .store
            .outbox_backlog()
            .await
            .expect("paused broker preserves obligation")
            > 0
    );
    container.unpause().await.expect("resume own broker");
    support::bounded::within(
        "actual engine confirms committed Redis notification",
        Duration::from_secs(30),
        async {
            loop {
                if world.store.outbox_backlog().await.expect("outbox state") == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    let entries = read(&client, "test:engine").await;
    assert!(
        entries
            .ids
            .iter()
            .any(|entry| entry.get::<String>("message-id") == Some(expected.to_string()))
    );
    engine
        .finish()
        .await
        .expect("bounded clean engine shutdown");
}

#[test]
fn review_internal_configuration_refuses_unusable_endpoints_and_subsecond_deadlines() {
    for url in [
        "redis://example.test:0/0".to_owned(),
        format!("rediss://user:{}@example.test/0", "x".repeat(16 * 1024)),
    ] {
        assert!(
            RedisConfig::new(&url, "events", 10, Duration::from_secs(1), false).is_err(),
            "configuration must be bounded and usable"
        );
    }
    assert!(
        RedisConfig::new(
            "redis://example.test/0",
            "events",
            10,
            Duration::from_millis(999),
            false
        )
        .is_err()
    );
}

#[tokio::test]
async fn review_internal_rejection_and_unavailability_keep_distinct_outbox_codes() {
    let (container, url, client) = redis().await;
    let mut connection = client
        .get_multiplexed_async_connection()
        .await
        .expect("fixture");
    redis::cmd("SET")
        .arg("test:wrong-type")
        .arg("value")
        .query_async::<()>(&mut connection)
        .await
        .expect("wrong type");
    let publisher = RedisPublisher::new(
        RedisConfig::new(&url, "test:wrong-type", 100, Duration::from_secs(1), false)
            .expect("config"),
    );
    assert_eq!(
        publisher
            .publish("sagat.v1.task.ready.t.a", "id", b"{}")
            .await
            .expect_err("wrong type")
            .code(),
        "REJECTED"
    );
    container.stop().await.expect("stop owned broker");
    assert_eq!(
        publisher
            .publish("sagat.v1.task.ready.t.a", "id", b"{}")
            .await
            .expect_err("outage")
            .code(),
        "BROKER_UNAVAILABLE"
    );
}
