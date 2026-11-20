//! Control entry admission owns an iteration without fabricating a worker activity or hint.
use crate::support::scope_flow::{Flow, activity, collection_document, iteration, node};
use serde_json::json;

#[tokio::test]
async fn a_control_only_collection_finishes_without_any_worker_lease_or_task_hint() {
    let name = "collection-control-entry-only";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"choose", "type":"condition", "expression":"item > 0", "ifTrue":"returned", "ifFalse":"returned"}),
            json!({"id":"returned", "type":"return", "result":"item"}),
        ],
        2,
    );
    document["limits"] = json!({"maxInstances":6, "maxDepth":1});
    let flow = Flow::start(name, document, json!([1, -1])).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    assert!(flow.poll().await.is_empty());
    let evidence: (i64,i64,i64,i32,i32,i32) = sqlx::query_as(
        "SELECT \
           (SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND (attempt<>0 OR generation<>0 OR activity IS NOT NULL)), \
           (SELECT count(*) FROM event_outbox WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready'), \
           (SELECT count(*) FROM collection_iterations WHERE tenant_id=$1 AND run_id=$2 AND activity IS NULL AND outbox_id IS NULL AND NOT slot_held AND remaining_reservations=0), \
           admitted_instances,reserved_instances,retired_reservations FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2",
    ).bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("control-only ownership and lifetime");
    assert_eq!(evidence, (0, 0, 2, 6, 0, 0));
    flow.verify().await;
}

#[tokio::test]
async fn a_switch_entry_dispatches_only_the_selected_real_activity() {
    let name = "collection-switch-entry-work";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"choose", "type":"switch", "cases":[{"id":"positive", "when":"item > 0", "target":"work"}], "default":"returned"}),
            activity("work", "line.notify", "returned", json!({})),
            json!({"id":"returned", "type":"return", "result":"item"}),
        ],
        2,
    );
    document["limits"] = json!({"maxInstances":8, "maxDepth":1});
    let flow = Flow::start(name, document, json!([1, -1])).await;
    flow.drive().await;
    let work = flow.poll().await;
    assert_eq!(work.len(), 1);
    assert_eq!((iteration(&work[0]), node(&work[0])), (0, "work".into()));
    assert_eq!(work[0].task_kind, "FORWARD");
    flow.complete(&work[0], json!({"selected":true})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "SUCCEEDED");
    let usage: (i32,i32,i32) = sqlx::query_as("SELECT admitted_instances,reserved_instances,retired_reservations FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("selected branch lifetime");
    assert_eq!(usage, (7, 0, 1));
    flow.verify().await;
}
