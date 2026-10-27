//! Existing persisted event-consumer keys are compatibility data for active root workflows.
#![allow(clippy::expect_used)]
use sagat_application::dispatch::{ClaimedEvent, DeliveryRef, WaitingStep};
use sagat_domain::{
    events::{EventEnvelope, EventId, EventKey},
    execution::{CorrelationKey, RunId},
    identity::{NamespaceId, Slug, TenantId},
    workflow::{EventType, NodeId},
};
use time::OffsetDateTime;
use uuid::Uuid;

fn event() -> ClaimedEvent {
    ClaimedEvent {
        tenant_id: TenantId::new(Uuid::from_u128(1)),
        namespace_id: NamespaceId::new(Uuid::from_u128(2)),
        envelope: EventEnvelope::new(
            EventKey {
                source: Slug::parse("api").expect("source"),
                event_id: EventId::parse("event-1").expect("event"),
            },
            EventType::parse("order.notified").expect("event type"),
            None,
            Some(CorrelationKey::parse("order-1").expect("key")),
        )
        .expect("envelope"),
        occurred_at: None,
        payload: b"{}".to_vec(),
        received_at: OffsetDateTime::UNIX_EPOCH,
        attempts: 0,
        trace: None,
    }
}

#[test]
fn root_wait_delivery_preserves_the_exact_existing_consumer_and_metadata() {
    let event = event();
    let run = RunId::new(Uuid::from_u128(3));
    let wait = WaitingStep {
        tenant_id: event.tenant_id,
        namespace_id: event.namespace_id,
        run_id: run,
        node: NodeId::parse("notified").expect("node"),
        event_type: event.envelope.event_type.clone(),
        correlation_key: event.envelope.correlation_key.clone().expect("key"),
        since: OffsetDateTime::UNIX_EPOCH,
    };
    let delivery = DeliveryRef::to_wait(&event, &wait);
    assert_eq!(
        delivery.consumer,
        "wait:00000000-0000-0000-0000-000000000003/notified"
    );
    assert_eq!(delivery.namespace_id, event.namespace_id);
    assert_eq!(delivery.source, "api");
    assert_eq!(delivery.event_id, "event-1");
    assert_eq!(delivery.run_id, Some(run));
    assert_eq!(delivery.node.as_deref(), Some("notified"));
    assert_eq!(delivery.trigger_id, None);
}

#[test]
fn trigger_delivery_preserves_its_independent_existing_consumer_namespace() {
    let event = event();
    let run = RunId::new(Uuid::from_u128(3));
    let trigger = Uuid::from_u128(4);
    let delivery = DeliveryRef::to_trigger(&event, trigger, run);
    assert_eq!(
        delivery.consumer,
        "trigger:00000000-0000-0000-0000-000000000004"
    );
    assert_eq!(delivery.namespace_id, event.namespace_id);
    assert_eq!(delivery.run_id, Some(run));
    assert_eq!(delivery.node, None);
    assert_eq!(delivery.trigger_id, Some(trigger));
}
