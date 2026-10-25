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

const NONFINAL_CLOSED_SOURCE_SQL: &str = r#"WITH selected_scopes AS (
  SELECT scope.* FROM iteration_scopes scope
   WHERE scope.tenant_id=$1 AND scope.run_id=$2
     AND scope.owner_path_nodes='{}'::text[]
     AND scope.owner_path_indices='{}'::integer[]
     AND scope.owner_node=ANY($3::text[])
), selected_events AS (
  SELECT event.* FROM run_events event
   WHERE event.tenant_id=$1 AND event.run_id=$2 AND event.seq <= $4
) SELECT jsonb_build_object(
  'scopes',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.owner_node)
    FROM selected_scopes row_data),
  'steps',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.instance_path_nodes,row_data.instance_path_indices,row_data.node)
    FROM steps row_data WHERE row_data.tenant_id=$1 AND row_data.run_id=$2
      AND ((cardinality(row_data.instance_path_nodes)=0 AND row_data.node=ANY($3::text[]))
        OR row_data.instance_path_nodes[1]=ANY($3::text[]))),
  'item_sources',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_scope_items row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'iterations',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM collection_iterations row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'checkpoints',(SELECT jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.scope_id,row_data.iteration_index)
    FROM iteration_checkpoints row_data JOIN selected_scopes scope
      ON scope.tenant_id=row_data.tenant_id AND scope.run_id=row_data.run_id
     AND scope.scope_id=row_data.scope_id),
  'events',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY row_data.seq)
    FROM selected_events row_data),
  'payloads',(SELECT jsonb_agg(to_jsonb(row_data) ORDER BY encode(row_data.digest,'hex'))
    FROM payloads row_data WHERE row_data.tenant_id=$1 AND row_data.digest IN (
      SELECT scope.snapshot_digest FROM selected_scopes scope
      UNION SELECT scope.result_digest FROM selected_scopes scope
      UNION SELECT step.output_digest FROM steps step
        WHERE step.tenant_id=$1 AND step.run_id=$2
          AND ((cardinality(step.instance_path_nodes)=0 AND step.node=ANY($3::text[]))
            OR step.instance_path_nodes[1]=ANY($3::text[]))
      UNION SELECT item.item_digest FROM collection_scope_items item JOIN selected_scopes scope
        ON scope.tenant_id=item.tenant_id AND scope.run_id=item.run_id
       AND scope.scope_id=item.scope_id
      UNION SELECT member.result_digest FROM collection_iterations member JOIN selected_scopes scope
        ON scope.tenant_id=member.tenant_id AND scope.run_id=member.run_id
       AND scope.scope_id=member.scope_id
      UNION SELECT checkpoint.result_digest FROM iteration_checkpoints checkpoint
        JOIN selected_scopes scope ON scope.tenant_id=checkpoint.tenant_id
         AND scope.run_id=checkpoint.run_id AND scope.scope_id=checkpoint.scope_id
      UNION SELECT event.payload_digest FROM selected_events event)),
  'certifications',(SELECT COALESCE(jsonb_agg(to_jsonb(row_data)
      ORDER BY row_data.owner_node,row_data.iteration_index),'[]'::jsonb)
    FROM leaf_collection_success_certifications row_data
    WHERE row_data.tenant_id=$1 AND row_data.run_id=$2
      AND row_data.owner_node=ANY($3::text[]))
)"#;

// Separate format and public fixture: unlike the original matrix, no old forward lease is live.
const NONFINAL_WORKER: &str = "nonfinal-return-worker";
const NONFINAL_NAME: &str = "q5-nonfinal-return";

fn nonfinal_document() -> Value {
    let roots = (0..11)
        .map(|index| {
            json!({
                "id": format!("c{index}"), "type": "forEach", "items": "input.lines",
                "body": if index == 10 { "targetLine" } else { "legacyLine" },
                "result": "bodyResult", "maxItems": 2, "maxConcurrency": 1,
                "next": if index == 10 { "done".to_owned() } else { format!("c{}", index + 1) }
            })
        })
        .chain(std::iter::once(json!({"id":"done","type":"end"})))
        .collect::<Vec<_>>();
    json!({
        "apiVersion":"sagat-octopus-orchestrator.io/v1", "kind":"Workflow",
        "metadata":{"name":NONFINAL_NAME}, "entrypoint":"c0", "nodes":roots,
        "bodies":{
            "legacyLine":{"entrypoint":"legacyWork","nodes":[
                {"id":"legacyWork","type":"activity","activity":"line.reserve","next":"legacyReturned"},
                {"id":"legacyReturned","type":"return","result":"steps.legacyWork.output"}
            ]},
            "targetLine":{"entrypoint":"work","nodes":[
                {"id":"work","type":"activity","activity":"line.reserve","compensateWith":"line.release",
                 "compensationRetry":{"maxAttempts":2,"initialDelayMs":100,"maxDelayMs":100,"strategy":"fixed","jitter":false},
                 "next":"returned"},
                {"id":"returned","type":"return","result":"steps.work.output"}
            ]}
        }
    })
}

fn nonfinal_sha(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("canonical private witness"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn nonfinal_source(pool: &sqlx::PgPool, tenant: Uuid, run: Uuid, cutoff: i64) -> String {
    let closed: Value = sqlx::query_scalar(NONFINAL_CLOSED_SOURCE_SQL)
        .bind(tenant)
        .bind(run)
        .bind((0..10).map(|i| format!("c{i}")).collect::<Vec<_>>())
        .bind(cutoff)
        .fetch_one(pool)
        .await
        .expect("immutable closed old source");
    let target: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'version',(SELECT to_jsonb(v) FROM workflow_versions v WHERE v.tenant_id=r.tenant_id AND v.id=r.workflow_version_id),
      'captured',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.iteration_index) FROM collection_scope_items i
          JOIN iteration_scopes s ON s.tenant_id=i.tenant_id AND s.run_id=i.run_id AND s.scope_id=i.scope_id
          WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id AND s.owner_node='c10'),
      'work',(SELECT to_jsonb(w) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id
          AND w.instance_path_nodes=ARRAY['c10']::text[] AND w.instance_path_indices=ARRAY[0]::integer[] AND w.node='work'),
      'work_payload',(SELECT to_jsonb(p) FROM steps w JOIN payloads p ON p.tenant_id=w.tenant_id AND p.digest=w.output_digest
          WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes=ARRAY['c10']::text[]
            AND w.instance_path_indices=ARRAY[0]::integer[] AND w.node='work'),
      'work_events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e
          WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.instance_path_nodes=ARRAY['c10']::text[]
            AND e.instance_path_indices=ARRAY[0]::integer[] AND e.node='work'),
      'work_receipt',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.scope,c.idempotency_key) FROM command_dedupe c
          WHERE c.tenant_id=r.tenant_id AND c.scope LIKE $3)) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2"#)
        .bind(tenant).bind(run).bind(format!("step:{run}:scoped:3:c10:0;4:work:%"))
        .fetch_one(pool).await.expect("old target source and actual report receipt");
    assert!(
        target["work_receipt"]
            .as_array()
            .is_some_and(|rows| rows.len() == 1),
        "one actual old forward report receipt"
    );
    nonfinal_sha(&json!({"closed":closed,"target":target}))
}

