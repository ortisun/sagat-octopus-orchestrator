//! Outbound deliveries: a worker that dies between sending and recording sends again with the same
//! delivery id, and nothing is promised beyond that (FR-004-006).

use std::time::Duration;

use sagat_application::outbound::{
    DeliveryStore, OutboundRequest, OutboundStore, SubscriptionRecord, WebhookSender,
};
use sagat_domain::outbound::{
    DeliveryLimits, DeliveryStatus, OutboundKind, SubscriptionId, SubscriptionStatus, TargetUrl,
};
use sagat_domain::secrets::SecretName;
use serde_json::json;

use crate::support::engine::{activity, document, world};
use crate::support::receiver::{LoopbackSender, Receiver};
use crate::support::{now, slug};

#[tokio::test]
async fn a_worker_that_died_after_sending_sends_again_with_the_same_delivery_id() {
    let w = world().await;
    w.put_secret("provider-signing", "whsec-0123456789abcdef0123456789abcdef")
        .await;
    let receiver = Receiver::start().await;
    let created = now();
    let subscription = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        name: slug("finance"),
        url: TargetUrl::parse(&receiver.url).expect("url"),
        secret_name: SecretName::parse("provider-signing").expect("secret"),
        kinds: vec![OutboundKind::RunFinished],
        limits: DeliveryLimits::new(100, Duration::from_secs(2)).expect("limits"),
        status: SubscriptionStatus::Active,
        health: sagat_application::outbound::Health::default(),
        created_by: "test".to_owned(),
        created_at: created,
        updated_at: created,
    };
    w.store
        .create_subscription(
            &subscription,
            10,
            &sagat_domain::audit::AuditRecord::provisioning(
                w.admin.tenant_id(),
                sagat_domain::audit::AuditAction::WebhookSubscriptionCreated,
                sagat_domain::audit::ResourceType::WebhookSubscription,
                subscription.id.to_string(),
                "test",
                created,
            ),
        )
        .await
        .expect("subscribe");
    let (workflow, _) = w
        .publish(document("close", "work", vec![activity("work")]))
        .await;
    w.start(workflow, &json!({})).await;
    let session = w.session(&["act.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete");

    // A worker claims the delivery, sends it, and dies before it can write down the answer.
    let claimed = w
        .store
        .claim_deliveries(10, Duration::from_secs(60))
        .await
        .expect("claim");
    assert_eq!(claimed.len(), 1);
    let due = &claimed[0];
    let sender = LoopbackSender::new();
    let answer = sender
        .send(&OutboundRequest {
            tenant_id: due.delivery.tenant_id,
            namespace_id: due.delivery.namespace_id,
            url: due.url.clone(),
            headers: vec![
                (
                    "x-octopus-delivery-id".to_owned(),
                    due.delivery.id.to_string(),
                ),
                ("x-octopus-attempt".to_owned(), "1".to_owned()),
            ],
            body: due.delivery.payload.clone(),
            timeout: Duration::from_secs(2),
        })
        .await
        .expect("sent");
    assert_eq!(answer.status, 200);
    assert!(
        !w.deliveries.tick().await.did_work(),
        "nobody else sends it while the lease holds"
    );

    // The lease ends. The next worker cannot know the first one got through, so it sends again, and
    // the receiver sees the same delivery id on both: what it needs to drop the repeat.
    sqlx::query(
        "UPDATE webhook_deliveries SET next_attempt_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("the lease ends");
    sqlx::query("UPDATE webhook_subscriptions SET next_slot_at = '-infinity'")
        .execute(&w.db.pool)
        .await
        .expect("slots");
    assert_eq!(w.deliveries.tick().await.delivered, 1);
    let seen = receiver.seen();
    assert_eq!(seen.len(), 2, "at-least-once: the receiver was told twice");
    assert_eq!(
        seen[0].headers["x-octopus-delivery-id"], seen[1].headers["x-octopus-delivery-id"],
        "and told it is the same delivery"
    );
    let delivery = w
        .outbound
        .deliveries(&w.admin, subscription.id, None, None, 10)
        .await
        .expect("deliveries")
        .remove(0);
    assert_eq!(
        (delivery.status, delivery.attempts),
        (DeliveryStatus::Delivered, 1)
    );
    assert!(
        !w.deliveries.tick().await.did_work(),
        "and it is done for good"
    );
}
