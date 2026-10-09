//! What the platform really produces, held to the shapes the document promises.

use std::time::Duration;

use sagat_application::outbound::{OutboundStore, SubscriptionRecord};
use sagat_domain::outbound::{
    DeliveryLimits, OutboundKind, SubscriptionId, SubscriptionStatus, TargetUrl,
};
use sagat_domain::secrets::SecretName;
use serde_json::{Value, json};

use crate::load;
use crate::support::engine::{activity, ctx, document, world};
use crate::support::receiver::Receiver;
use crate::support::{now, slug};

/// A validator for a schema of the document, with the document's components alongside.
fn validator(schema: &Value) -> jsonschema::Validator {
    let mut root = schema.clone();
    root.as_object_mut()
        .expect("object")
        .insert("components".into(), load()["components"].clone());
    jsonschema::options()
        .should_validate_formats(true)
        .build(&root)
        .expect("a valid schema")
}

fn conforms(schema: &Value, instance: &Value, what: &str) {
    let errors: Vec<String> = validator(schema)
        .iter_errors(instance)
        .map(|e| format!("{e} (at {})", e.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "{what} does not conform: {}\n{instance}",
        errors.join("; ")
    );
}

#[tokio::test]
async fn the_outbound_webhook_and_the_event_document_conform_to_the_document() {
    let w = world().await;
    w.put_secret("provider-signing", "whsec-0123456789abcdef0123456789abcdef")
        .await;
    let receiver = Receiver::start().await;
    let created = now();
    let subscription = SubscriptionRecord {
        id: SubscriptionId::generate(),
        tenant_id: w.admin.tenant_id(),
        namespace_id: w.namespace.id(),
        name: slug("conformance"),
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

    // A workflow started by an event: the document it is given as input, and the run it finishes.
    let mut ir = document("conformance-flow", "work", vec![activity("work")]);
    ir["triggers"] = json!([{"kind": "event", "eventType": "order.created"}]);
    let (_, _) = w.publish(ir).await;
    w.events
        .publish(
            &w.admin,
            sagat_application::events::PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(sagat_domain::events::EventId::parse("o-1").expect("id")),
                event_type: sagat_domain::workflow::EventType::parse("order.created")
                    .expect("type"),
                subject: Some("orders/1".to_owned()),
                correlation_key: Some(
                    sagat_domain::execution::CorrelationKey::parse("ord-1").expect("key"),
                ),
                occurred_at: Some(time::OffsetDateTime::now_utc()),
                data: br#"{"amount":10}"#.to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published");
    w.dispatcher.tick().await;
    let input: Vec<u8> = sqlx::query_scalar(
        "SELECT p.content FROM runs r JOIN payloads p ON p.tenant_id = r.tenant_id AND p.digest = r.input_digest",
    )
    .fetch_one(&w.db.pool)
    .await
    .expect("input");
    let document_schema = load();
    conforms(
        &document_schema["components"]["messages"]["EventDocument"]["payload"],
        &serde_json::from_slice(&input).expect("json"),
        "the document a run is started with",
    );

    // Finish the run and take the webhook the subscriber is sent.
    let session = w.session(&["act.work"]).await;
    let task = w.one_task(&session).await;
    w.complete(&session, &task, &json!({}))
        .await
        .expect("complete");
    w.deliveries.tick().await;
    let seen = &receiver.seen()[0];
    let message = &document_schema["components"]["messages"]["RunFinishedWebhook"];
    conforms(
        &message["payload"],
        &serde_json::from_slice(&seen.body).expect("json"),
        "the webhook body",
    );
    let headers: Value = serde_json::to_value(&seen.headers).expect("headers");
    // Header names are case-insensitive and the receiver sees them lower-cased.
    let by_name: serde_json::Map<String, Value> = message["headers"]["properties"]
        .as_object()
        .expect("declared headers")
        .keys()
        .filter_map(|declared| {
            headers
                .get(declared.to_ascii_lowercase())
                .map(|v| (declared.clone(), v.clone()))
        })
        .collect();
    conforms(
        &message["headers"],
        &Value::Object(by_name),
        "the webhook headers",
    );
    let _ = ctx();
}
