//! Test-only phase compiled against the immutable v0.2.0 production source archive.
#![allow(clippy::expect_used, clippy::panic)]
use crate::support::{
    api::{Options, json_of},
    scope_flow::{Flow, publish, reserve_and_charge},
};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

const RELEASE: &str = "bfda342073db71e84f30bdc79ccf3fbc97a7153c";

mod cutover;

fn profile_path() -> std::path::PathBuf {
    assert_eq!(
        env!("SAGAT_RELEASE_PROFILE_COMMIT"),
        RELEASE,
        "phase must be built from the reviewed release archive"
    );
    std::env::var_os("SAGAT_V02_PROFILE")
        .expect("private profile path")
        .into()
}

fn sha(value: &Value) -> String {
    Sha256::digest(serde_json::to_vec(value).expect("canonical metadata"))
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn immutable_prefix(pool: &sqlx::PgPool, tenant: Uuid, run: Uuid, cutoff: i64) -> String {
    let source:Value=sqlx::query_scalar("SELECT jsonb_build_object('definition',(SELECT to_jsonb(v) FROM workflow_versions v WHERE v.tenant_id=r.tenant_id AND v.id=r.workflow_version_id),'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.seq<=$3),'work',(SELECT to_jsonb(s) FROM steps s WHERE s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.node='reserve' AND s.instance_path_nodes=ARRAY['lines']::text[] AND s.instance_path_indices=ARRAY[0]::int[]),'output',(SELECT encode(s.output_digest,'hex') FROM steps s WHERE s.tenant_id=r.tenant_id AND s.run_id=r.id AND s.node='reserve' AND s.instance_path_indices=ARRAY[0]::int[]),'captured',(SELECT jsonb_agg(to_jsonb(i) ORDER BY i.scope_id,i.iteration_index) FROM collection_scope_items i WHERE i.tenant_id=r.tenant_id AND i.run_id=r.id)) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2")
        .bind(tenant).bind(run).bind(cutoff).fetch_one(pool).await.expect("immutable old prefix metadata");
    sha(&source)
}

async fn immutable_wait_prefix(
    pool: &sqlx::PgPool,
    tenant: Uuid,
    run: Uuid,
    cutoff: i64,
) -> String {
    let source:Value=sqlx::query_scalar("SELECT jsonb_build_object('definition',(SELECT to_jsonb(v) FROM workflow_versions v WHERE v.tenant_id=r.tenant_id AND v.id=r.workflow_version_id),'events',(SELECT jsonb_agg(to_jsonb(e) ORDER BY e.seq) FROM run_events e WHERE e.tenant_id=r.tenant_id AND e.run_id=r.id AND e.seq<=$3),'origin',jsonb_build_object('namespace',r.namespace_id,'workflow',r.workflow_id,'version',r.workflow_version_id,'input',encode(r.input_digest,'hex'),'root',r.root_run_id,'created',r.created_at)) FROM runs r WHERE r.tenant_id=$1 AND r.id=$2")
        .bind(tenant).bind(run).bind(cutoff).fetch_one(pool).await.expect("immutable root-wait definition, origin and journal prefix");
    sha(&source)
}

#[tokio::test]
async fn write_released_v02_profile() {
    let path = profile_path();
    assert!(
        !path.exists(),
        "a phase must not overwrite an earlier witness"
    );
    let flow = Flow::start(
        "released-v02-profile",
        reserve_and_charge("released-v02-profile", 1),
        json!([{"sku":"item-1"}]),
    )
    .await;
    flow.drive().await;
    let tasks = flow.poll().await;
    assert_eq!(tasks.len(), 1);
    assert_eq!(crate::support::scope_flow::node(&tasks[0]), "reserve");
    flow.complete(&tasks[0], json!({"producer":"v0.2.0"})).await;
    flow.drive().await;
    let next:(String,i32)=sqlx::query_as("SELECT status,attempt FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='charge' AND instance_path_indices=ARRAY[0]")
        .bind(flow.tenant.as_uuid()).bind(flow.run.as_uuid()).fetch_one(&flow.api.db.pool).await.expect("unleased next activity survives process exit");
    assert_eq!(next, ("READY".into(), 0));
    flow.verify().await;
    let wait_ir = json!({"apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow","metadata":{"name":"released-v02-root-wait"},"entrypoint":"payment","nodes":[{"id":"payment","type":"waitEvent","eventType":"payment.confirmed","correlationExpr":"input.orderId","timeoutMs":3600000,"lookbackMs":5000,"next":"done"},{"id":"done","type":"end"}]});
    let workflow = publish(&flow.api, "released-v02-root-wait", wait_ir)
        .await
        .expect("old public root wait publication");
    let response = flow
        .api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId":workflow,"input":{"orderId":"released-v02-order"}}))
        .send()
        .await
        .expect("old public wait start");
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let wait_run = json_of(response).await["runId"]
        .as_str()
        .expect("old wait run")
        .to_owned();
    let cutoff: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(flow.tenant.as_uuid())
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("old journal boundary");
    let max_migration: i64 =
        sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success")
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("actual released schema boundary");
    assert_eq!(max_migration, 20261220000000);
    let namespace: Uuid =
        sqlx::query_scalar("SELECT namespace_id FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(flow.tenant.as_uuid())
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("namespace");
    let old_owner: String =
        sqlx::query_scalar("SELECT owner FROM scheduler_shards WHERE shard=run_shard($1)")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("old scheduler owner");
    let wait_id: sagat_domain::execution::RunId = wait_run.parse().expect("old wait identity");
    flow.api
        .executions
        .verify_run(flow.tenant, wait_id)
        .await
        .expect("old wait replay before migration");
    let wait_cutoff: i64 =
        sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(flow.tenant.as_uuid())
            .bind(wait_id.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("old wait journal boundary");
    let wait_state: Value = sqlx::query_scalar(include_str!("wait-state.sql"))
        .bind(flow.tenant.as_uuid())
        .bind(wait_id.as_uuid())
        .fetch_one(&flow.api.db.pool)
        .await
        .expect("old wait subscription metadata");
    assert_eq!(wait_state["status"], "WAITING");
    // Renew using the real released Keeper so expiry injection cannot accidentally rely on TTL.
    sagat_scheduler::Keeper::new(
        flow.api.executions.clone(),
        sagat_scheduler::Placement::new(old_owner.clone()),
        sagat_scheduler::KeeperSettings {
            ttl: std::time::Duration::from_secs(600),
        },
    )
    .round()
    .await
    .expect("old owner lease renewed before process exit");
    let witness = json!({"releaseCommit":RELEASE,"database":flow.api.db.name,"tenant":flow.tenant.to_string(),"namespace":namespace.to_string(),"collectionRun":flow.run.to_string(),"rootWaitRun":wait_run,"cutoff":cutoff,"oldOwner":old_owner,"maxMigration":max_migration,"waitCutoff":wait_cutoff,"waitPrefixSha256":immutable_wait_prefix(&flow.api.db.pool,flow.tenant.as_uuid(),wait_id.as_uuid(),wait_cutoff).await,"waitStateSha256":sha(&wait_state),"waitStepId":wait_state["stepId"],"prefixSha256":immutable_prefix(&flow.api.db.pool,flow.tenant.as_uuid(),flow.run.as_uuid(),cutoff).await});
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    use std::io::Write as _;
    let mut file = options.open(path).expect("new private metadata witness");
    file.write_all(&serde_json::to_vec_pretty(&witness).expect("metadata witness"))
        .expect("write metadata witness");
    file.sync_all().expect("durable phase boundary");
    // Tokens, session capabilities, database credentials and payload bytes never enter this file.
}

#[tokio::test]
async fn read_upgraded_profile_with_released_v02_code() {
    let witness: Value =
        serde_json::from_slice(&std::fs::read(profile_path()).expect("private metadata witness"))
            .expect("witness");
    assert_eq!(witness["releaseCommit"], RELEASE);
    let admin =
        std::env::var("SAGAT_TEST_DATABASE_URL").expect("owned server supplied by coordinator");
    let db =
        crate::support::attach_existing_db(&admin, witness["database"].as_str().expect("database"))
            .await;
    // No old MIGRATOR is run against the newer schema. This matches serve's migration-off default.
    let api = crate::support::api::start_on_db(
        db,
        Options {
            runtime_role: true,
            ..Options::default()
        },
    )
    .await;
    let tenant = sagat_domain::identity::TenantId::new(
        witness["tenant"]
            .as_str()
            .expect("tenant")
            .parse()
            .expect("UUID"),
    );
    for field in ["collectionRun", "rootWaitRun"] {
        let run: sagat_domain::execution::RunId = witness[field]
            .as_str()
            .expect("run")
            .parse()
            .expect("run id");
        let response = api
            .as_user(Method::GET, &format!("/runs/{run}"), "alice", "acme")
            .send()
            .await
            .expect("released REST reader on current schema");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(json_of(response).await["status"], "SUCCEEDED");
        api.executions
            .verify_run(tenant, run)
            .await
            .expect("released native full reader on upgraded retained history");
    }
    assert_eq!(
        immutable_prefix(
            &api.db.pool,
            tenant.as_uuid(),
            witness["collectionRun"]
                .as_str()
                .expect("run")
                .parse()
                .expect("UUID"),
            witness["cutoff"].as_i64().expect("cutoff")
        )
        .await,
        witness["prefixSha256"]
    );
    let wait: Uuid = witness["rootWaitRun"]
        .as_str()
        .expect("wait run")
        .parse()
        .expect("UUID");
    assert_eq!(
        immutable_wait_prefix(
            &api.db.pool,
            tenant.as_uuid(),
            wait,
            witness["waitCutoff"].as_i64().expect("wait boundary")
        )
        .await,
        witness["waitPrefixSha256"]
    );
    let wait_step:Uuid=sqlx::query_scalar("SELECT step_id FROM steps WHERE tenant_id=$1 AND run_id=$2 AND node='payment' AND cardinality(instance_path_nodes)=0").bind(tenant.as_uuid()).bind(wait).fetch_one(&api.db.pool).await.expect("old reader retained exact physical wait identity");
    assert_eq!(wait_step.to_string(), witness["waitStepId"]);
}

#[cfg(unix)]
#[path = "inflight.rs"]
mod inflight;