async fn nonfinal_shape(pool: &sqlx::PgPool, tenant: Uuid, run: Uuid) -> Value {
    let state: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'head',r.run_version,'workflow_version',r.workflow_version_id,'scope_id',s.scope_id,
      'running',r.status='RUNNING' AND r.stop_reason IS NULL AND r.stop_code IS NULL AND r.parent_run_id IS NULL AND r.depth=0,
      'L',u.logical_output_bytes,
      'stored',(SELECT sum(octet_length(p.content)) FROM steps w JOIN payloads p ON p.tenant_id=w.tenant_id AND p.digest=w.output_digest WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id),
      'V',(SELECT sum(octet_length(p.content)) FROM iteration_scopes c JOIN steps o ON o.tenant_id=c.tenant_id AND o.run_id=c.run_id AND o.node=c.owner_node AND cardinality(o.instance_path_nodes)=0
          JOIN payloads p ON p.tenant_id=c.tenant_id AND p.digest=c.result_digest WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED' AND o.output_digest IS NULL),
      'closed_scopes',(SELECT count(*) FROM iteration_scopes c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED' AND c.item_count=2 AND c.completed_count=2 AND c.active_count=0),
      'closed_owners',(SELECT count(*) FROM steps o WHERE o.tenant_id=r.tenant_id AND o.run_id=r.id AND cardinality(o.instance_path_nodes)=0 AND o.status='SUCCEEDED' AND o.routed AND o.output_digest IS NULL),
      'historical_work',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.node='legacyWork' AND w.status='SUCCEEDED' AND w.routed AND w.output_digest IS NOT NULL),
      'historical_members',(SELECT count(*) FROM collection_iterations m JOIN iteration_scopes c ON c.tenant_id=m.tenant_id AND c.run_id=m.run_id AND c.scope_id=m.scope_id WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED'),
      'checkpoint_coordinates',(SELECT jsonb_agg(jsonb_build_array(c.owner_node,m.iteration_index) ORDER BY c.owner_node,m.iteration_index)
          FROM iteration_scopes c JOIN collection_iterations m ON m.tenant_id=c.tenant_id AND m.run_id=c.run_id AND m.scope_id=c.scope_id
          JOIN iteration_checkpoints k ON k.tenant_id=m.tenant_id AND k.run_id=m.run_id AND k.scope_id=m.scope_id AND k.iteration_index=m.iteration_index
          JOIN steps fw ON fw.tenant_id=m.tenant_id AND fw.run_id=m.run_id AND fw.step_id=m.entry_instance_id
          JOIN steps ret ON ret.tenant_id=m.tenant_id AND ret.run_id=m.run_id AND ret.step_id=k.return_instance_id
          JOIN run_events e ON e.tenant_id=k.tenant_id AND e.run_id=k.run_id AND e.seq=k.journal_sequence
          WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED'
            AND m.lifecycle='CHECKPOINTED' AND NOT m.slot_held AND m.checkpoint_sequence=k.journal_sequence
            AND m.return_instance_id=k.return_instance_id AND m.result_digest=k.result_digest
            AND fw.status='SUCCEEDED' AND fw.routed AND fw.output_digest=k.result_digest AND ret.status='SUCCEEDED'
            AND cardinality(c.owner_path_nodes)=0 AND cardinality(c.owner_path_indices)=0
            AND fw.node='legacyWork' AND fw.instance_path_nodes=ARRAY[c.owner_node]::text[] AND fw.instance_path_indices=ARRAY[m.iteration_index]::integer[]
            AND ret.node='legacyReturned' AND ret.instance_path_nodes=ARRAY[c.owner_node]::text[] AND ret.instance_path_indices=ARRAY[m.iteration_index]::integer[]
            AND m.entry_node=fw.node AND m.entry_path_nodes=fw.instance_path_nodes AND m.entry_path_indices=fw.instance_path_indices
            AND m.return_node=ret.node AND m.return_path_nodes=ret.instance_path_nodes AND m.return_path_indices=ret.instance_path_indices
            AND e.event_type='collection.iteration_checkpointed' AND e.node=c.owner_node
            AND e.detail->>'index'=m.iteration_index::text AND e.detail->>'result_digest'=encode(k.result_digest,'hex')),
      'finish_route_pairs',(SELECT count(*) FROM iteration_scopes c
          JOIN run_events f ON f.tenant_id=c.tenant_id AND f.run_id=c.run_id AND f.seq=c.finish_sequence
          JOIN run_events routed ON routed.tenant_id=c.tenant_id AND routed.run_id=c.run_id AND routed.seq=c.finish_sequence+1
          WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED'
            AND f.event_type='collection.finished' AND f.node=c.owner_node AND f.detail->>'outcome'='succeeded'
            AND f.detail->>'result_digest'=encode(c.result_digest,'hex')
            AND routed.event_type='step.routed' AND routed.node=c.owner_node
            AND cardinality(f.instance_path_nodes)=0 AND cardinality(routed.instance_path_nodes)=0),
      'historical_returns',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.node='legacyReturned' AND w.status='SUCCEEDED'),
      'checkpoints',(SELECT count(*) FROM iteration_checkpoints c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id),
      'open',s.lifecycle='OPEN' AND s.item_count=2 AND s.max_concurrency=1 AND s.next_admission_index=1 AND s.admitted_count=1 AND s.completed_count=0 AND s.active_count=1 AND s.result_digest IS NULL AND s.finish_sequence IS NULL,
      'held',(SELECT count(*) FROM collection_iterations m WHERE m.tenant_id=r.tenant_id AND m.run_id=r.id AND m.scope_id=s.scope_id AND m.iteration_index=0 AND m.lifecycle='ADMITTED' AND m.slot_held AND m.result_digest IS NULL),
      'unadmitted',(SELECT count(*) FROM collection_iterations m WHERE m.tenant_id=r.tenant_id AND m.run_id=r.id AND m.scope_id=s.scope_id AND m.iteration_index=1),
      'unadmitted_steps',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes[1]='c10' AND w.instance_path_indices[1]=1),
      'captured',(SELECT jsonb_agg(convert_from(p.content,'UTF8')::jsonb ORDER BY i.iteration_index) FROM collection_scope_items i JOIN payloads p ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id AND i.scope_id=s.scope_id),
      'target_work',(SELECT count(*) FROM steps w JOIN payloads p ON p.tenant_id=w.tenant_id AND p.digest=w.output_digest WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes=ARRAY['c10']::text[] AND w.instance_path_indices=ARRAY[0]::integer[] AND w.node='work' AND w.status='SUCCEEDED' AND w.routed AND p.content=decode('31','hex')),
      'pending_return',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes=ARRAY['c10']::text[] AND w.instance_path_indices=ARRAY[0]::integer[] AND w.node='returned' AND w.status='PENDING' AND NOT w.routed AND w.attempt=0 AND w.generation=0 AND w.lease_token_hash IS NULL AND w.leased_by IS NULL AND w.lease_expires_at IS NULL AND w.output_digest IS NULL AND w.failure_code IS NULL),
      'live',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.status IN ('LEASED','RUNNING')),
      'sagas',(SELECT count(*) FROM scope_sagas g WHERE g.tenant_id=r.tenant_id AND g.run_id=r.id),
      'certificates',(SELECT count(*) FROM leaf_collection_success_certifications c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id),
      'chains',(SELECT count(*) FROM chain_segments c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id),
      'cutoff',(SELECT max(seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.node='c9' AND cardinality(e.instance_path_nodes)=0 AND e.event_type='step.routed'))
      FROM runs r JOIN run_instance_usage u ON u.tenant_id=r.tenant_id AND u.run_id=r.id
      JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10'
      WHERE r.tenant_id=$1 AND r.id=$2"#)
        .bind(tenant).bind(run).fetch_one(pool).await.expect("genuine old nonfinal Return premise");
    for (key, expected) in [
        ("running", json!(true)),
        ("L", json!(2_100_001)),
        ("stored", json!(2_100_001)),
        ("V", json!(2_100_030)),
        ("closed_scopes", json!(10)),
        ("closed_owners", json!(10)),
        ("historical_work", json!(20)),
        ("historical_returns", json!(20)),
        ("checkpoints", json!(20)),
        ("open", json!(true)),
        ("held", json!(1)),
        ("unadmitted", json!(0)),
        ("unadmitted_steps", json!(0)),
        ("captured", json!([1, 2])),
        ("target_work", json!(1)),
        ("pending_return", json!(1)),
        ("live", json!(0)),
        ("sagas", json!(0)),
        ("certificates", json!(0)),
        ("chains", json!(0)),
    ] {
        assert!(state[key] == expected, "old nonfinal Return premise: {key}");
    }
    assert!(
        state["historical_members"] == 20 && state["finish_route_pairs"] == 10,
        "twenty historical members and ten exact finish/routing journal pairs"
    );
    let coordinates = (0..10)
        .flat_map(|i| [json!([format!("c{i}"), 0]), json!([format!("c{i}"), 1])])
        .collect::<Vec<_>>();
    assert!(
        state["checkpoint_coordinates"] == json!(coordinates),
        "exact twenty CHECKPOINTED coordinates match work, Return, checkpoint and journal identities"
    );
    state
}

