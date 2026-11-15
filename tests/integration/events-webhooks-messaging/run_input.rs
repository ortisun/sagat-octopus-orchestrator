//! Design 043 against a real PostgreSQL, rows 9 and 10 of the creators (an event trigger, a cron trigger) and `report` mode
//! in the engine (T043-7, T043-21, T043-22, FR-043-007, FR-043-013).

use std::sync::Arc;

use sagat_application::definitions::InputValidationMode;
use sagat_application::events::PublishRequest;
use sagat_application::execution::Engine;
use sagat_application::run_input::test_support::Scripted;
use sagat_application::run_input::{RunInputGate, SchemaRefusal};
use sagat_domain::events::EventId;
use sagat_domain::workflow::EventType;
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_schema::run_input::RunInputValidator;
use sagat_scheduler::{EventDispatchSettings, EventDispatcher};
use serde_json::{Value, json};

use crate::support::engine::{World, activity, ctx, document, world};
use crate::support::run_input::{SENTINEL, audits, publish_in, unusable_schema};

fn triggered(name: &str, triggers: Value) -> Value {
    let mut doc = document(name, "work", vec![activity("work")]);
    doc["triggers"] = triggers;
    doc
}

/// A schema of the envelope `{event, data}` that requires `data.orderId`.
fn needs_order() -> Value {
    json!({
        "type": "object",
        "required": ["data"],
        "properties": {"data": {"type": "object", "required": ["orderId"]}}
    })
}

async fn publish_event(w: &World, id: &str, data: &Value) {
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse(id).expect("id")),
                event_type: EventType::parse("order.created").expect("type"),
                subject: None,
                correlation_key: None,
                occurred_at: None,
                data: serde_json::to_vec(data).expect("json"),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published");
}

async fn runs_named(w: &World, name: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM runs r JOIN workflows f ON f.id = r.workflow_id WHERE f.name = $1",
    )
    .bind(name)
    .fetch_one(&w.db.pool)
    .await
    .expect("runs")
}

/// Deliveries of `event` without a run, to a trigger.
async fn refused_deliveries(w: &World, event: &str) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM event_deliveries WHERE event_id = $1 AND consumer LIKE 'trigger:%' AND run_id IS NULL",
    )
    .bind(event)
    .fetch_one(&w.db.pool)
    .await
    .expect("deliveries")
}

/// Puts the event back in the inbox, as a retry after another consumer failed does.
async fn redeliver(w: &World, event: &str) {
    sqlx::query(
        "UPDATE inbox_events SET status = 'PENDING', dispatched_at = NULL, next_attempt_at = clock_timestamp() WHERE event_id = $1",
    )
    .bind(event)
    .execute(&w.db.pool)
    .await
    .expect("redeliver");
}

/// Row 9, CUJ 3: a trigger whose version refuses the envelope starts nothing, records its delivery without a run and one
/// audit row; the other trigger of the event is served; a redelivery repeats neither.
#[tokio::test]
async fn row_9_a_refused_trigger_records_a_delivery_and_one_audit_and_the_others_are_served() {
    let w = world().await;
    let mut strict = triggered(
        "row-9-strict",
        json!([{"kind": "event", "eventType": "order.created"}]),
    );
    strict["inputSchema"] = needs_order();
    w.publish(strict).await;
    w.publish(triggered(
        "row-9-free",
        json!([{"kind": "event", "eventType": "order.created"}]),
    ))
    .await;
    publish_event(&w, "row-9-e1", &json!({"amount": 1, "note": SENTINEL})).await;
    let round = w.dispatcher.tick().await;
    assert_eq!(round.runs_started, 1);
    assert_eq!(runs_named(&w, "row-9-strict").await, 0);
    assert_eq!(runs_named(&w, "row-9-free").await, 1);
    assert_eq!(refused_deliveries(&w, "row-9-e1").await, 1);
    assert_eq!(
        audits(&w, "run.input_refused", "RUN_INPUT_INVALID rule=required").await,
        1
    );
    assert_eq!(audits(&w, "run.input_refused", SENTINEL).await, 0);

    redeliver(&w, "row-9-e1").await;
    w.dispatcher.tick().await;
    assert_eq!(runs_named(&w, "row-9-strict").await, 0);
    assert_eq!(
        runs_named(&w, "row-9-free").await,
        1,
        "the run of the other is not started twice"
    );
    assert_eq!(refused_deliveries(&w, "row-9-e1").await, 1);
    assert_eq!(
        audits(&w, "run.input_refused", "RUN_INPUT_INVALID").await,
        1,
        "a redelivery audits nothing again"
    );

    publish_event(&w, "row-9-e2", &json!({"orderId": "o-1"})).await;
    w.dispatcher.tick().await;
    assert_eq!(
        runs_named(&w, "row-9-strict").await,
        1,
        "a matching envelope starts the run"
    );
}

/// Row 9 with a schema outside the subset: the same record, with its own reason.
#[tokio::test]
async fn row_9_an_unusable_schema_is_recorded_with_its_reason() {
    let w = world().await;
    let mut unusable = triggered(
        "row-9-unusable",
        json!([{"kind": "event", "eventType": "order.created"}]),
    );
    unusable["inputSchema"] = unusable_schema();
    publish_in(&w, InputValidationMode::Report, &unusable).await;
    publish_event(&w, "row-9-u1", &json!({"orderId": "o-1"})).await;
    w.dispatcher.tick().await;
    assert_eq!(runs_named(&w, "row-9-unusable").await, 0);
    assert_eq!(refused_deliveries(&w, "row-9-u1").await, 1);
    assert_eq!(
        audits(&w, "run.input_refused", "RUN_INPUT_SCHEMA_UNUSABLE").await,
        1
    );
}

