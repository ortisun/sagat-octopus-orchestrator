//! Test-only old-writer phase copied into an immutable `6c44a0e4` source archive by the Q5 runner.
//! All successful collection state is produced through that commit's public API, scheduler, and
//! worker protocol. SQL below only observes the resulting state and builds a phase witness.
#![allow(clippy::expect_used, clippy::panic)]

use std::io::Write as _;

use reqwest::{Method, StatusCode};
use sagat_domain::execution::RunId;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::support::api::json_of;
use crate::support::scope_flow::{Flow, activity};

const OLD_COMMIT: &str = "6c44a0e49670cf19660792cd1518d241e6c99b3b";
const PROJECTION_WITNESS_SQL: &str = r#"WITH selected_runs AS (
  SELECT run.id FROM runs run WHERE run.tenant_id=$1 AND run.id=ANY($2::uuid[])
) SELECT jsonb_build_object(
  'runs',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id),'[]'::jsonb)
    FROM runs row_data JOIN selected_runs selected ON selected.id=row_data.id
    WHERE row_data.tenant_id=$1),
  'usage',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id),'[]'::jsonb)
    FROM run_instance_usage row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'scopes',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.owner_path_nodes,row_data.owner_path_indices,row_data.owner_node),'[]'::jsonb)
    FROM iteration_scopes row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'steps',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.instance_path_nodes,row_data.instance_path_indices,row_data.node),'[]'::jsonb)
    FROM steps row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'events',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.seq),'[]'::jsonb)
    FROM run_events row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'item_sources',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.scope_id,row_data.iteration_index),'[]'::jsonb)
    FROM collection_scope_items row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'iterations',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.scope_id,row_data.iteration_index),'[]'::jsonb)
    FROM collection_iterations row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'checkpoints',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.scope_id,row_data.iteration_index),'[]'::jsonb)
    FROM iteration_checkpoints row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'payloads',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY encode(row_data.digest,'hex')),'[]'::jsonb)
    FROM payloads row_data WHERE row_data.tenant_id=$1 AND row_data.digest IN (
      SELECT run.input_digest FROM runs run JOIN selected_runs selected ON selected.id=run.id WHERE run.tenant_id=$1
      UNION SELECT step.output_digest FROM steps step JOIN selected_runs selected ON selected.id=step.run_id WHERE step.tenant_id=$1
      UNION SELECT scope.snapshot_digest FROM iteration_scopes scope JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT scope.result_digest FROM iteration_scopes scope JOIN selected_runs selected ON selected.id=scope.run_id WHERE scope.tenant_id=$1
      UNION SELECT item.item_digest FROM collection_scope_items item JOIN selected_runs selected ON selected.id=item.run_id WHERE item.tenant_id=$1
      UNION SELECT member.item_digest FROM collection_iterations member JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT member.result_digest FROM collection_iterations member JOIN selected_runs selected ON selected.id=member.run_id WHERE member.tenant_id=$1
      UNION SELECT checkpoint.result_digest FROM iteration_checkpoints checkpoint JOIN selected_runs selected ON selected.id=checkpoint.run_id WHERE checkpoint.tenant_id=$1
      UNION SELECT event.payload_digest FROM run_events event JOIN selected_runs selected ON selected.id=event.run_id WHERE event.tenant_id=$1)),
  'source_blobs',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.namespace_id,row_data.id),'[]'::jsonb)
    FROM blobs row_data WHERE row_data.tenant_id=$1 AND (row_data.namespace_id,row_data.id) IN (
      SELECT scope.namespace_id,scope.source_blob_id FROM iteration_scopes scope
      JOIN selected_runs selected ON selected.id=scope.run_id
      WHERE scope.tenant_id=$1 AND scope.source_blob_id IS NOT NULL)),
  'workflow_versions',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id),'[]'::jsonb)
    FROM workflow_versions row_data WHERE row_data.tenant_id=$1 AND row_data.id IN (
      SELECT run.workflow_version_id FROM runs run JOIN selected_runs selected ON selected.id=run.id WHERE run.tenant_id=$1)),
  'certifications',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.run_id,row_data.owner_node,row_data.iteration_index),'[]'::jsonb)
    FROM leaf_collection_success_certifications row_data JOIN selected_runs selected ON selected.id=row_data.run_id
    WHERE row_data.tenant_id=$1),
  'outbox',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data) ORDER BY row_data.id),'[]'::jsonb)
    FROM event_outbox row_data JOIN selected_runs selected ON selected.id=row_data.aggregate_id
    WHERE row_data.tenant_id=$1))"#;

fn document() -> Value {
    let roots = (0..11)
        .map(|index| {
            let next = if index == 10 {
                "done".to_owned()
            } else {
                format!("c{}", index + 1)
            };
            json!({
                "id": format!("c{index}"), "type": "forEach", "items": "input.lines",
                "body": "line", "result": "bodyResult", "maxItems": 1,
                "maxConcurrency": 1, "next": next
            })
        })
        .chain(std::iter::once(json!({"id": "done", "type": "end"})))
        .collect::<Vec<_>>();
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": "q5-frozen-legacy-output"}, "entrypoint": "c0",
        "bodies": {"line": {"entrypoint": "work", "nodes": [
            activity("work", "line.reserve", "returned", json!({})),
            {"id": "returned", "type": "return", "result": "steps.work.output"}
        ]}},
        "nodes": roots
    })
}

async fn start_run(flow: &Flow, workflow: Uuid) -> RunId {
    let response = flow
        .api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow.to_string(), "input": {"lines": [1]}}))
        .send()
        .await
        .expect("start another old-writer run");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    json_of(response).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("valid run id")
}