#[tokio::test]
async fn frozen_old_nonfinal_return_frontier_producer() {
    nonfinal_produce(false).await;
}

#[tokio::test]
async fn frozen_old_nonfinal_return_frontier_guard_red() {
    nonfinal_produce(true).await;
}

async fn nonfinal_produce(red: bool) {
    use crate::support::{
        api::Options,
        worker::{TENANT, Worker},
    };
    use sagat_application::{entropy::SystemEntropy, execution::Engine};
    use sagat_domain::{execution::ExecutionInstanceId, identity::Role, workflow::NodeId};
    use sagat_scheduler::{
        CollectionAdvance, Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings,
    };
    assert!(
        env!("SAGAT_Q5_SOURCE_COMMIT") == OLD_COMMIT,
        "immutable old production pin"
    );
    let runtime = crate::support::as_runtime_role();
    let role = if runtime { "runtime" } else { "owner" };
    let platform = crate::support::worker::platform_with(Options {
        runtime_role: runtime,
        ..Options::default()
    })
    .await;
    let namespace = platform.namespace_id().await;
    let (status, _) = platform
        .rest(
            Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit":1})),
        )
        .await;
    assert!(status == 200, "explicit public iteration capacity");
    platform
        .grant(platform.tenant, NONFINAL_WORKER, Role::Worker)
        .await;
    let (status, _) = platform
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/{NONFINAL_WORKER}"),
            Some(&json!({"activities":["line.reserve","line.release"]})),
        )
        .await;
    assert!(status == 200, "public exact worker activity binding");
    let workflow = platform.publish(NONFINAL_NAME, &nonfinal_document()).await;
    let run_text = platform.start(&workflow, &json!({"lines":[1,2]})).await;
    let run: RunId = run_text.parse().expect("old run id");
    let api_role: (String, String) =
        sqlx::query_as("SELECT current_user::text,current_database()::text")
            .fetch_one(&platform.api.api_pool)
            .await
            .expect("actual old API/store shared pool identity");
    let owner_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("old inspection role");
    assert!(
        api_role.1 == platform.api.db.name && owner_role != crate::support::RUNTIME_ROLE,
        "owned old database identity"
    );
    assert!(
        api_role.0
            == if runtime {
                crate::support::RUNTIME_ROLE
            } else {
                owner_role.as_str()
            },
        "actual selected old API and scheduler-store role"
    );
    println!(
        "old_nonfinal_role role={role} api={} store={} database={}",
        api_role.0, api_role.0, api_role.1
    );
    let scheduler_owner = format!("nonfinal-return-{role}");
    let placement = Placement::new(scheduler_owner.clone());
    let keeper = Keeper::new(
        platform.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(3600),
        },
    );
    assert!(
        keeper.round().await.is_ok(),
        "old scheduler owns its actual shards"
    );
    let scheduler = Scheduler::new(
        platform.api.executions.clone(),
        Engine::new(sagat_infra_ir::JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: std::time::Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement);
    let mut worker = platform.connect(NONFINAL_WORKER, TENANT).await;
    worker
        .register_version(&["line.reserve", "line.release"], "1.5")
        .await;
    let large = Value::String("x".repeat(104_998));
    let large_bytes = serde_json::to_vec(&large).expect("105000-byte public old output");
    assert!(large_bytes.len() == 105_000, "old individual output width");
    let mut ticks = 0usize;
    let mut forward = None;
    for index in 0..21 {
        let task = loop {
            ticks += 1;
            assert!(ticks <= 256, "bounded old public scheduling transitions");
            assert!(
                scheduler.tick().await.failed == 0,
                "old public scheduler setup succeeds"
            );
            let mut tasks = worker.poll(1).await;
            if let Some(task) = tasks.pop() {
                assert!(tasks.is_empty(), "one old task at a time");
                break task;
            }
        };
        let identity = task.instance.as_ref().expect("old scoped task identity");
        assert!(
            task.run_id == run_text
                && task.activity_name == "line.reserve"
                && identity.instance_path.len() == 1
                && identity.instance_path[0].scope_node_id == format!("c{}", index / 2)
                && identity.instance_path[0].iteration_index == index % 2
                && identity.definition_node_id == if index < 20 { "legacyWork" } else { "work" },
            "exact ordered old genuine scoped work"
        );
        let output = if index < 20 {
            std::str::from_utf8(&large_bytes).expect("JSON UTF8")
        } else {
            "1"
        };
        let request = worker.req(Worker::complete_message(&task, output));
        let ack = match worker.client.complete_step(request).await {
            Ok(r) => r.into_inner(),
            Err(_) => panic!("old genuine successful work completion"),
        };
        assert!(
            !ack.already_applied && ack.step_status == "SUCCEEDED" && ack.run_status == "RUNNING",
            "actual old success without a stop"
        );
        if index == 20 {
            forward = Some((
                Uuid::parse_str(&task.step_id).expect("old forward step id"),
                task.idempotency_key.clone(),
            ));
        }
    }
    // NO old tick/advance follows the final successful work report in the positive producer.
    let state = nonfinal_shape(
        &platform.api.db.pool,
        platform.tenant.as_uuid(),
        run.as_uuid(),
    )
    .await;
    let aggregates: Vec<(String,Vec<u8>)>=sqlx::query_as("SELECT s.owner_node,p.content FROM iteration_scopes s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.result_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.lifecycle='SUCCEEDED' ORDER BY s.owner_node")
        .bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_all(&platform.api.db.pool).await.expect("all ten genuine old aggregate payloads");
    let expected =
        serde_json::to_vec(&json!([large.clone(), large])).expect("actual two-item aggregate");
    assert!(
        aggregates.len() == 10 && expected.len() == 210_003,
        "ten actual 210003-byte aggregate references"
    );
    for (index, (owner, bytes)) in aggregates.iter().enumerate() {
        assert!(
            *owner == format!("c{index}") && *bytes == expected,
            "each old aggregate has the exact ordered two outputs"
        );
    }
    let payloads: Vec<(Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT digest,content FROM payloads WHERE tenant_id=$1 ORDER BY encode(digest,'hex')",
    )
    .bind(platform.tenant.as_uuid())
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("actual old retained payload digests and bytes");
    for (digest, content) in payloads {
        assert!(
            digest == Sha256::digest(&content).to_vec(),
            "every old aggregate, checkpoint and retained work digest matches its actual bytes"
        );
    }
    let cutoff = state["cutoff"].as_i64().expect("actual c9 routing cutoff");
    let source = nonfinal_source(
        &platform.api.db.pool,
        platform.tenant.as_uuid(),
        run.as_uuid(),
        cutoff,
    )
    .await;
    let projection: Value = sqlx::query_scalar(PROJECTION_WITNESS_SQL)
        .bind(platform.tenant.as_uuid())
        .bind(vec![run.as_uuid()])
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("full genuine old projection");
    println!(
        "old_nonfinal_premise L=2100001 V=2100030 E=4200031 closed=10 checkpoints=20 pending_nonfinal_return=true live_forward=0"
    );
    if red {
        let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("target owner"));
        let result = scheduler
            .advance_collection(platform.tenant, run, &owner)
            .await
            .expect("old normal nonfinal Return advance");
        let checkpointed = matches!(result, CollectionAdvance::Checkpointed { index: 0 });
        println!("observed_legacy_nonfinal_return_checkpointed={checkpointed}");
        assert!(
            matches!(result, CollectionAdvance::DrainStarted),
            "old guard RED: expected DrainStarted instead of nonfinal checkpoint"
        );
        return;
    }
    let (forward_step_id, forward_target_key) = forward.expect("actual completed target work");
    let receipt = json!({"format":"legacy-nonfinal-return-frontier-v1","source_commit":OLD_COMMIT,
        "case":"nonfinal-return","role":role,"database_name":platform.api.db.name,"database_url":platform.api.db.url,
        "runtime_database_url":platform.api.db.url_for(crate::support::RUNTIME_ROLE,crate::support::RUNTIME_ROLE_PASSWORD),
        "tenant":platform.tenant.to_string(),"other_tenant":platform.other_tenant.to_string(),"namespace":namespace,
        "run":run.to_string(),"workflow_version":state["workflow_version"],"scope_id":state["scope_id"],
        "scheduler_owner":scheduler_owner,"handoff_run_version":state["head"],"closed_source_cutoff":cutoff,
        "forward_step_id":forward_step_id.to_string(),"forward_target_key":forward_target_key,
        "producer_witness_sha256":nonfinal_sha(&projection),"closed_source_sha256":source});
    assert!(
        receipt.as_object().is_some_and(|o| o.len() == 20),
        "closed twenty-field settled-work format"
    );
    let path = std::path::PathBuf::from(
        std::env::var_os("SAGAT_Q5_PHASE_RECEIPT").expect("private handoff path"),
    );
    let temporary = path.with_extension("tmp");
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .expect("exclusive private handoff");
    file.write_all(&serde_json::to_vec(&receipt).expect("private handoff JSON"))
        .expect("write settled-work handoff");
    file.sync_all().expect("fsync settled-work handoff");
    std::fs::rename(temporary, &path).expect("atomic settled-work handoff");
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum AdmissionFixture {
    OverLimit,
    EqualLimit,
    FailedMember,
}

