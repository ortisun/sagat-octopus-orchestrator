//! Existing persisted event-consumer keys are compatibility data for active root workflows.
#![allow(clippy::expect_used)]
use sagat_application::dispatch::{ClaimedEvent, DeliveryRef, WaitingStep};
use sagat_domain::{
    events::{EventEnvelope, EventId, EventKey},
    execution::{CorrelationKey, ExecutionInstanceId, InstancePath, RunId},
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
        instance: ExecutionInstanceId::root(NodeId::parse("notified").expect("node")),
        step_id: Uuid::from_u128(4),
        event_type: event.envelope.event_type.clone(),
        correlation_key: event.envelope.correlation_key.clone().expect("key"),
        since: OffsetDateTime::UNIX_EPOCH,
    };
    let delivery = DeliveryRef::to_wait(&event, &wait).expect("root delivery");
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

#[test]
fn scoped_candidates_cannot_construct_a_legacy_root_delivery() {
    let event = event();
    for index in [0, 1, 9999] {
        let path = InstancePath::root()
            .child(NodeId::parse("lines").expect("scope"), index)
            .expect("path");
        let wait = WaitingStep {
            tenant_id: event.tenant_id,
            namespace_id: event.namespace_id,
            run_id: RunId::new(Uuid::from_u128(3)),
            instance: ExecutionInstanceId::in_path(path, NodeId::parse("notified").expect("node")),
            step_id: Uuid::from_u128(100 + u128::from(index)),
            event_type: event.envelope.event_type.clone(),
            correlation_key: event.envelope.correlation_key.clone().expect("key"),
            since: OffsetDateTime::UNIX_EPOCH,
        };
        assert_eq!(
            DeliveryRef::to_wait(&event, &wait),
            None,
            "a qualified candidate cannot be projected to a root receipt"
        );
    }
}

#[test]
fn root_receiver_requires_the_exact_pinned_version_run_path_and_step_uuid() {
    let event = event();
    let version = sagat_domain::definition::WorkflowVersionId::generate();
    let mut wait = WaitingStep {
        tenant_id: event.tenant_id,
        namespace_id: event.namespace_id,
        run_id: RunId::new(Uuid::from_u128(3)),
        instance: ExecutionInstanceId::root(NodeId::parse("notified").expect("node")),
        step_id: Uuid::nil(),
        event_type: event.envelope.event_type.clone(),
        correlation_key: event.envelope.correlation_key.clone().expect("key"),
        since: OffsetDateTime::UNIX_EPOCH,
    };
    assert_eq!(wait.root_node(version), None, "wrong stored UUID");
    wait.step_id =
        sagat_application::execution::instance_step_id(version, wait.run_id, &wait.instance);
    assert_eq!(wait.root_node(version), Some(wait.instance.node()));
    assert_eq!(
        wait.root_node(sagat_domain::definition::WorkflowVersionId::generate()),
        None,
        "wrong pinned version"
    );
    let original_run = wait.run_id;
    wait.run_id = RunId::new(Uuid::from_u128(5));
    assert_eq!(wait.root_node(version), None, "wrong owning run");
    wait.run_id = original_run;
    wait.instance = ExecutionInstanceId::in_path(
        InstancePath::root()
            .child(NodeId::parse("lines").expect("scope"), 0)
            .expect("path"),
        wait.instance.node().clone(),
    );
    wait.step_id =
        sagat_application::execution::instance_step_id(version, wait.run_id, &wait.instance);
    assert_eq!(
        wait.root_node(version),
        None,
        "even a valid scoped UUID cannot authorize a root receiver"
    );
}