async fn finish_then_hold(flow: &Flow, run: RunId, successes: usize, output: &Value) {
    for owner in 0..successes {
        assert!(flow.drive().await > 0, "c{owner} becomes runnable");
        let tasks = flow.poll().await;
        let task = tasks
            .iter()
            .find(|task| task.run_id == run.to_string())
            .unwrap_or_else(|| panic!("old run {run} has a task at c{owner}"));
        flow.complete(task, output.clone()).await;
    }
    assert!(
        flow.drive().await > 0,
        "the later collection becomes runnable"
    );
    let tasks = flow.poll().await;
    assert!(
        tasks.iter().any(|task| task.run_id == run.to_string()),
        "a genuine later scoped lease is retained"
    );
}

async fn projection_witness(flow: &Flow, runs: &[RunId]) -> String {
    let ids = runs.iter().map(|run| (*run).as_uuid()).collect::<Vec<_>>();
    let value: Value = sqlx::query_scalar(PROJECTION_WITNESS_SQL)
        .bind(flow.tenant.as_uuid())
        .bind(ids)
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("bounded canonical old projection witness");
    let digest = Sha256::digest(serde_json::to_vec(&value).expect("canonical witness JSON"));
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

async fn assert_shape(flow: &Flow, run: RunId, successes: i64) {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT count(*)::bigint,
          count(*) FILTER (WHERE owner.output_digest IS NULL)::bigint,
          count(DISTINCT encode(scope.result_digest,'hex'))::bigint
         FROM iteration_scopes scope JOIN steps owner
           ON owner.tenant_id=scope.tenant_id AND owner.run_id=scope.run_id
          AND owner.instance_path_nodes='{}'::text[]
          AND owner.instance_path_indices='{}'::integer[] AND owner.node=scope.owner_node
         WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.lifecycle='SUCCEEDED'
           AND scope.owner_path_nodes='{}'::text[]
           AND scope.owner_path_indices='{}'::integer[]",
    )
    .bind(flow.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("old collection shape");
    assert_eq!(row, (successes, successes, 1));
    let held: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND status='LEASED'
         AND cardinality(instance_path_nodes)>0",
    )
    .bind(flow.tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("held later lease");
    assert_eq!(held, 1);
}

#[tokio::test]
async fn frozen_old_collection_output_producer() {
    assert_eq!(env!("SAGAT_Q5_SOURCE_COMMIT"), OLD_COMMIT);
    assert!(
        std::env::var_os("SAGAT_TEST_DATABASE_URL").is_some(),
        "the runner must supply its owned external PostgreSQL admin URL"
    );
    let receipt_path = std::env::var_os("SAGAT_Q5_PHASE_RECEIPT")
        .map(std::path::PathBuf::from)
        .expect("runner receipt path");

    let flow = Flow::start("q5-frozen-legacy-output", document(), json!([1])).await;
    let quota = flow
        .api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": 32}))
        .send()
        .await
        .expect("raise held-fixture capacity");
    assert_eq!(quota.status(), StatusCode::OK);
    let workflow: Uuid = sqlx::query_scalar("SELECT workflow_id FROM runs WHERE id=$1")
        .bind(flow.run.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("published workflow");
    let alias_workspace = flow
        .api
        .create_workspace("alice", "acme", "q5-corruption-alias")
        .await;
    let alias_namespace = flow
        .api
        .create_namespace("alice", "acme", &alias_workspace, "q5-alias", "dev")
        .await;

    let overlay = flow.run;
    let equal = json!({"same": true});
    finish_then_hold(&flow, overlay, 2, &equal).await;
    let mut small = Vec::new();
    for output in [
        json!({"case": "missing_owner"}),
        json!({"case": "wrong_namespace"}),
        json!({"case": "bad_metadata"}),
        json!({"case": "missing_payload"}),
        json!({"case": "bad_hash"}),
    ] {
        let run = start_run(&flow, workflow).await;
        finish_then_hold(&flow, run, 2, &output).await;
        small.push(run);
    }
    let over_budget = start_run(&flow, workflow).await;
    let large = Value::String("x".repeat(209_998));
    assert_eq!(
        serde_json::to_vec(&large).expect("large JSON").len(),
        210_000
    );
    finish_then_hold(&flow, over_budget, 10, &large).await;

    assert_shape(&flow, overlay, 2).await;
    for run in &small {
        assert_shape(&flow, *run, 2).await;
    }
    assert_shape(&flow, over_budget, 10).await;
    let all = [
        overlay,
        small[0],
        small[1],
        small[2],
        small[3],
        small[4],
        over_budget,
    ];
    let receipt = json!({
        "source_commit": OLD_COMMIT,
        "database_url": flow.api.db.url.clone(),
        "runtime_database_url": flow.api.db.url_for(
            crate::support::RUNTIME_ROLE,
            crate::support::RUNTIME_ROLE_PASSWORD,
        ),
        "database_name": flow.api.db.name.clone(),
        "tenant": flow.tenant.to_string(),
        "alias_namespace": alias_namespace,
        "overlay": overlay.to_string(),
        "missing_owner": small[0].to_string(),
        "wrong_namespace": small[1].to_string(),
        "bad_metadata": small[2].to_string(),
        "missing_payload": small[3].to_string(),
        "bad_hash": small[4].to_string(),
        "over_budget": over_budget.to_string(),
        "producer_witness_sha256": projection_witness(&flow, &all).await,
    });
    let temporary = receipt_path.with_extension("tmp");
    let bytes = serde_json::to_vec(&receipt).expect("serialize receipt");
    let mut file = std::fs::File::create(&temporary).expect("create private receipt");
    file.write_all(&bytes).expect("write receipt");
    file.sync_all().expect("fsync receipt");
    std::fs::rename(temporary, receipt_path).expect("publish receipt atomically");
}