impl AdmissionFixture {
    fn format(self) -> &'static str {
        match self {
            Self::OverLimit => "legacy-free-admission-frontier-v1",
            Self::EqualLimit => "legacy-free-admission-equal-limit-v1",
            Self::FailedMember => "legacy-failed-member-frontier-v1",
        }
    }
    fn case(self) -> &'static str {
        match self {
            Self::OverLimit => "free-admission",
            Self::EqualLimit => "free-admission-equal-limit",
            Self::FailedMember => "failed-member",
        }
    }
}

const FREE_WORKER: &str = "free-admission-worker";
const FREE_NAME: &str = "q5-free-admission";

fn free_admission_document() -> Value {
    let mut document = nonfinal_document();
    document["metadata"]["name"] = json!(FREE_NAME);
    for node in document["nodes"]
        .as_array_mut()
        .expect("fixture root nodes")
    {
        if node["type"] == "forEach" {
            node["body"] = json!("legacyLine");
        }
    }
    document["bodies"]
        .as_object_mut()
        .expect("fixture bodies")
        .remove("targetLine");
    document
}

async fn free_admission_source(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    run: Uuid,
    cutoff: i64,
) -> String {
    admission_source(pool, tenant, run, cutoff, false).await
}

async fn admission_source(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    run: Uuid,
    cutoff: i64,
    failed_member: bool,
) -> String {
    let closed: Value = sqlx::query_scalar(NONFINAL_CLOSED_SOURCE_SQL)
        .bind(tenant)
        .bind(run)
        .bind((0..10).map(|i| format!("c{i}")).collect::<Vec<_>>())
        .bind(cutoff)
        .fetch_one(pool)
        .await
        .expect("immutable old closed source");
    let mut target:Value=sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'run',to_jsonb(r)-ARRAY['status','run_version','stop_reason','stop_code','updated_at','finished_at'],
      'version',to_jsonb(version_row),
      'scope',to_jsonb(s)-ARRAY['lifecycle','drain_kind','drain_iteration','drain_failure_code','drain_sequence','drain_outbox_id','finish_sequence','finish_outbox_id','updated_at'],
      'owner',to_jsonb(o)-ARRAY['status','waiting_kind','failure_code','updated_at'],
      'snapshot',to_jsonb(snapshot_row),'snapshot_json',convert_from(snapshot_row.content,'UTF8')::jsonb,
      'captured',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.iteration_index) FROM collection_scope_items i WHERE i.tenant_id=s.tenant_id AND i.run_id=s.run_id AND i.scope_id=s.scope_id),
      'item_payloads',(SELECT jsonb_agg(to_jsonb(p) ORDER BY i.iteration_index) FROM collection_scope_items i JOIN payloads p ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest WHERE i.tenant_id=s.tenant_id AND i.run_id=s.run_id AND i.scope_id=s.scope_id),
      'entry_event',(SELECT to_jsonb(e) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.seq=$3),
      'entry_matches',(SELECT e.instance_id=o.step_id AND e.instance_path_nodes=s.owner_path_nodes AND e.instance_path_indices=s.owner_path_indices AND e.node=s.owner_node AND e.actor='system:engine' AND e.trace_id=r.trace_id AND e.attempt IS NULL AND e.payload_digest IS NULL AND e.occurred_at=s.created_at AND e.detail=jsonb_build_object('item_count',s.item_count,'max_items',s.max_items,'max_concurrency',s.max_concurrency,'snapshot_digest',encode(s.snapshot_digest,'hex'),'source_blob',s.source_blob_id::text) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.seq=$3))
      FROM runs r JOIN workflow_versions version_row ON version_row.tenant_id=r.tenant_id AND version_row.id=r.workflow_version_id
      JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10' AND cardinality(s.owner_path_nodes)=0
      JOIN steps o ON o.tenant_id=s.tenant_id AND o.run_id=s.run_id AND o.node=s.owner_node AND cardinality(o.instance_path_nodes)=0
      JOIN payloads snapshot_row ON snapshot_row.tenant_id=s.tenant_id AND snapshot_row.digest=s.snapshot_digest
      WHERE r.tenant_id=$1 AND r.id=$2"#).bind(tenant).bind(run).bind(cutoff+1)
        .fetch_one(pool).await.expect("real immutable free-admission target source");
    assert!(
        target["run"].is_object()
            && target["run"]["id"] == run.to_string()
            && target["entry_matches"] == true
            && target["version"].is_object()
            && target["version"]["id"].is_string()
            && target["scope"]["owner_node"] == "c10"
            && target["snapshot_json"] == json!([1, 2])
            && target["captured"].as_array().is_some_and(|a| a.len() == 2)
            && target["item_payloads"]
                .as_array()
                .is_some_and(|a| a.len() == 2)
            && target["entry_event"]["seq"] == cutoff + 1
            && target["entry_event"]["event_type"] == "collection.entered"
            && target["entry_event"]["node"] == "c10",
        "nonvacuous pinned definition, captured source, payloads and original entry"
    );
    if failed_member {
        let scope = target["scope"]
            .as_object_mut()
            .expect("actual target scope");
        scope.remove("active_count");
        scope.remove("completed_count");
        let failure: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
          'step',to_jsonb(w),
          'member',(SELECT to_jsonb(m)-ARRAY['lifecycle','slot_held','remaining_reservations','drained_sequence','drain_outbox_id','updated_at'] FROM collection_iterations m WHERE m.tenant_id=w.tenant_id AND m.run_id=w.run_id AND m.entry_instance_id=w.step_id),
          'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e WHERE e.tenant_id=w.tenant_id AND e.run_id=w.run_id AND e.seq BETWEEN $3+2 AND $3+6),
          'receipts',(SELECT jsonb_agg(to_jsonb(c) ORDER BY c.scope,c.idempotency_key) FROM command_dedupe c WHERE c.tenant_id=w.tenant_id AND c.scope='step:'||w.run_id::text||':scoped:3:c10:0;10:legacyWork:'||substring(encode(w.reported_lease_token_hash,'hex') from 1 for 16)),
          'reported',octet_length(w.reported_lease_token_hash)=32 AND w.lease_token_hash IS NULL AND w.leased_by IS NULL AND w.lease_expires_at IS NULL)
          FROM steps w WHERE w.tenant_id=$1 AND w.run_id=$2 AND w.node='legacyWork' AND w.instance_path_nodes=ARRAY['c10']::text[] AND w.instance_path_indices=ARRAY[0]::integer[]"#)
            .bind(tenant).bind(run).bind(cutoff).fetch_one(pool).await.expect("real failed-member source and original report receipt");
        let receipts = failure["receipts"]
            .as_array()
            .expect("original failure receipt");
        assert!(
            failure["step"]["status"] == "FAILED"
                && failure["step"]["routed"] == true
                && failure["step"]["failure_code"] == "Q5_MEMBER_FAILURE"
                && failure["step"]["output_digest"].is_null()
                && failure["reported"] == true
                && failure["member"]["entry_instance_id"] == failure["step"]["step_id"]
                && failure["member"]["entry_path_nodes"] == json!(["c10"])
                && failure["member"]["entry_path_indices"] == json!([0])
                && failure["member"]["entry_node"] == "legacyWork"
                && failure["member"]["admitted_sequence"] == cutoff + 2
                && receipts.len() == 1
                && receipts[0]["resource_id"] == format!("FAILED|WAITING|{}", cutoff + 6),
            "actual terminal failed work and canonical original receipt, no active lease or output"
        );
        let events = failure["events"]
            .as_array()
            .expect("original old admission/claim/failure suffix");
        assert!(
            events.len() == 5,
            "exact old five-event member suffix after entry"
        );
        for (i, kind) in [
            "collection.iteration_admitted",
            "step.released",
            "step.leased",
            "step.failed",
            "step.routed",
        ]
        .iter()
        .enumerate()
        {
            assert!(
                events[i]["seq"] == cutoff + 2 + i as i64
                    && events[i]["event_type"] == *kind
                    && events[i]["tenant_id"] == tenant.to_string()
                    && events[i]["run_id"] == run.to_string()
                    && events[i]["trace_id"] == target["run"]["trace_id"],
                "exact ordered old failure source event identity"
            );
            if i > 0 {
                assert!(
                    events[i]["instance_id"] == failure["step"]["step_id"]
                        && events[i]["node"] == "legacyWork"
                        && events[i]["instance_path_nodes"] == json!(["c10"])
                        && events[i]["instance_path_indices"] == json!([0]),
                    "failure suffix retains exact original scoped entry"
                );
            }
        }
        assert!(
            events[3]["detail"]["code"] == "Q5_MEMBER_FAILURE"
                && events[4]["detail"]["activated"] == json!([]),
            "known nonretryable failure routes no successor"
        );
        target["failed_member"] = failure;
    }
    nonfinal_sha(&json!({"closed":closed,"target":target}))
}

