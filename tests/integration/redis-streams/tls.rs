//! Real Redis TLS/authentication; fresh namespace credentials and immutable per-attempt DB.
use super::{read, support};
use sagat_application::outbound::{BrokerFailure, BrokerPublisher, BrokerRequest};
use sagat_domain::{
    outbound::{BrokerDestination, SubscriptionId},
    secrets::SecretValue,
};
use sagat_infra_connectors::guard::ResolveFailure;
use sagat_infra_connectors::{guard::Unguarded, publish::Publisher, webhook::Clients};
use std::time::Duration;
use testcontainers_modules::testcontainers::{
    GenericImage, ImageExt,
    core::{IntoContainerPort, WaitFor},
    runners::AsyncRunner,
};

fn request(
    w: &support::engine::World,
    endpoint: &str,
    database: u8,
    secret: Option<serde_json::Value>,
) -> BrokerRequest {
    BrokerRequest {
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        subscription_id: SubscriptionId::generate(),
        destination: BrokerDestination::redis(endpoint, "test:tls", database).expect("destination"),
        credentials: secret.map(|value| {
            SecretValue::new(serde_json::to_vec(&value).expect("fixture JSON"))
                .expect("fixture value")
        }),
        headers: [
            ("content-type", "application/json"),
            ("event-id", "evt-1"),
            ("event-type", "run.finished"),
            ("delivery-id", "delivery-1"),
            ("attempt", "1"),
            ("schema-version", "1"),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect(),
        body: br#"{"type":"run.finished"}"#.to_vec(),
        timeout: Duration::from_secs(3),
    }
}

#[tokio::test]
async fn real_tls_authentication_checks_ca_hostname_acl_and_database_isolation() {
    use rcgen::{BasicConstraints, CertificateParams, IsCa, KeyPair, KeyUsagePurpose};
    let ca_key = KeyPair::generate().expect("CA key");
    let mut params = CertificateParams::new(Vec::new()).expect("params");
    params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca = params.self_signed(&ca_key).expect("CA");
    let key = KeyPair::generate().expect("server key");
    // Only localhost is signed: connecting by IP must fail hostname verification.
    let leaf = CertificateParams::new(vec!["localhost".to_owned()])
        .expect("leaf params")
        .signed_by(&key, &ca, &ca_key)
        .expect("leaf");
    let image = GenericImage::new("redis", "8.6.8-alpine")
        .with_exposed_port(6379.tcp())
        .with_wait_for(WaitFor::message_on_stdout("Ready to accept connections"))
        .with_copy_to("/tmp/ca.crt", ca.pem().into_bytes())
        .with_copy_to("/tmp/server.crt", leaf.pem().into_bytes())
        .with_copy_to("/tmp/server.key", key.serialize_pem().into_bytes())
        .with_cmd([
            "redis-server",
            "--port",
            "0",
            "--tls-port",
            "6379",
            "--tls-cert-file",
            "/tmp/server.crt",
            "--tls-key-file",
            "/tmp/server.key",
            "--tls-ca-cert-file",
            "/tmp/ca.crt",
            "--tls-auth-clients",
            "no",
            "--requirepass",
            "fixture-only-password",
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
    let container = support::bounded::within(
        "start own TLS Redis",
        Duration::from_secs(90),
        image.start(),
    )
    .await
    .expect("TLS Redis");
    let port = container.get_host_port_ipv4(6379).await.expect("port");
    let endpoint = format!("rediss://localhost:{port}");
    let w = support::engine::world().await;
    let publisher = Publisher::with_parts(
        PinnedFixture(std::net::SocketAddr::from(([127, 0, 0, 1], port))),
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    let secret = serde_json::json!({"password":"fixture-only-password", "caCert":ca.pem()});
    let a = request(&w, &endpoint, 0, Some(secret.clone()));
    let mut b = request(&w, &endpoint, 1, Some(secret.clone()));
    b.tenant_id = w.other_admin.tenant_id();
    b.namespace_id = w.other_namespace.id();
    let (a_result, b_result) = tokio::join!(publisher.publish(&a), publisher.publish(&b));
    a_result.expect("TLS db0");
    b_result.expect("TLS db1");
    for database in [0, 1] {
        let client = redis::Client::build_with_tls(
            format!("{endpoint}/{database}"),
            redis::TlsCertificates {
                client_tls: None,
                root_cert: Some(ca.pem().into_bytes()),
            },
        )
        .expect("reader");
        let mut info = client.get_connection_info().clone();
        info = info.clone().set_redis_settings(
            info.redis_settings()
                .clone()
                .set_password("fixture-only-password"),
        );
        let client = redis::Client::build_with_tls(
            info,
            redis::TlsCertificates {
                client_tls: None,
                root_cert: Some(ca.pem().into_bytes()),
            },
        )
        .expect("authenticated reader");
        assert_eq!(
            read(&client, "test:tls").await.ids.len(),
            1,
            "SELECT cannot leak across concurrent namespace attempts"
        );
    }
    let wrong_ca = request(
        &w,
        &endpoint,
        0,
        Some(serde_json::json!({"password":"fixture-only-password"})),
    );
    assert!(
        publisher.publish(&wrong_ca).await.is_err(),
        "untrusted CA must fail"
    );
    let wrong_host = request(
        &w,
        &format!("rediss://127.0.0.1:{port}"),
        0,
        Some(secret.clone()),
    );
    assert!(
        publisher.publish(&wrong_host).await.is_err(),
        "IP is not signed by the server certificate"
    );
    let wrong_password = request(
        &w,
        &endpoint,
        0,
        Some(serde_json::json!({"password":"wrong-fixture-password", "caCert":ca.pem()})),
    );
    assert!(publisher.publish(&wrong_password).await.is_err());
    // An ACL user only gets the two fixture keys and protocol commands used by this adapter.
    use redis::IntoConnectionInfo as _;
    let info = format!("{endpoint}/0")
        .into_connection_info()
        .expect("reader URL");
    let settings = info
        .redis_settings()
        .clone()
        .set_password("fixture-only-password");
    let admin = redis::Client::build_with_tls(
        info.set_redis_settings(settings),
        redis::TlsCertificates {
            client_tls: None,
            root_cert: Some(ca.pem().into_bytes()),
        },
    )
    .expect("admin client");
    let mut admin_connection = admin
        .get_multiplexed_async_connection()
        .await
        .expect("admin TLS");
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("callback-user")
        .arg("on")
        .arg(">first-fixture-pass")
        .arg("~test:tls")
        .arg("~test:callbacks")
        .arg("+xadd")
        .arg("+select")
        .arg("+ping")
        .query_async::<String>(&mut admin_connection)
        .await
        .expect("fixture ACL");
    let first_secret = serde_json::json!({"username":"callback-user", "password":"first-fixture-pass", "caCert":ca.pem()});
    publisher
        .publish(&request(&w, &endpoint, 0, Some(first_secret.clone())))
        .await
        .expect("ACL credentials");
    // The durable delivery worker opens the current namespace secret on each attempt.
    w.put_secret("redis-auth", &first_secret.to_string()).await;
    let subscription = super::callbacks::subscribe(&w, &format!("{endpoint}/0")).await;
    sqlx::query("UPDATE webhook_subscriptions SET secret_name = 'redis-auth' WHERE id = $1")
        .bind(subscription.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("attach fixture secret reference");
    let delivery_worker = sagat_scheduler::DeliveryWorker::with_broker(
        w.store.clone(),
        support::receiver::LoopbackSender::new(),
        Publisher::with_parts(
            PinnedFixture(std::net::SocketAddr::from(([127, 0, 0, 1], port))),
            Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
        ),
        sagat_application::entropy::SystemEntropy,
        w.keys.cipher(),
        sagat_scheduler::DeliverySettings::default(),
    );
    super::callbacks::finish(&w).await;
    assert_eq!(delivery_worker.tick().await.delivered, 1);
    redis::cmd("ACL")
        .arg("SETUSER")
        .arg("callback-user")
        .arg("resetpass")
        .arg(">rotated-fixture-pass")
        .query_async::<String>(&mut admin_connection)
        .await
        .expect("rotate broker password");
    assert!(
        publisher
            .publish(&request(&w, &endpoint, 0, Some(first_secret)))
            .await
            .is_err(),
        "fresh attempts cannot reuse authenticated old sessions"
    );
    w.put_secret("redis-auth", &serde_json::json!({"username":"callback-user", "password":"rotated-fixture-pass", "caCert":ca.pem()}).to_string()).await;
    let (workflow, _) = w
        .publish(support::engine::document(
            "redis-after-rotation",
            "work",
            vec![support::engine::activity("work")],
        ))
        .await;
    w.start(workflow, &serde_json::json!({})).await;
    let session = w.session(&["act.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &serde_json::json!({"ok":true}))
        .await
        .expect("second run");
    // Release the existing subscription rate slot explicitly instead of sleeping.
    sqlx::query("UPDATE webhook_subscriptions SET next_slot_at = '-infinity' WHERE id = $1")
        .bind(subscription.id.as_uuid())
        .execute(&w.db.pool)
        .await
        .expect("rate slot");
    assert_eq!(
        delivery_worker.tick().await.delivered,
        1,
        "current secret version is used"
    );
    assert_eq!(read(&admin, "test:callbacks").await.ids.len(), 2);
    let debug = format!("{a:?}");
    assert!(!debug.contains("fixture-only-password"));
}

#[tokio::test]
async fn plaintext_auth_wrong_secret_types_and_reserved_fields_fail_without_an_append() {
    let (_redis, url, client) = super::redis().await;
    let w = support::engine::world().await;
    let publisher = Publisher::with_parts(
        Unguarded,
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    for secret in [
        serde_json::json!({"password":"fixture"}),
        serde_json::json!({"password":12}),
        serde_json::json!({"username":"acl-only"}),
        serde_json::json!({"token":"ignored-field"}),
        serde_json::json!({}),
        serde_json::json!({"caCert":false}),
    ] {
        let request = request(&w, url.trim_end_matches("/0"), 0, Some(secret));
        assert_eq!(
            publisher.publish(&request).await,
            Err(BrokerFailure::Rejected)
        );
    }
    let mut collision = request(&w, url.trim_end_matches("/0"), 0, None);
    collision
        .headers
        .push(("body".to_owned(), "shadow".to_owned()));
    assert_eq!(
        publisher.publish(&collision).await,
        Err(BrokerFailure::Rejected)
    );
    collision.headers.pop();
    collision
        .headers
        .push(("event-id".to_owned(), "duplicate".to_owned()));
    assert_eq!(
        publisher.publish(&collision).await,
        Err(BrokerFailure::Rejected)
    );
    assert!(read(&client, "test:tls").await.ids.is_empty());
    let guarded = Publisher::new(w.store.clone());
    let denied = request(&w, url.trim_end_matches("/0"), 0, None);
    assert_eq!(guarded.publish(&denied).await, Err(BrokerFailure::Denied));
}

struct PinnedFixture(std::net::SocketAddr);
impl sagat_infra_connectors::guard::BrokerGuard for PinnedFixture {
    async fn permits(
        &self,
        _: sagat_domain::identity::TenantId,
        _: sagat_domain::identity::NamespaceId,
        _: &[sagat_domain::connectors::Server],
    ) -> bool {
        true
    }
    async fn resolve(
        &self,
        _: sagat_domain::identity::TenantId,
        _: sagat_domain::identity::NamespaceId,
        _: &sagat_domain::connectors::Server,
    ) -> Result<Vec<std::net::SocketAddr>, ResolveFailure> {
        Ok(vec![self.0])
    }
}
#[tokio::test]
async fn the_connection_uses_only_the_addresses_returned_by_the_guard() {
    let (_redis, url, client) = super::redis().await;
    let w = support::engine::world().await;
    let address = url
        .trim_start_matches("redis://")
        .trim_end_matches("/0")
        .parse::<std::net::SocketAddr>()
        .expect("fixture address");
    let publisher = Publisher::with_parts(
        PinnedFixture(address),
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    let request = request(
        &w,
        &format!("redis://never-resolve-this.invalid:{}", address.port()),
        0,
        None,
    );
    publisher
        .publish(&request)
        .await
        .expect("the original hostname must not be resolved a second time");
    assert_eq!(read(&client, "test:tls").await.ids.len(), 1);
}

#[tokio::test]
async fn a_callback_cannot_omit_or_change_the_json_content_type() {
    let (_redis, url, client) = super::redis().await;
    let w = support::engine::world().await;
    let publisher = Publisher::with_parts(
        Unguarded,
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    let mut missing = request(&w, url.trim_end_matches("/0"), 0, None);
    missing.headers.retain(|(name, _)| name != "content-type");
    assert_eq!(
        publisher.publish(&missing).await,
        Err(BrokerFailure::Rejected)
    );
    missing
        .headers
        .push(("content-type".to_owned(), "text/plain".to_owned()));
    assert_eq!(
        publisher.publish(&missing).await,
        Err(BrokerFailure::Rejected)
    );
    assert!(read(&client, "test:tls").await.ids.is_empty());
}

struct TwoCandidates(Vec<std::net::SocketAddr>);
impl sagat_infra_connectors::guard::BrokerGuard for TwoCandidates {
    async fn permits(
        &self,
        _: sagat_domain::identity::TenantId,
        _: sagat_domain::identity::NamespaceId,
        _: &[sagat_domain::connectors::Server],
    ) -> bool {
        true
    }
    async fn resolve(
        &self,
        _: sagat_domain::identity::TenantId,
        _: sagat_domain::identity::NamespaceId,
        _: &sagat_domain::connectors::Server,
    ) -> Result<Vec<std::net::SocketAddr>, ResolveFailure> {
        Ok(self.0.clone())
    }
}
#[tokio::test]
async fn one_attempt_never_races_all_dns_candidates() {
    let (_redis, url, client) = super::redis().await;
    let address = url
        .trim_start_matches("redis://")
        .trim_end_matches("/0")
        .parse::<std::net::SocketAddr>()
        .expect("fixture address");
    let reserved = tokio::net::TcpSocket::new_v6().expect("reserved socket");
    reserved
        .bind(std::net::SocketAddr::new(
            std::net::Ipv6Addr::LOCALHOST.into(),
            address.port(),
        ))
        .expect("reserve closed TCP port");
    let closed = reserved.local_addr().expect("reserved address");
    let w = support::engine::world().await;
    let publisher = Publisher::with_parts(
        TwoCandidates(vec![closed, address]),
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    // A single-attempt adapter chooses one checked address; it does not race/fall back to a second
    // socket in the same attempt. The first address is held but deliberately does not listen.
    let request = request(&w, url.trim_end_matches("/0"), 0, None);
    assert!(publisher.publish(&request).await.is_err());
    assert!(read(&client, "test:tls").await.ids.is_empty());
    drop(reserved);
}

struct CountingGuard {
    address: Option<std::net::SocketAddr>,
    calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl sagat_infra_connectors::guard::BrokerGuard for CountingGuard {
    async fn permits(
        &self,
        _: sagat_domain::identity::TenantId,
        _: sagat_domain::identity::NamespaceId,
        _: &[sagat_domain::connectors::Server],
    ) -> bool {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        true
    }
    async fn resolve(
        &self,
        _: sagat_domain::identity::TenantId,
        _: sagat_domain::identity::NamespaceId,
        _: &sagat_domain::connectors::Server,
    ) -> Result<Vec<std::net::SocketAddr>, ResolveFailure> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.address
            .map(|address| vec![address])
            .ok_or(ResolveFailure::Network)
    }
}
#[tokio::test]
async fn redis_checks_and_resolves_the_destination_once_per_attempt() {
    let (_redis, url, _client) = super::redis().await;
    let w = support::engine::world().await;
    let address = url
        .trim_start_matches("redis://")
        .trim_end_matches("/0")
        .parse()
        .expect("fixture address");
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let publisher = Publisher::with_parts(
        CountingGuard {
            address: Some(address),
            calls: calls.clone(),
        },
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    publisher
        .publish(&request(&w, url.trim_end_matches("/0"), 0, None))
        .await
        .expect("append");
    assert_eq!(
        calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the checked resolution must be the only guard operation"
    );
}
#[tokio::test]
async fn resolver_outages_are_network_failures_instead_of_policy_denials() {
    let w = support::engine::world().await;
    let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    // This fault guard represents an unavailable resolver, never an explicit policy refusal.
    let publisher = Publisher::with_parts(
        CountingGuard {
            address: None,
            calls,
        },
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    assert_eq!(
        publisher
            .publish(&request(&w, "rediss://redis.example.test:6379", 0, None))
            .await,
        Err(BrokerFailure::Network)
    );
}

#[tokio::test]
async fn a_real_redis_ack_lost_on_the_wire_retries_with_the_same_delivery_identity() {
    use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, BufReader};
    let (_redis, url, client) = super::redis().await;
    let backend = url
        .trim_start_matches("redis://")
        .trim_end_matches("/0")
        .parse::<std::net::SocketAddr>()
        .expect("backend");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("owned proxy");
    let endpoint = format!("redis://{}", listener.local_addr().expect("proxy address"));
    let proxy = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("first attempt");
        let upstream = tokio::net::TcpStream::connect(backend)
            .await
            .expect("owned Redis");
        let (mut receive, send) = socket.into_split();
        let (receive_upstream, mut send_upstream) = upstream.into_split();
        let forwarding = tokio::spawn(async move {
            tokio::io::copy(&mut receive, &mut send_upstream)
                .await
                .expect("forward XADD");
        });
        let mut reader = BufReader::new(receive_upstream);
        let mut header = String::new();
        reader
            .read_line(&mut header)
            .await
            .expect("actual Redis reply");
        let size = header
            .strip_prefix('$')
            .expect("bulk stream id")
            .trim_end()
            .parse::<usize>()
            .expect("size");
        assert!(size <= 64);
        let mut acknowledgement = vec![0; size + 2];
        reader
            .read_exact(&mut acknowledgement)
            .await
            .expect("complete acknowledged stream id");
        assert_eq!(&acknowledgement[size..], b"\r\n");
        let id = String::from_utf8(acknowledgement[..size].to_vec()).expect("stream id");
        // Redis accepted the append. Drop that exact ACK before it reaches the publisher.
        drop(send);
        drop(reader);
        support::bounded::within(
            "close first proxy connection",
            Duration::from_secs(10),
            forwarding,
        )
        .await
        .expect("forwarder joined");
        let (mut socket, _) = listener
            .accept()
            .await
            .expect("retry on same configured endpoint");
        let mut upstream = tokio::net::TcpStream::connect(backend)
            .await
            .expect("Redis retry");
        tokio::io::copy_bidirectional(&mut socket, &mut upstream)
            .await
            .expect("forward retry and ACK");
        id
    });
    let w = support::engine::world().await;
    let publisher = Publisher::with_parts(
        Unguarded,
        Clients::with_factory(w.store.clone(), support::receiver::OpenFactory),
    );
    let request = request(&w, &endpoint, 0, None);
    assert!(
        publisher.publish(&request).await.is_err(),
        "a lost real ACK cannot confirm delivery"
    );
    assert_eq!(read(&client, "test:tls").await.ids.len(), 1);
    publisher
        .publish(&request)
        .await
        .expect("retry acknowledged");
    let lost_id =
        support::bounded::within("join owned ACK-loss proxy", Duration::from_secs(10), proxy)
            .await
            .expect("proxy joined");
    let entries = read(&client, "test:tls").await.ids;
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, lost_id);
    assert_ne!(entries[0].id, entries[1].id);
    assert_eq!(
        entries[0].get::<String>("delivery-id"),
        entries[1].get::<String>("delivery-id")
    );
    assert_eq!(
        entries[0].get::<Vec<u8>>("body"),
        entries[1].get::<Vec<u8>>("body")
    );
}
