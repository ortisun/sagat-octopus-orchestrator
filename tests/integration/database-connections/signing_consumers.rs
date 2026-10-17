//! Migration grace versions cannot become an outage fallback for HMAC consumers.
use super::{
    consumers::Down,
    support::{
        self,
        engine::{activity, ctx, document, world},
        receiver::Receiver,
    },
};
use sagat_application::{
    clock::SystemClock,
    error::AppError,
    events::{EndpointSettings, WebhookRequest},
    managed_secrets::ManagedSecretService,
    outbound::{Health, OutboundStore, SubscriptionRecord},
    secret_provider::SecretProvider,
};
use sagat_domain::{
    audit::{AuditAction, AuditRecord, ResourceType},
    events::{EndpointLimits, EndpointStatus, WebhookAuthentication},
    outbound::{
        DeliveryLimits, OutboundDestination, OutboundKind, SubscriptionId, SubscriptionStatus,
        TargetUrl,
    },
    secrets::{SecretName, SecretValue},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

pub async fn run(provider: Arc<dyn SecretProvider>) {
    let mut w = world().await;
    let name = "migrated-signing";
    let old = "prior-hmac-secret-0123456789abcdef";
    let current = "current-hmac-secret-0123456789abcdef";
    w.put_secret(name, old).await;
    w.put_secret(name, current).await;
    let ns = w.namespace.id();
    let endpoint = w
        .events
        .create_endpoint(
            &w.admin,
            ns,
            support::slug("migrated-hmac"),
            EndpointSettings {
                secret_name: Some(SecretName::parse(name).unwrap()),
                authentication: WebhookAuthentication::Hmac,
                default_event_type: None,
                limits: EndpointLimits::default(),
                status: EndpointStatus::Active,
            },
            &ctx(),
        )
        .await
        .unwrap();
    ManagedSecretService::new(
        w.store.clone(),
        SystemClock,
        provider.clone(),
        SecretValue::new(vec![42; 32]).unwrap(),
    )
    .unwrap()
    .with_legacy_cipher(w.keys.cipher())
    .migrate(&w.admin, ns, name, 2, Uuid::new_v4(), &ctx())
    .await
    .unwrap();
    w.events = w
        .events
        .with_secret_provider(Arc::new(Down(provider.clone())));
    let timestamp = time::OffsetDateTime::now_utc().unix_timestamp();
    let stamp = timestamp.to_string();
    let body = b"{}";
    let signature =
        support::signed::inbound(old, timestamp, "provider-outage", "invoice.paid", body);
    let request = WebhookRequest {
        trace: None,
        signature: Some(&signature),
        bearer: None,
        certificate_sha256: None,
        timestamp: Some(&stamp),
        event_id: Some("provider-outage"),
        event_type: Some("invoice.paid"),
        subject: None,
        correlation_key: None,
        body,
    };
    assert!(
        matches!(
            w.events.receive(endpoint.id, &request).await,
            Err(AppError::Unavailable)
        ),
        "provider outage cannot verify using a retained local key"
    );
    let inbox: i64 = sqlx::query_scalar("SELECT count(*) FROM inbox_events")
        .fetch_one(&w.db.pool)
        .await
        .unwrap();
    assert_eq!(inbox, 0);
    w.events = w.events.with_secret_provider(provider.clone());
    assert!(
        !w.events
            .receive(endpoint.id, &request)
            .await
            .unwrap()
            .duplicate,
        "healthy migration grace still verifies old signatures"
    );

    let receiver = Receiver::start().await;
    let now = support::now();
    let subscription = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: w.admin.tenant_id(),
        namespace_id: ns,
        name: support::slug("migrated-delivery"),
        destination: OutboundDestination::Webhook {
            url: TargetUrl::parse(&receiver.url).unwrap(),
            signing_secret: SecretName::parse(name).unwrap(),
        },
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::new(100, Duration::from_secs(2)).unwrap(),
        status: SubscriptionStatus::Active,
        health: Health::default(),
        created_by: "test".into(),
        created_at: now,
        updated_at: now,
    };
    // The test sender permits its loopback receiver; production egress has its independent gates.
    w.store
        .create_subscription(
            &subscription,
            100,
            &AuditRecord::provisioning(
                w.admin.tenant_id(),
                AuditAction::WebhookSubscriptionCreated,
                ResourceType::WebhookSubscription,
                subscription.id.to_string(),
                "test",
                now,
            ),
        )
        .await
        .unwrap();
    let (workflow, _) = w
        .publish(document("hmac-delivery", "a", vec![activity("a")]))
        .await;
    w.start(workflow, &json!({})).await;
    let session = w.session(&["act.a"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({})).await.unwrap();
    w.deliveries = w
        .deliveries
        .with_secret_provider(Arc::new(Down(provider.clone())));
    let failed = w.deliveries.tick().await;
    assert_eq!((failed.delivered, failed.retried), (0, 1));
    assert!(
        receiver.seen().is_empty(),
        "no outbound request may use only the retained local key"
    );
    sqlx::query(
        "UPDATE webhook_deliveries SET next_attempt_at=clock_timestamp() WHERE status='PENDING'",
    )
    .execute(&w.db.pool)
    .await
    .unwrap();
    sqlx::query("UPDATE webhook_subscriptions SET next_slot_at='-infinity'")
        .execute(&w.db.pool)
        .await
        .unwrap();
    w.deliveries = w.deliveries.with_secret_provider(provider);
    assert_eq!(w.deliveries.tick().await.delivered, 1);
    let seen = receiver.seen();
    assert_eq!(seen.len(), 1);
    assert!(support::signed::delivery_verifies(
        current,
        &seen[0].headers,
        &seen[0].body
    ));
}