async fn free_admission_shape(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    run: Uuid,
    equal_limit: bool,
) -> Value {
    let state: Value = sqlx::query_scalar(r#"SELECT jsonb_build_object(
      'head',r.run_version,'workflow_version',r.workflow_version_id,'scope_id',s.scope_id,
      'waiting',r.status='WAITING' AND r.stop_reason IS NULL AND r.stop_code IS NULL AND r.parent_run_id IS NULL AND r.depth=0,
      'L',u.logical_output_bytes,
      'stored',(SELECT sum(octet_length(p.content)) FROM steps w JOIN payloads p ON p.tenant_id=w.tenant_id AND p.digest=w.output_digest WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id),
      'V',(SELECT sum(octet_length(p.content)) FROM iteration_scopes c JOIN steps o ON o.tenant_id=c.tenant_id AND o.run_id=c.run_id AND o.node=c.owner_node AND cardinality(o.instance_path_nodes)=0
          JOIN payloads p ON p.tenant_id=c.tenant_id AND p.digest=c.result_digest WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED' AND o.output_digest IS NULL),
      'closed_scopes',(SELECT count(*) FROM iteration_scopes c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED' AND c.item_count=2 AND c.completed_count=2 AND c.active_count=0),
      'closed_owners',(SELECT count(*) FROM steps o WHERE o.tenant_id=r.tenant_id AND o.run_id=r.id AND cardinality(o.instance_path_nodes)=0 AND o.status='SUCCEEDED' AND o.routed AND o.output_digest IS NULL),
      'historical_work',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.node='legacyWork' AND w.status='SUCCEEDED' AND w.routed AND w.output_digest IS NOT NULL),
      'historical_members',(SELECT count(*) FROM collection_iterations m JOIN iteration_scopes c ON c.tenant_id=m.tenant_id AND c.run_id=m.run_id AND c.scope_id=m.scope_id WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED'),
      'checkpoint_coordinates',(SELECT jsonb_agg(jsonb_build_array(c.owner_node,m.iteration_index) ORDER BY c.owner_node,m.iteration_index)
          FROM iteration_scopes c JOIN collection_iterations m ON m.tenant_id=c.tenant_id AND m.run_id=c.run_id AND m.scope_id=c.scope_id
          JOIN iteration_checkpoints k ON k.tenant_id=m.tenant_id AND k.run_id=m.run_id AND k.scope_id=m.scope_id AND k.iteration_index=m.iteration_index
          JOIN steps fw ON fw.tenant_id=m.tenant_id AND fw.run_id=m.run_id AND fw.step_id=m.entry_instance_id
          JOIN steps ret ON ret.tenant_id=m.tenant_id AND ret.run_id=m.run_id AND ret.step_id=k.return_instance_id
          JOIN run_events e ON e.tenant_id=k.tenant_id AND e.run_id=k.run_id AND e.seq=k.journal_sequence
          JOIN steps owner_row ON owner_row.tenant_id=c.tenant_id AND owner_row.run_id=c.run_id AND owner_row.node=c.owner_node AND owner_row.instance_path_nodes=c.owner_path_nodes AND owner_row.instance_path_indices=c.owner_path_indices
          WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED'
            AND m.lifecycle='CHECKPOINTED' AND NOT m.slot_held AND m.checkpoint_sequence=k.journal_sequence
            AND m.return_instance_id=k.return_instance_id AND m.result_digest=k.result_digest
            AND fw.status='SUCCEEDED' AND fw.routed AND fw.output_digest=k.result_digest AND ret.status='SUCCEEDED'
            AND cardinality(c.owner_path_nodes)=0 AND cardinality(c.owner_path_indices)=0
            AND fw.node='legacyWork' AND fw.instance_path_nodes=ARRAY[c.owner_node]::text[] AND fw.instance_path_indices=ARRAY[m.iteration_index]::integer[]
            AND ret.node='legacyReturned' AND ret.instance_path_nodes=ARRAY[c.owner_node]::text[] AND ret.instance_path_indices=ARRAY[m.iteration_index]::integer[]
            AND m.entry_node=fw.node AND m.entry_path_nodes=fw.instance_path_nodes AND m.entry_path_indices=fw.instance_path_indices
            AND m.return_node=ret.node AND m.return_path_nodes=ret.instance_path_nodes AND m.return_path_indices=ret.instance_path_indices
            AND e.event_type='collection.iteration_checkpointed' AND e.node=c.owner_node
            AND e.instance_id=owner_row.step_id AND e.instance_path_nodes=c.owner_path_nodes AND e.instance_path_indices=c.owner_path_indices
            AND e.detail->'return_instance'=jsonb_build_object('node',ret.node,'path',jsonb_build_array(jsonb_build_object('scope',c.owner_node,'index',m.iteration_index)))
            AND e.detail->>'index'=m.iteration_index::text AND e.detail->>'result_digest'=encode(k.result_digest,'hex')),
      'finish_route_pairs',(SELECT count(*) FROM iteration_scopes c
          JOIN run_events f ON f.tenant_id=c.tenant_id AND f.run_id=c.run_id AND f.seq=c.finish_sequence
          JOIN run_events routed ON routed.tenant_id=c.tenant_id AND routed.run_id=c.run_id AND routed.seq=c.finish_sequence+1
          JOIN steps owner_row ON owner_row.tenant_id=c.tenant_id AND owner_row.run_id=c.run_id AND owner_row.node=c.owner_node AND owner_row.instance_path_nodes=c.owner_path_nodes AND owner_row.instance_path_indices=c.owner_path_indices
          WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id AND c.lifecycle='SUCCEEDED'
            AND f.event_type='collection.finished' AND f.node=c.owner_node AND f.detail->>'outcome'='succeeded'
            AND f.detail->>'result_digest'=encode(c.result_digest,'hex')
            AND routed.event_type='step.routed' AND routed.node=c.owner_node
            AND f.instance_id=owner_row.step_id AND routed.instance_id=owner_row.step_id AND f.instance_path_indices=c.owner_path_indices AND routed.instance_path_indices=c.owner_path_indices
            AND routed.detail->'activated'=jsonb_build_array('c'||(substring(c.owner_node from 2)::integer+1)::text)
            AND cardinality(f.instance_path_nodes)=0 AND cardinality(routed.instance_path_nodes)=0),
      'historical_returns',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.node='legacyReturned' AND w.status='SUCCEEDED'),
      'checkpoints',(SELECT count(*) FROM iteration_checkpoints c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id),
      'open',s.lifecycle='OPEN' AND s.item_count=2 AND s.max_concurrency=1 AND s.next_admission_index=0 AND s.admitted_count=0 AND s.completed_count=0 AND s.active_count=0 AND s.result_digest IS NULL AND s.finish_sequence IS NULL,
      'target_members',(SELECT count(*) FROM collection_iterations m WHERE m.tenant_id=r.tenant_id AND m.run_id=r.id AND m.scope_id=s.scope_id),
      'unadmitted',(SELECT count(*) FROM collection_iterations m WHERE m.tenant_id=r.tenant_id AND m.run_id=r.id AND m.scope_id=s.scope_id AND m.iteration_index=1),
      'unadmitted_steps',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes[1]='c10' AND w.instance_path_indices[1]=1),
      'captured_indices',(SELECT jsonb_agg(jsonb_build_array(i.iteration_index,convert_from(p.content,'UTF8')::jsonb) ORDER BY i.iteration_index) FROM collection_scope_items i JOIN payloads p ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id AND i.scope_id=s.scope_id),
      'captured',(SELECT jsonb_agg(convert_from(p.content,'UTF8')::jsonb ORDER BY i.iteration_index) FROM collection_scope_items i JOIN payloads p ON p.tenant_id=i.tenant_id AND p.digest=i.item_digest WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id AND i.scope_id=s.scope_id),
      'target_owner',(SELECT count(*) FROM steps o WHERE o.tenant_id=r.tenant_id AND o.run_id=r.id AND o.node='c10' AND cardinality(o.instance_path_nodes)=0 AND o.status='WAITING' AND o.waiting_kind='COLLECTION' AND NOT o.routed AND o.output_digest IS NULL AND o.failure_code IS NULL AND o.attempt=0 AND o.generation=0 AND o.lease_token_hash IS NULL AND o.leased_by IS NULL AND o.lease_expires_at IS NULL),
      'target_steps',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes[1]='c10'),
      'target_receipts',(SELECT count(*) FROM command_dedupe c WHERE c.tenant_id=r.tenant_id AND c.scope LIKE 'step:'||r.id::text||':scoped:3:c10:%'),
      'step_count',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id),
      'done',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.node='done'),
      'reserved',u.reserved_instances,
      'tenant_active',(SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=r.tenant_id),
      'saga_entries',(SELECT count(*) FROM scope_saga_entries g WHERE g.tenant_id=r.tenant_id AND g.run_id=r.id),
      'live',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.status IN ('LEASED','RUNNING')),
      'sagas',(SELECT count(*) FROM scope_sagas g WHERE g.tenant_id=r.tenant_id AND g.run_id=r.id),
      'certificates',(SELECT count(*) FROM leaf_collection_success_certifications c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id),
      'chains',(SELECT count(*) FROM chain_segments c WHERE c.tenant_id=r.tenant_id AND c.run_id=r.id),
      'cutoff',(SELECT max(seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.node='c9' AND cardinality(e.instance_path_nodes)=0 AND e.event_type='step.routed'))
      FROM runs r JOIN run_instance_usage u ON u.tenant_id=r.tenant_id AND u.run_id=r.id
      JOIN iteration_scopes s ON s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.owner_node='c10'
      WHERE r.tenant_id=$1 AND r.id=$2"#)
        .bind(tenant).bind(run).fetch_one(pool).await.expect("genuine old free-admission premise");
    for (key, expected) in [
        ("waiting", json!(true)),
        ("L", json!(if equal_limit { 2_097_137 } else { 2_100_000 })),
        (
            "stored",
            json!(if equal_limit { 2_097_137 } else { 2_100_000 }),
        ),
        ("V", json!(if equal_limit { 2_097_167 } else { 2_100_030 })),
        ("closed_scopes", json!(10)),
        ("closed_owners", json!(10)),
        ("historical_work", json!(20)),
        ("historical_returns", json!(20)),
        ("checkpoints", json!(20)),
        ("open", json!(true)),
        ("target_members", json!(0)),
        ("unadmitted", json!(0)),
        ("unadmitted_steps", json!(0)),
        ("captured", json!([1, 2])),
        ("captured_indices", json!([[0, 1], [1, 2]])),
        ("target_owner", json!(1)),
        ("target_steps", json!(0)),
        ("target_receipts", json!(0)),
        ("step_count", json!(51)),
        ("done", json!(0)),
        ("reserved", json!(0)),
        ("tenant_active", json!(0)),
        ("saga_entries", json!(0)),
        ("live", json!(0)),
        ("sagas", json!(0)),
        ("certificates", json!(0)),
        ("chains", json!(0)),
    ] {
        assert!(state[key] == expected, "old free-admission premise: {key}");
    }
    assert!(
        state["historical_members"] == 20 && state["finish_route_pairs"] == 10,
        "twenty historical members and ten exact finish/routing journal pairs"
    );
    let coordinates = (0..10)
        .flat_map(|i| [json!([format!("c{i}"), 0]), json!([format!("c{i}"), 1])])
        .collect::<Vec<_>>();
    assert!(
        state["checkpoint_coordinates"] == json!(coordinates),
        "exact twenty CHECKPOINTED coordinates match work, Return, checkpoint and journal identities"
    );
    state
}

