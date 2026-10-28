//! Publicly published scoped event waits use the production receiver and scheduler.
use crate::support::{
    bounded,
    scope_flow::{Flow, activity, collection_document, node, retry},
};
use sagat_application::{
    entropy::SystemEntropy,
    execution::{DefinitionCache, Engine},
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_scheduler::{EventDispatchSettings, EventDispatcher};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub(super) fn document(name: &str, prefix: bool, handled_timeout: bool, timeout_ms: u64) -> Value {
    let mut wait = json!({
        "id":"confirmed", "type":"waitEvent", "eventType":"payment.confirmed",
        "correlationExpr":"item.sku", "timeoutMs":timeout_ms, "lookbackMs":60000,
        "next":"returned"
    });
    if handled_timeout {
        wait["onTimeout"] = json!("returned");
    }
    let mut nodes = Vec::new();
    if prefix {
        nodes.push(activity(
            "reserve",
            "line.reserve",
            "confirmed",
            json!({
                "compensateWith":"line.release", "compensationRetry":retry()
            }),
        ));
    }
    nodes.push(wait);
    nodes.push(json!({"id":"returned", "type":"return", "result":"item.sku"}));
    collection_document(name, nodes, 2)
}

async fn publish(flow: &Flow, key: &str, id: &str) {
    let response = flow.api.as_user(reqwest::Method::POST, "/events", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .json(&json!({"eventId":id,"type":"payment.confirmed","correlationKey":key,"data":{"accepted":key}}))
        .send().await.expect("public event admission");
    assert_eq!(response.status(), reqwest::StatusCode::ACCEPTED);
}

fn receiver(
    flow: &Flow,
    scan_batch: u32,
) -> EventDispatcher<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy> {
    EventDispatcher::new(
        flow.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        DefinitionCache::new(Arc::new(JsonIrCodec), 8),
        EventDispatchSettings {
            batch: 8,
            fan_out: 1,
            scan_batch,
            ..EventDispatchSettings::default()
        },
    )
}

async fn waiting(flow: &Flow) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND waiting_kind='EVENT'",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("durable scoped subscriptions")
}

async fn no_slots(flow: &Flow) {
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND slot_held",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("settled iteration slots");
    assert_eq!(held, 0);
    assert!(flow.poll().await.is_empty());
    flow.verify().await;
}

async fn delivered(prefix: bool) {
    let name = format!("public-collection-event-{prefix}");
    let flow = Flow::start(
        &name,
        document(&name, prefix, true, 60000),
        json!([{"sku":"first"},{"sku":"second"}]),
    )
    .await;
    flow.drive().await;
    if prefix {
        // Finish inbox dispatch before subscriptions exist. Only durable lookback can recover it.
        publish(&flow, "second", "public-second").await;
        publish(&flow, "first", "public-first").await;
        let dispatched = receiver(&flow, 0).tick().await;
        assert_eq!(
            (
                dispatched.dispatched,
                dispatched.steps_woken,
                dispatched.retried
            ),
            (2, 0, 0)
        );
        let tasks = flow.poll().await;
        assert_eq!(tasks.len(), 2);
        for task in tasks {
            assert_eq!(node(&task), "reserve");
            flow.complete(&task, json!({"held":true})).await;
        }
        flow.drive().await;
    }
    assert_eq!(waiting(&flow).await, 2);
    assert!(
        flow.poll().await.is_empty(),
        "event waits own no worker lease"
    );
    flow.verify().await;
    if !prefix {
        publish(&flow, "second", "public-second").await;
        publish(&flow, "first", "public-first").await;
    }
    let delivered = receiver(&flow, 8).tick().await;
    assert_eq!((delivered.steps_woken, delivered.retried), (2, 0));
    assert_eq!(receiver(&flow, 8).tick().await.steps_woken, 0);
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    let rows: Vec<(Vec<i32>,Vec<u8>)> = sqlx::query_as("SELECT s.instance_path_indices,p.content FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.node='confirmed' ORDER BY s.instance_path_indices")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_all(&flow.api.db.pool).await.expect("exact retained event values");
    assert_eq!(rows.len(), 2);
    for (index, bytes) in rows {
        let value: Value = serde_json::from_slice(&bytes).expect("canonical event JSON");
        assert_eq!(
            value["data"]["accepted"],
            if index == [0] { "first" } else { "second" }
        );
    }
    let aggregate: Vec<u8> = sqlx::query_scalar("SELECT p.content FROM iteration_scopes s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.result_digest WHERE s.tenant_id=$1 AND s.run_id=$2")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("ordered aggregate");
    assert_eq!(
        serde_json::from_slice::<Value>(&aggregate).expect("aggregate JSON"),
        json!(["first", "second"])
    );
    let receipts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("single receipt per member");
    assert_eq!(receipts, 2);
    no_slots(&flow).await;
}

#[tokio::test]
async fn public_event_entries_finish_in_input_order_with_scoped_receipts() {
    bounded::within(
        "public event entry lifecycle",
        Duration::from_secs(120),
        delivered(false),
    )
    .await;
}

#[tokio::test]
async fn public_activity_event_bodies_recover_already_dispatched_events_with_a_cold_receiver() {
    bounded::within(
        "public event lookback lifecycle",
        Duration::from_secs(120),
        delivered(true),
    )
    .await;
}

#[tokio::test]
async fn public_event_deadlines_route_without_worker_tasks() {
    let name = "public-collection-event-deadlines";
    let flow = Flow::start(
        name,
        document(name, false, true, 1),
        json!([{"sku":"first"},{"sku":"second"}]),
    )
    .await;
    bounded::within(
        "public event timeout lifecycle",
        Duration::from_secs(60),
        async {
            loop {
                flow.drive().await;
                if flow.run_status().await == "SUCCEEDED" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    let outcomes:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM run_events WHERE tenant_id=$1 AND run_id=$2 AND event_type='wait.timed_out'),(SELECT count(*) FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2)")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("timeout outcomes");
    assert_eq!(outcomes, (2, 0));
    no_slots(&flow).await;
}

#[tokio::test]
async fn cancelling_public_event_entries_retires_slots_and_refuses_late_events() {
    let name = "public-collection-event-cancel";
    let flow = Flow::start(
        name,
        document(name, false, true, 60000),
        json!([{"sku":"first"},{"sku":"second"}]),
    )
    .await;
    flow.drive().await;
    assert_eq!(waiting(&flow).await, 2);
    let (status, _) = flow
        .operate(json!({"action":"CANCEL","reason":"cancel pending scoped event collection"}))
        .await;
    assert_eq!(status, reqwest::StatusCode::ACCEPTED);
    flow.drive().await;
    assert_eq!(flow.run_status().await, "CANCELLED");
    publish(&flow, "first", "cancelled-public-first").await;
    publish(&flow, "second", "cancelled-public-second").await;
    let round = receiver(&flow, 8).tick().await;
    assert_eq!((round.steps_woken, round.retried), (0, 0));
    assert_eq!(waiting(&flow).await, 0);
    no_slots(&flow).await;
}

#[tokio::test]
async fn an_unhandled_public_event_timeout_compensates_each_original_activity() {
    let name = "public-collection-event-timeout-compensation";
    let flow = Flow::start(
        name,
        document(name, true, false, 1),
        json!([{"sku":"first"},{"sku":"second"}]),
    )
    .await;
    flow.drive().await;
    let forward = flow.poll().await;
    assert_eq!(forward.len(), 2);
    for task in forward {
        assert_eq!(node(&task), "reserve");
        flow.complete(&task, json!({"held":true})).await;
    }
    let mut undone = 0;
    bounded::within(
        "unhandled public event timeout compensation",
        Duration::from_secs(60),
        async {
            loop {
                flow.drive().await;
                for task in flow.poll().await {
                    assert_eq!(task.task_kind, "COMPENSATION");
                    assert_eq!(node(&task), "reserve~c");
                    assert_eq!(task.target_node, "reserve");
                    flow.complete(&task, json!({"undone":true})).await;
                    undone += 1;
                }
                if flow.run_status().await == "COMPENSATED" {
                    break;
                }
                tokio::task::yield_now().await;
            }
        },
    )
    .await;
    assert_eq!(undone, 2);
    let code: String = sqlx::query_scalar(
        "SELECT drain_failure_code FROM iteration_scopes WHERE tenant_id=$1 AND run_id=$2",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("original timeout cause");
    assert_eq!(code, "WAIT_TIMED_OUT");
    no_slots(&flow).await;
}

#[tokio::test]
async fn public_controls_route_through_events_to_workers_with_only_their_own_event_value() {
    let name = "public-collection-control-event-activity";
    let mut ir = document(name, false, true, 60000);
    ir["bodies"]["line"]["entrypoint"] = json!("choose");
    ir["bodies"]["line"]["nodes"] = json!([
        {"id":"choose","type":"condition","expression":"item.await","ifTrue":"confirmed","ifFalse":"returned"},
        {"id":"confirmed","type":"waitEvent","eventType":"payment.confirmed","correlationExpr":"item.sku","timeoutMs":60000,"next":"check"},
        {"id":"check","type":"condition","expression":"steps.confirmed.output.data.accepted == item.sku","ifTrue":"notify","ifFalse":"returned"},
        {"id":"notify","type":"activity","activity":"line.notify","next":"returned"},
        {"id":"returned","type":"return","result":"item.sku"}
    ]);
    let flow = Flow::start(
        name,
        ir,
        json!([{"sku":"first","await":true},{"sku":"second","await":true}]),
    )
    .await;
    flow.drive().await;
    assert_eq!(waiting(&flow).await, 2);
    assert!(flow.poll().await.is_empty());
    publish(&flow, "second", "public-routed-second").await;
    publish(&flow, "first", "public-routed-first").await;
    assert_eq!(receiver(&flow, 8).tick().await.steps_woken, 2);
    flow.drive().await;
    let ready = flow.poll().await;
    assert_eq!(ready.len(), 2);
    for task in ready {
        assert_eq!(node(&task), "notify");
        let input: Value =
            serde_json::from_str(&task.input_json).expect("current local worker context");
        assert_eq!(
            input["steps"]["confirmed"]["output"]["data"]["accepted"],
            input["item"]["sku"]
        );
        flow.complete(&task, json!({"notified":true})).await;
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    no_slots(&flow).await;
}
