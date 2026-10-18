//! Public collection completion must honor the already declared End output contract.

use crate::support::scope_flow::{Flow, collection_document};
use serde_json::{Value, json};
use sqlx::Row;

fn decoded(value: Option<Vec<u8>>) -> Option<Value> {
    value.map(|bytes| serde_json::from_slice(&bytes).expect("retained JSON output"))
}

#[tokio::test]
async fn collection_end_retains_owner_aggregate_and_evaluated_run_result() {
    let name = "collection-end-result";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    // A scalar, distinct from the aggregate, proves the declared expression was evaluated.
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0].amount");
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], json!({"amount":17})).await;
    f.drive().await;
    assert_eq!(f.run_status().await, "SUCCEEDED");
    f.verify().await;

    let row = sqlx::query(
        "SELECT scope.lifecycle,aggregate.content AS aggregate,\
         owner_payload.content AS owner_output,end_payload.content AS end_output,\
         run_payload.content AS run_result,\
         owner.output_digest AS owner_digest,scope.result_digest AS aggregate_digest,\
         ended.output_digest AS end_digest,r.result_digest AS run_digest,\
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='run.finished') AS finished_events \
         FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
          AND scope.run_id=r.id AND scope.owner_node='lines' \
          AND scope.owner_path_nodes='{}' AND scope.owner_path_indices='{}' \
         JOIN steps owner ON owner.tenant_id=r.tenant_id AND owner.run_id=r.id \
          AND owner.node='lines' AND owner.instance_path_nodes='{}' \
          AND owner.instance_path_indices='{}' \
         JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
          AND ended.node='done' AND ended.instance_path_nodes='{}' \
          AND ended.instance_path_indices='{}' \
         JOIN payloads aggregate ON aggregate.tenant_id=r.tenant_id \
          AND aggregate.digest=scope.result_digest \
         LEFT JOIN payloads owner_payload ON owner_payload.tenant_id=r.tenant_id \
          AND owner_payload.digest=owner.output_digest \
         LEFT JOIN payloads end_payload ON end_payload.tenant_id=r.tenant_id \
          AND end_payload.digest=ended.output_digest \
         LEFT JOIN payloads run_payload ON run_payload.tenant_id=r.tenant_id \
          AND run_payload.digest=r.result_digest \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual successful collection and terminal output projection");
    let aggregate = json!([{"amount":17}]);
    assert_eq!(row.get::<String, _>("lifecycle"), "SUCCEEDED");
    assert_eq!(decoded(Some(row.get("aggregate"))), Some(aggregate.clone()));
    assert_eq!(row.get::<i64, _>("finished_events"), 1);
    let outputs = (
        decoded(row.get("owner_output")),
        decoded(row.get("end_output")),
        decoded(row.get("run_result")),
    );
    assert_eq!(
        outputs,
        (Some(aggregate), Some(json!(17)), Some(json!(17))),
        "collection aggregate, evaluated End output and Run result must all be retained"
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("owner_digest"),
        Some(row.get::<Vec<u8>, _>("aggregate_digest"))
    );
    assert_eq!(
        row.get::<Option<Vec<u8>>, _>("end_digest"),
        row.get::<Option<Vec<u8>>, _>("run_digest")
    );
}

#[tokio::test]
async fn collection_success_does_not_bypass_the_successor_end_instance_limit() {
    let name = "collection-end-instance-limit";
    let mut document = collection_document(
        name,
        vec![
            json!({"id":"work","type":"activity","activity":"line.reserve","next":"returned"}),
            json!({"id":"returned","type":"return","result":"steps.work.output"}),
        ],
        1,
    );
    document["limits"] = json!({"maxInstances":3});
    document["nodes"][1]["outputExpr"] = json!("steps.lines.output[0].amount");
    let f = Flow::start(name, document, json!([{"id":1}])).await;
    f.drive().await;
    let tasks = f.poll().await;
    assert_eq!(tasks.len(), 1);
    f.complete(&tasks[0], json!({"amount":17})).await;
    f.drive().await;
    assert_eq!(
        f.run_status().await,
        "FAILED",
        "End needs its own admitted instance"
    );
    let boundary: (String, String, bool, bool, i64, Option<String>) = sqlx::query_as(
        "SELECT scope.lifecycle,ended.status,ended.output_digest IS NULL,\
         r.result_digest IS NULL,\
         (SELECT count(*) FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='end.reached'),\
         (SELECT e.detail->>'code' FROM run_events e WHERE e.tenant_id=r.tenant_id \
          AND e.run_id=r.id AND e.event_type='run.stop_requested') \
         FROM runs r JOIN iteration_scopes scope ON scope.tenant_id=r.tenant_id \
          AND scope.run_id=r.id AND scope.owner_node='lines' \
         JOIN steps ended ON ended.tenant_id=r.tenant_id AND ended.run_id=r.id \
          AND ended.node='done' AND ended.instance_path_nodes='{}' \
          AND ended.instance_path_indices='{}' \
         WHERE r.tenant_id=$1 AND r.id=$2",
    )
    .bind(f.tenant.as_uuid())
    .bind(f.run.as_uuid())
    .fetch_one(&f.api.db.pool)
    .await
    .expect("actual collection-success/End-refusal boundary");
    assert_eq!(
        boundary,
        (
            "SUCCEEDED".into(),
            "CANCELLED".into(),
            true,
            true,
            0,
            Some("INSTANCE_LIMIT".into()),
        )
    );
    f.verify().await;
}