#[tokio::test(flavor = "current_thread")]
async fn frozen_old_free_admission_frontier_producer() {
    free_admission_produce(false, AdmissionFixture::OverLimit).await;
}

#[tokio::test(flavor = "current_thread")]
async fn frozen_old_free_admission_frontier_guard_red() {
    free_admission_produce(true, AdmissionFixture::OverLimit).await;
}

#[tokio::test(flavor = "current_thread")]
async fn frozen_old_free_admission_equal_limit_producer() {
    free_admission_produce(false, AdmissionFixture::EqualLimit).await;
}

#[tokio::test(flavor = "current_thread")]
async fn frozen_old_failed_member_frontier_producer() {
    free_admission_produce(false, AdmissionFixture::FailedMember).await;
}

async fn free_admission_produce(red: bool, scenario: AdmissionFixture) {
    let equal_limit = scenario == AdmissionFixture::EqualLimit;
    assert!(
        !red || scenario == AdmissionFixture::OverLimit,
        "only genuine over-limit guard has a RED"
    );
    assert!(
        !(red && equal_limit),
        "equality is a positive control, never a manufactured RED"
    );
    use crate::support::{
        api::Options,
        worker::{TENANT, Worker},
    };
    use sagat_application::{entropy::SystemEntropy, execution::Engine};
    use sagat_domain::{execution::ExecutionInstanceId, identity::Role, workflow::NodeId};
    use sagat_scheduler::{
        CollectionAdvance, Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings,
    };
    assert!(
        env!("SAGAT_Q5_SOURCE_COMMIT") == OLD_COMMIT,
        "immutable old production pin"
    );
    let runtime = crate::support::as_runtime_role();
    let role = if runtime { "runtime" } else { "owner" };
    let platform = crate::support::worker::platform_with(Options {
        runtime_role: runtime,
        ..Options::default()
    })
    .await;
    let namespace = platform.namespace_id().await;
    let (status, _) = platform
        .rest(
            Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit":1})),
        )
        .await;
    assert!(status == 200, "explicit public iteration capacity");
    platform
        .grant(platform.tenant, FREE_WORKER, Role::Worker)
        .await;
    let (status, _) = platform
        .rest(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/{FREE_WORKER}"),
            Some(&json!({"activities":["line.reserve"]})),
        )
        .await;
    assert!(status == 200, "public exact worker activity binding");
    let workflow = platform
        .publish(FREE_NAME, &free_admission_document())
        .await;
    let run_text = platform.start(&workflow, &json!({"lines":[1,2]})).await;
    let run: RunId = run_text.parse().expect("old run id");
    let api_role: (String, String) =
        sqlx::query_as("SELECT current_user::text,current_database()::text")
            .fetch_one(&platform.api.api_pool)
            .await
            .expect("actual old API/store shared pool identity");
    let owner_role: String = sqlx::query_scalar("SELECT current_user::text")
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("old inspection role");
    assert!(
        api_role.1 == platform.api.db.name && owner_role != crate::support::RUNTIME_ROLE,
        "owned old database identity"
    );
    assert!(
        api_role.0
            == if runtime {
                crate::support::RUNTIME_ROLE
            } else {
                owner_role.as_str()
            },
        "actual selected old API and scheduler-store role"
    );
    println!(
        "old_free_admission_role role={role} api={} store={} database={}",
        api_role.0, api_role.0, api_role.1
    );
    let scheduler_owner = format!("free-admission-{role}");
    let placement = Placement::new(scheduler_owner.clone());
    let keeper = Keeper::new(
        platform.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: std::time::Duration::from_secs(3600),
        },
    );
    assert!(
        keeper.round().await.is_ok(),
        "old scheduler owns its actual shards"
    );
    let scheduler = Scheduler::new(
        platform.api.executions.clone(),
        Engine::new(sagat_infra_ir::JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: std::time::Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement);
    let mut worker = platform.connect(FREE_WORKER, TENANT).await;
    worker.register_version(&["line.reserve"], "1.5").await;
    let large = Value::String("x".repeat(104_998));
    let large_bytes = serde_json::to_vec(&large).expect("105000-byte public old output");
    assert!(large_bytes.len() == 105_000, "old individual output width");
    let short = Value::String("x".repeat(102_135));
    let short_bytes = serde_json::to_vec(&short).expect("102137-byte public old output");
    assert!(
        short_bytes.len() == 102_137,
        "exact final equality output width"
    );
    let mut ticks = 0usize;
    for index in 0..20 {
        let task = loop {
            ticks += 1;
            assert!(ticks <= 256, "bounded old public scheduling transitions");
            assert!(
                scheduler.tick().await.failed == 0,
                "old public scheduler setup succeeds"
            );
            let mut tasks = worker.poll(1).await;
            if let Some(task) = tasks.pop() {
                assert!(tasks.is_empty(), "one old task at a time");
                break task;
            }
        };
        let identity = task.instance.as_ref().expect("old scoped task identity");
        assert!(
            task.run_id == run_text
                && task.activity_name == "line.reserve"
                && identity.instance_path.len() == 1
                && identity.instance_path[0].scope_node_id == format!("c{}", index / 2)
                && identity.instance_path[0].iteration_index == index % 2
                && identity.definition_node_id == "legacyWork",
            "exact ordered old genuine scoped work"
        );
        let output = std::str::from_utf8(if equal_limit && index == 19 {
            &short_bytes
        } else {
            &large_bytes
        })
        .expect("JSON UTF8");
        let request = worker.req(Worker::complete_message(&task, output));
        let ack = match worker.client.complete_step(request).await {
            Ok(r) => r.into_inner(),
            Err(_) => panic!("old genuine successful work completion"),
        };
        assert!(
            !ack.already_applied && ack.step_status == "SUCCEEDED" && ack.run_status == "RUNNING",
            "actual old success without a stop"
        );
    }
    // No tick after completion20: isolate final checkpoint, root routing and entry.
    for (node, expected) in [
        ("c9", CollectionAdvance::Checkpointed { index: 1 }),
        ("c9", CollectionAdvance::Finished),
        ("c10", CollectionAdvance::Entered),
    ] {
        ticks += 1;
        assert!(
            ticks <= 256,
            "bounded old setup including exact frontier boundary"
        );
        let owner = ExecutionInstanceId::root(NodeId::parse(node).expect("fixture owner"));
        let actual = scheduler
            .advance_collection(platform.tenant, run, &owner)
            .await
            .expect("normal old boundary transition");
        let exact = matches!(
            (&actual, &expected),
            (
                CollectionAdvance::Checkpointed { index: 1 },
                CollectionAdvance::Checkpointed { index: 1 }
            ) | (CollectionAdvance::Finished, CollectionAdvance::Finished)
                | (CollectionAdvance::Entered, CollectionAdvance::Entered)
        );
        assert!(exact, "exact checkpoint then finish then unadmitted entry");
    }
    let mut state = free_admission_shape(
        &platform.api.db.pool,
        platform.tenant.as_uuid(),
        run.as_uuid(),
        equal_limit,
    )
    .await;
    let aggregates: Vec<(String,Vec<u8>)>=sqlx::query_as("SELECT s.owner_node,p.content FROM iteration_scopes s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.result_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.lifecycle='SUCCEEDED' ORDER BY s.owner_node")
        .bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_all(&platform.api.db.pool).await.expect("all ten genuine old aggregate payloads");
    let expected = serde_json::to_vec(&json!([large.clone(), large.clone()]))
        .expect("actual two-item aggregate");
    assert!(
        aggregates.len() == 10 && expected.len() == 210_003,
        "ten actual 210003-byte aggregate references"
    );
    let final_expected =
        serde_json::to_vec(&json!([large, short])).expect("actual short final aggregate");
    assert!(
        final_expected.len() == 207_140,
        "exact final equality aggregate width"
    );
    for (index, (owner, bytes)) in aggregates.iter().enumerate() {
        let wanted = if equal_limit && index == 9 {
            &final_expected
        } else {
            &expected
        };
        assert!(
            *owner == format!("c{index}") && bytes == wanted,
            "each old aggregate has the exact ordered two outputs"
        );
    }
    let payloads: Vec<(Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT digest,content FROM payloads WHERE tenant_id=$1 ORDER BY encode(digest,'hex')",
    )
    .bind(platform.tenant.as_uuid())
    .fetch_all(&platform.api.db.pool)
    .await
    .expect("actual old retained payload digests and bytes");
    for (digest, content) in payloads {
        assert!(
            digest == Sha256::digest(&content).to_vec(),
            "every old aggregate, checkpoint and retained work digest matches its actual bytes"
        );
    }
    let cutoff = state["cutoff"].as_i64().expect("actual c9 routing cutoff");
    assert!(
        state["head"] == cutoff + 1,
        "actual old handoff is exactly c10 entry after c9 route"
    );
    if scenario == AdmissionFixture::FailedMember {
        ticks += 1;
        assert!(ticks <= 256, "bounded old failed-member admission");
        let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("target owner"));
        assert!(
            matches!(
                scheduler
                    .advance_collection(platform.tenant, run, &owner)
                    .await,
                Ok(CollectionAdvance::Admitted { index: 0 })
            ),
            "old normal admission before actual failure"
        );
        let mut tasks = worker.poll(1).await;
        assert!(tasks.len() == 1, "one actual original failed-member task");
        let task = tasks.pop().expect("actual member lease");
        let identity = task.instance.as_ref().expect("actual scoped task identity");
        assert!(
            task.run_id == run_text
                && task.activity_name == "line.reserve"
                && task.attempt == 1
                && task.generation > 0
                && !task.lease_token.is_empty()
                && identity.definition_node_id == "legacyWork"
                && identity.instance_path.len() == 1
                && identity.instance_path[0].scope_node_id == "c10"
                && identity.instance_path[0].iteration_index == 0,
            "actual exact c10 item0 forward lease, no replacement task"
        );
        let message = Worker::fail_message(&task, "Q5_MEMBER_FAILURE", false);
        let request = worker.req(message.clone());
        let ack = match worker.client.fail_step(request).await {
            Ok(r) => r.into_inner(),
            Err(_) => panic!("real old nonretryable member failure"),
        };
        assert!(
            !ack.already_applied && ack.step_status == "FAILED" && ack.run_status == "WAITING",
            "genuine failed-member ACK without a root stop"
        );
        let request = worker.req(message);
        let replay = match worker.client.fail_step(request).await {
            Ok(r) => r.into_inner(),
            Err(_) => panic!("canonical old failure replay"),
        };
        assert!(
            replay.already_applied
                && replay.command_id == ack.command_id
                && replay.step_status == ack.step_status
                && replay.run_status == ack.run_status,
            "original failure ACK replay before active lease check"
        );
        let failed:Value=sqlx::query_scalar(r#"SELECT jsonb_build_object(
          'head',r.run_version,'waiting',r.status='WAITING' AND r.stop_reason IS NULL AND r.stop_code IS NULL,
          'scope',to_jsonb(sc),'member',to_jsonb(m),'usage',to_jsonb(u),
          'tenant_active',(SELECT active_iterations FROM tenant_iteration_usage WHERE tenant_id=r.tenant_id),
          'target_steps',(SELECT count(*) FROM steps w WHERE w.tenant_id=r.tenant_id AND w.run_id=r.id AND w.instance_path_nodes=ARRAY['c10']::text[]),
          'checkpoints',(SELECT count(*) FROM iteration_checkpoints k WHERE k.tenant_id=r.tenant_id AND k.run_id=r.id),
          'next_member',(SELECT count(*) FROM collection_iterations other WHERE other.tenant_id=sc.tenant_id AND other.run_id=sc.run_id AND other.scope_id=sc.scope_id AND other.iteration_index=1))
          FROM runs r JOIN iteration_scopes sc ON sc.tenant_id=r.tenant_id AND sc.run_id=r.id AND sc.owner_node='c10'
          JOIN collection_iterations m ON m.tenant_id=sc.tenant_id AND m.run_id=sc.run_id AND m.scope_id=sc.scope_id AND m.iteration_index=0
          JOIN run_instance_usage u ON u.tenant_id=r.tenant_id AND u.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2"#)
            .bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("actual old failed-member handoff");
        assert!(
            failed["waiting"] == true
                && failed["head"] == cutoff + 6
                && failed["scope"]["scope_id"] == state["scope_id"]
                && failed["scope"]["lifecycle"] == "OPEN"
                && failed["scope"]["next_admission_index"] == 1
                && failed["scope"]["admitted_count"] == 1
                && failed["scope"]["completed_count"] == 0
                && failed["scope"]["active_count"] == 1
                && failed["member"]["lifecycle"] == "ADMITTED"
                && failed["member"]["slot_held"] == true
                && failed["member"]["reserved_instances"] == 1
                && failed["member"]["remaining_reservations"] == 1
                && failed["member"]["return_instance_id"].is_null()
                && failed["member"]["result_digest"].is_null()
                && failed["member"]["drained_sequence"].is_null()
                && failed["member"]["drain_outbox_id"].is_null()
                && failed["usage"]["logical_output_bytes"] == 2_100_000
                && failed["usage"]["reserved_instances"] == 1
                && failed["tenant_active"] == 1
                && failed["target_steps"] == 1
                && failed["checkpoints"] == 20
                && failed["next_member"] == 0,
            "exact real failed member with held capacity, no root stop/Return/checkpoint/next work"
        );
        state["head"] = failed["head"].clone();
        println!(
            "old_failed_member_premise L=2100000 V=2100030 E=4200030 original_failure=Q5_MEMBER_FAILURE root_stop=false active=1 reserved=1"
        );
    }
    let source = if scenario == AdmissionFixture::FailedMember {
        admission_source(
            &platform.api.db.pool,
            platform.tenant.as_uuid(),
            run.as_uuid(),
            cutoff,
            true,
        )
        .await
    } else {
        free_admission_source(
            &platform.api.db.pool,
            platform.tenant.as_uuid(),
            run.as_uuid(),
            cutoff,
        )
        .await
    };
    let projection: Value = sqlx::query_scalar(PROJECTION_WITNESS_SQL)
        .bind(platform.tenant.as_uuid())
        .bind(vec![run.as_uuid()])
        .fetch_one(&platform.api.db.pool)
        .await
        .expect("full genuine old projection");
    if equal_limit {
        println!(
            "old_equal_limit_premise L=2097137 V=2097167 E=4194304 closed=10 checkpoints=20 members=0 free_capacity=true live_forward=0"
        );
    } else if scenario == AdmissionFixture::OverLimit {
        println!(
            "old_free_admission_premise L=2100000 V=2100030 E=4200030 closed=10 checkpoints=20 members=0 free_capacity=true live_forward=0"
        );
    }
    if red {
        let owner = ExecutionInstanceId::root(NodeId::parse("c10").expect("target owner"));
        let result = scheduler
            .advance_collection(platform.tenant, run, &owner)
            .await
            .expect("old normal free admission advance");
        let admitted = matches!(result, CollectionAdvance::Admitted { index: 0 });
        println!("observed_legacy_free_admission_admitted_zero={admitted}");
        assert!(
            matches!(result, CollectionAdvance::DrainStarted),
            "old guard RED: expected DrainStarted before first admission"
        );
        return;
    }
    let format = scenario.format();
    let case = scenario.case();
    let receipt = json!({"format":format,"source_commit":OLD_COMMIT,
        "case":case,"role":role,"database_name":platform.api.db.name,"database_url":platform.api.db.url,
        "runtime_database_url":platform.api.db.url_for(crate::support::RUNTIME_ROLE,crate::support::RUNTIME_ROLE_PASSWORD),
        "tenant":platform.tenant.to_string(),"other_tenant":platform.other_tenant.to_string(),"namespace":namespace,
        "run":run.to_string(),"workflow_version":state["workflow_version"],"scope_id":state["scope_id"],
        "scheduler_owner":scheduler_owner,"handoff_run_version":state["head"],"closed_source_cutoff":cutoff,
        "producer_witness_sha256":nonfinal_sha(&projection),"closed_source_sha256":source});
    assert!(
        receipt.as_object().is_some_and(|o| o.len() == 18),
        "closed eighteen-field unadmitted format"
    );
    let path = std::path::PathBuf::from(
        std::env::var_os("SAGAT_Q5_PHASE_RECEIPT").expect("private handoff path"),
    );
    let temporary = path.with_extension("tmp");
    use std::os::unix::fs::OpenOptionsExt as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temporary)
        .expect("exclusive private handoff");
    file.write_all(&serde_json::to_vec(&receipt).expect("private handoff JSON"))
        .expect("write settled-work handoff");
    file.sync_all().expect("fsync settled-work handoff");
    std::fs::rename(temporary, &path).expect("atomic settled-work handoff");
}
