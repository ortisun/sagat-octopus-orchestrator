//! Production boot paths with no local keyring and no OpenBao provider.
use super::support;
use sagat_application::outbound::{BrokerSubscriptionSettings, OutboundService};
use sagat_domain::outbound::{BrokerDestination, DeliveryLimits, OutboundKind, SubscriptionStatus};
use std::time::Duration;
use support::{
    engine::{ctx, world},
    worker::{RunningEngine, engine_env},
};

#[tokio::test]
async fn secret_references_require_tls_at_management_time() {
    let w = world().await;
    w.put_secret("redis-password", r#"{"password":"fixture-only"}"#)
        .await;
    w.egress
        .set(
            &w.admin,
            w.namespace.id(),
            &["redis.example.test".to_owned()],
            &[6379],
            true,
            &ctx(),
        )
        .await
        .expect("policy");
    let service = OutboundService::new(
        w.store.clone(),
        sagat_application::clock::SystemClock,
        sagat_application::egress::EgressService::new(
            w.store.clone(),
            sagat_application::clock::SystemClock,
        ),
    )
    .with_redis_callbacks(true);
    let settings = BrokerSubscriptionSettings {
        destination: BrokerDestination::redis("redis://redis.example.test:6379", "callbacks", 0)
            .expect("destination"),
        credentials_secret: Some(
            sagat_domain::secrets::SecretName::parse("redis-password").expect("secret"),
        ),
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::default(),
        status: SubscriptionStatus::Active,
    };
    assert_eq!(
        service
            .create_broker(
                &w.admin,
                w.namespace.id(),
                support::slug("plaintext-refused"),
                settings,
                &ctx()
            )
            .await,
        Err(sagat_application::error::AppError::NotAllowed(
            "Redis credentials require a TLS destination"
        ))
    );
}

#[tokio::test]
async fn the_actual_control_plane_manages_anonymous_callbacks_without_a_secret_backend() {
    let w = world().await;
    let idp = support::oidc::TestIdp::start().await;
    w.egress
        .set(
            &w.admin,
            w.namespace.id(),
            &["redis.example.test".to_owned()],
            &[6379],
            true,
            &ctx(),
        )
        .await
        .expect("policy");
    let runtime =
        w.db.url_for(support::RUNTIME_ROLE, support::RUNTIME_ROLE_PASSWORD);
    let env = engine_env(
        &w.db,
        &idp,
        &[
            ("DATABASE_URL", runtime.as_str()),
            ("SAGAT_REDIS_CALLBACKS_ENABLED", "true"),
            ("SAGAT_ADMIN_ADDR", "off"),
        ],
    );
    let lookup = |name: &str| env.get(name).cloned();
    let config = sagat_control_plane::config::Config::from_lookup(&lookup).expect("config");
    let serve =
        sagat_control_plane::config::ServeConfig::from_lookup(&lookup).expect("serve config");
    assert!(serve.secrets.is_none() && serve.openbao.is_none());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener");
    let base = format!("http://{}", listener.local_addr().expect("address"));
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        sagat_control_plane::serve(&config, &serve, listener, async {
            let _ = stopped.await;
        })
        .await
    });
    let client = reqwest::Client::new();
    support::bounded::within(
        "secretless control-plane startup",
        Duration::from_secs(10),
        async {
            loop {
                if client
                    .get(format!("{base}/v1/health"))
                    .send()
                    .await
                    .is_ok_and(|r| r.status() == 200)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    let tenant: String = sqlx::query_scalar("SELECT slug FROM tenants WHERE id=$1")
        .bind(w.admin.tenant_id().as_uuid())
        .fetch_one(&w.db.pool)
        .await
        .expect("tenant");
    let token = idp.token(w.admin.subject().as_str(), &tenant).build();
    let response = client.post(format!("{base}/v1/namespaces/{}/broker-subscriptions", w.namespace.id()))
        .bearer_auth(token).header("idempotency-key", "anonymous-redis")
        .json(&serde_json::json!({"name":"anonymous-redis", "kind":"REDIS", "settings":{"server":"redis://redis.example.test:6379", "stream":"callbacks"}, "events":["run.finished"]}))
        .send().await.expect("management request");
    let status = response.status();
    let body: serde_json::Value = response.json().await.expect("response");
    stop.send(()).expect("stop server");
    support::bounded::within("join control plane", Duration::from_secs(15), server)
        .await
        .expect("joined")
        .expect("clean exit");
    assert_eq!(status, 201, "{body}");
}

#[tokio::test]
async fn the_actual_engine_attempts_anonymous_callbacks_without_a_secret_backend() {
    let w = world().await;
    let subscription = super::callbacks::subscribe(&w, "redis://127.0.0.1:6379/0").await;
    super::callbacks::finish(&w).await;
    let idp = support::oidc::TestIdp::start().await;
    let runtime =
        w.db.url_for(support::RUNTIME_ROLE, support::RUNTIME_ROLE_PASSWORD);
    let env = engine_env(
        &w.db,
        &idp,
        &[
            ("DATABASE_URL", runtime.as_str()),
            ("SAGAT_EVENT_TRANSPORT", "none"),
            ("SAGAT_DELIVERY_POLL_MS", "20"),
        ],
    );
    let mut engine = RunningEngine::start(env).await;
    let result = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let outcome: Option<String> = sqlx::query_scalar(
                "SELECT last_outcome FROM webhook_deliveries WHERE subscription_id=$1",
            )
            .bind(subscription.id.as_uuid())
            .fetch_one(&w.db.pool)
            .await
            .expect("outcome");
            if let Some(outcome) = outcome {
                break outcome;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    engine.finish().await.expect("clean engine exit");
    assert_eq!(
        result.expect("anonymous delivery loop must run without secrets"),
        "DENIED",
        "production egress still refuses loopback"
    );
}