/// Row 10: a cron trigger whose version refuses `{event, data: {scheduledAt, firedAt}}` starts nothing and audits the
/// tick, naming its key, and the trigger moves on.
#[tokio::test]
async fn a_cron_trigger_whose_input_is_refused_starts_nothing_and_audits_the_tick() {
    let w = world().await;
    let mut cron = triggered(
        "row-10",
        json!([{"kind": "cron", "cron": "*/5 * * * *", "timezone": "UTC"}]),
    );
    cron["inputSchema"] = needs_order();
    w.publish(cron).await;
    w.dispatcher.tick().await; // plans the first time
    sqlx::query(
        "UPDATE workflow_triggers SET next_fire_at = clock_timestamp() - interval '1 second'",
    )
    .execute(&w.db.pool)
    .await
    .expect("due");
    w.dispatcher.tick().await;
    assert_eq!(runs_named(&w, "row-10").await, 0);
    assert_eq!(audits(&w, "run.input_refused", "at tick").await, 1);
    let moved: bool =
        sqlx::query_scalar("SELECT next_fire_at > clock_timestamp() FROM workflow_triggers")
            .fetch_one(&w.db.pool)
            .await
            .expect("next");
    assert!(moved, "the tick is acted on, not retried");
}

/// T043-21 for row 9: no room in the validator is transient: the event is retried and nothing is recorded; once there is
/// room the run starts.
#[tokio::test]
async fn row_9_no_room_retries_the_event_and_records_nothing() {
    let w = world().await;
    let mut strict = triggered(
        "row-9-busy",
        json!([{"kind": "event", "eventType": "order.created"}]),
    );
    strict["inputSchema"] = needs_order();
    w.publish(strict).await;
    let busy = Arc::new(Scripted::answering([Err(SchemaRefusal::Backstop)]));
    let dispatcher = EventDispatcher::new(
        w.store.clone(),
        Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        w.cache.clone(),
        EventDispatchSettings::default(),
    )
    .with_run_inputs(Arc::new(RunInputGate::new(
        busy,
        InputValidationMode::Enforce,
    )));
    publish_event(&w, "row-9-b1", &json!({"orderId": "o-1"})).await;
    let round = dispatcher.tick().await;
    assert_eq!(round.retried, 1);
    assert_eq!(refused_deliveries(&w, "row-9-b1").await, 0);
    assert_eq!(runs_named(&w, "row-9-busy").await, 0);
    redeliver(&w, "row-9-b1").await;
    dispatcher.tick().await;
    assert_eq!(runs_named(&w, "row-9-busy").await, 1);
}

/// T043-22 in the engine: in `report` mode the refused envelope still starts the run, and the refusal is audited once
/// with it, across a redelivery.
#[tokio::test]
async fn report_mode_starts_the_triggered_run_and_audits_the_refusal_once() {
    let w = world().await;
    let mut strict = triggered(
        "row-9-report",
        json!([{"kind": "event", "eventType": "order.created"}]),
    );
    strict["inputSchema"] = needs_order();
    w.publish(strict).await;
    let reporting = EventDispatcher::new(
        w.store.clone(),
        Engine::new(JsonEvaluator),
        sagat_application::entropy::SystemEntropy,
        w.cache.clone(),
        EventDispatchSettings::default(),
    )
    .with_run_inputs(RunInputValidator::gate(InputValidationMode::Report));
    publish_event(&w, "row-9-r1", &json!({"amount": 2})).await;
    reporting.tick().await;
    redeliver(&w, "row-9-r1").await;
    reporting.tick().await;
    assert_eq!(runs_named(&w, "row-9-report").await, 1);
    assert_eq!(refused_deliveries(&w, "row-9-r1").await, 0);
    assert_eq!(
        audits(&w, "run.input_refused", "reported: RUN_INPUT_INVALID").await,
        1
    );
}

/// CUJ 3: the event a refusing trigger starts nothing for still wakes the step that waits for it.
#[tokio::test]
async fn row_9_a_refused_trigger_does_not_keep_a_wait_for_the_same_event_from_waking() {
    use crate::support::engine::{wait_node, with};

    let w = world().await;
    let mut strict = triggered(
        "row-9-wait-strict",
        json!([{"kind": "event", "eventType": "order.created"}]),
    );
    strict["inputSchema"] = needs_order();
    w.publish(strict).await;
    let (waiter, _) = w
        .publish(document(
            "row-9-waiter",
            "wait",
            vec![
                with(
                    wait_node("wait", "order.created", "input.key", 300_000, None),
                    "next",
                    json!("after"),
                ),
                activity("after"),
            ],
        ))
        .await;
    let run = w.start(waiter, &json!({"key": "c-1"})).await;
    w.events
        .publish(
            &w.admin,
            PublishRequest {
                namespace: "payments".to_owned(),
                event_id: Some(EventId::parse("row-9-w1").expect("id")),
                event_type: EventType::parse("order.created").expect("type"),
                subject: None,
                correlation_key: Some(
                    sagat_domain::execution::CorrelationKey::parse("c-1").expect("key"),
                ),
                occurred_at: None,
                data: br#"{"amount": 1}"#.to_vec(),
                dedupe_ttl: None,
            },
            &ctx(),
        )
        .await
        .expect("published");
    let round = w.dispatcher.tick().await;
    assert_eq!(round.steps_woken, 1, "the wait is woken");
    assert_eq!(runs_named(&w, "row-9-wait-strict").await, 0);
    assert_eq!(refused_deliveries(&w, "row-9-w1").await, 1);
    assert_ne!(
        w.step_status(run, "wait").await,
        Some(sagat_domain::execution::StepStatus::Waiting)
    );
}
