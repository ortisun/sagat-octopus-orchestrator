//! Genuine released client retains its old task capability in memory across server upgrade.
use crate::support::{
    api::Options,
    scope_flow::reserve_and_charge,
    worker::{Worker, platform_with},
};
use prost::Message as _;
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_application::{entropy::SystemEntropy, execution::Engine, worker::WorkerSettings};
use sagat_domain::execution::RunId;
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::json;
use std::time::Duration;

#[path = "control.rs"]
pub(super) mod control;

#[tokio::test]
async fn report_old_lease_to_current_server() {
    report(control::ProfileMode::Forward).await;
}

#[tokio::test]
async fn report_old_applied_lookup_to_current_server() {
    report(control::ProfileMode::AppliedLookup).await;
}

#[tokio::test]
async fn report_old_applied_lookup_crossing_the_run_output_quota() {
    report(control::ProfileMode::AppliedOutputQuota).await;
}

fn with_output_history(mut document: serde_json::Value) -> serde_json::Value {
    document["entrypoint"] = json!("history0");
    document["bodies"]["history-line"] = json!({"entrypoint":"historyLoad","nodes":[
        {"id":"historyLoad","type":"activity","activity":"line.notify","next":"historyReturn"},
        {"id":"historyReturn","type":"return","result":"steps.historyLoad.output"}
    ]});
    let mut roots = Vec::new();
    for index in 0..9 {
        roots.push(json!({"id":format!("history{index}"),"type":"forEach","items":"input.lines","body":"history-line","result":"bodyResult","maxItems":1,"maxConcurrency":1,"next":if index==8 {"lines".to_owned()} else {format!("history{}",index+1)}}));
    }
    roots.extend(
        document["nodes"]
            .as_array()
            .expect("root nodes")
            .iter()
            .cloned(),
    );
    document["nodes"] = json!(roots);
    document
}

async fn report(mode: control::ProfileMode) {
    let applied_lookup = mode.is_applied();
    assert_eq!(env!("SAGAT_RELEASE_PROFILE_COMMIT"), super::RELEASE);
    crate::support::bounded::within("released in-flight client", Duration::from_secs(120), async {
        let mut stream=control::inherited_stream();
        let platform=platform_with(Options {runtime_role:true, worker:WorkerSettings {lease_ttl:Duration::from_secs(3600),..WorkerSettings::default()},..Options::default()}).await;
        let (status,_)=platform.rest(reqwest::Method::PUT,"/quotas/iteration_concurrency",Some(&json!({"limit":1}))).await;
        assert_eq!(status,200);
        let mut document=reserve_and_charge("released-inflight",1);
        if applied_lookup {document["bodies"]["line"]["nodes"][1]["reconcileWith"]=json!("line.lookup");}
        if mode.refuses_output() {document=with_output_history(document);}
        let workflow=platform.publish("released-inflight",&document).await;
        let run:RunId=platform.start(&workflow,&json!({"lines":[{"sku":"old-item"}]})).await.parse().expect("run id");
        let owner=format!("released-inflight-{}",run);
        let placement=Placement::new(owner.clone());
        Keeper::new(platform.api.executions.clone(),placement.clone(),KeeperSettings {ttl:Duration::from_secs(600)}).round().await.expect("old scheduler authority");
        let scheduler=Scheduler::new(platform.api.executions.clone(),Engine::new(JsonEvaluator),SystemEntropy,SchedulerSettings::default()).with_placement(placement);
        let mut worker=platform.worker("released-inflight-worker").await;
        worker.register_version(&crate::support::scope_flow::ACTIVITIES,"1.5").await;
        drive(&scheduler).await;
        if mode.refuses_output() {
            for _ in 0..9 {
                let task=worker.one_task().await;
                assert_eq!(crate::support::scope_flow::node(&task),"historyLoad");
                worker.complete(&task,&json!("x".repeat(control::QUOTA_HISTORY_BYTES-2))).await.expect("actual old bounded history output");
                drive(&scheduler).await;
            }
        }
        let reserve=worker.one_task().await;
        assert_eq!(crate::support::scope_flow::node(&reserve),"reserve");
        let reserve_output=if mode.refuses_output() {json!("r".repeat(control::QUOTA_RESERVE_BYTES-2))} else {json!({"producer":"v0.2.0"})};
        worker.complete(&reserve,&reserve_output).await.expect("old reserve completion");
        drive(&scheduler).await;
        let forward=worker.one_task().await;
        assert_eq!(crate::support::scope_flow::node(&forward),"charge");
        assert_eq!(forward.task_kind,"FORWARD");
        let lease=if applied_lookup {
            let unknown=worker.fail_with(&forward,"OUTCOME_UNKNOWN",false).await.expect("real old uncertain forward result");
            assert!(!unknown.already_applied);assert_eq!(unknown.step_status,"WAITING");
            drive(&scheduler).await;
            let lookup=worker.one_task().await;
            assert_eq!(lookup.task_kind,"RECONCILIATION");
            assert_eq!(crate::support::scope_flow::node(&lookup),"charge~r");
            assert!(lookup.target_instance==forward.instance);
            assert_eq!(lookup.target_node,"charge");
            assert_eq!(lookup.target_idempotency_key,forward.idempotency_key);
            assert_ne!(lookup.idempotency_key,forward.idempotency_key);
            lookup
        } else {forward.clone()};
        assert_eq!(lease.instance.as_ref().expect("scoped lease").instance_path.len(),1);
        let cutoff:i64=sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2").bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("old journal cutoff");
        let prefix=super::immutable_prefix(&platform.api.db.pool,platform.tenant.as_uuid(),run.as_uuid(),cutoff).await;
        platform.api.executions.verify_run(platform.tenant,run).await.expect("actual released leased-path replay");
        if mode.refuses_output() {
            let ledger:i64=sqlx::query_scalar("SELECT logical_output_bytes FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2").bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("actual released ledger");
            let owners:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND instance_path_nodes='{}' AND node LIKE 'history%' AND status='SUCCEEDED' AND output_digest IS NOT NULL").bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("released owner outputs");
            assert_eq!(owners,9);
            assert_eq!(ledger,4_030_018);
            assert!(ledger<4_194_304 && ledger+control::QUOTA_RESULT_BYTES as i64>4_194_304);
        }
        let output_sources=if mode.refuses_output() {control::quota_sources(&platform.api.db.pool,platform.tenant.as_uuid(),run.as_uuid()).await} else {serde_json::Value::Null};
        // This exact request and lease remain only in the old process memory.
        let report_output=mode.request_output();
        let mut request=Worker::complete_message(&lease,report_output);
        request.idempotency_key="released-inflight-report".into();
        let request_hash=control::sha(&request.encode_to_vec());
        control::send(&mut stream,&json!({"scenario":mode.scenario(),"cutoff":cutoff,"prefixSha256":prefix,"appliedLookup":applied_lookup,"forwardStep":forward.step_id,"forwardAttempt":forward.attempt,"forwardGeneration":forward.generation,"forwardTokenSha256":control::sha(forward.lease_token.as_bytes()),"forwardKeySha256":control::sha(forward.idempotency_key.as_bytes()),"reserveKeySha256":control::sha(reserve.idempotency_key.as_bytes()),"refusedOutput":mode.refuses_output(),"outputSources":output_sources,"requestSha256":request_hash,"tokenSha256":control::sha(lease.lease_token.as_bytes()),"expiresAtMs":lease.expires_at_epoch_ms,"releaseCommit":super::RELEASE,"database":platform.api.db.name,"tenant":platform.tenant.to_string(),"run":run.to_string(),"step":lease.step_id,"attempt":lease.attempt,"generation":lease.generation,"owner":owner})).await;
        let endpoint=control::receive(&mut stream).await;
        let mut client=WorkerServiceClient::connect(endpoint["url"].as_str().expect("current endpoint").to_owned()).await.expect("old client connects current endpoint");
        let authorization=tonic::metadata::MetadataValue::try_from(format!("Bearer {}",endpoint["token"].as_str().expect("refreshed test authentication"))).expect("valid authentication metadata");
        let mut first=tonic::Request::new(request.clone());
        first.metadata_mut().insert("authorization",authorization.clone());
        let applied=client.complete_step(first).await.expect("old in-flight report accepted by current service").into_inner();
        assert!(!applied.already_applied);
        assert_eq!(applied.step_status,"SUCCEEDED");assert_eq!(applied.run_status,mode.report_status());
        if mode.refuses_output() {
            let stopped:(String,String,i64)=sqlx::query_as("SELECT r.status,r.stop_code,u.logical_output_bytes FROM runs r JOIN run_instance_usage u ON u.tenant_id=r.tenant_id AND u.run_id=r.id WHERE r.tenant_id=$1 AND r.id=$2").bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.api_pool).await.expect("state immediately after first report, before replay");
            assert_eq!(stopped,("CANCELLING".into(),"RUN_DATA_LIMIT".into(),4_030_018));
            assert_eq!(control::quota_sources(&platform.api.db.pool,platform.tenant.as_uuid(),run.as_uuid()).await,output_sources);
            platform.api.executions.verify_run(platform.tenant,run).await.expect("old reader before replay cannot rely on a repair");
        }
        let mut duplicate=tonic::Request::new(request);
        duplicate.metadata_mut().insert("authorization",authorization);
        let replay=client.complete_step(duplicate).await.expect("old exact duplicate to current service").into_inner();
        assert!(replay.already_applied);
        assert_eq!(replay.command_id,applied.command_id);
        assert_eq!(replay.step_status,applied.step_status);
        assert_eq!(replay.run_status,applied.run_status);
        let retained:Option<Vec<u8>>=sqlx::query_scalar("SELECT p.content FROM steps s LEFT JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.step_id=$3 AND s.node='charge'").bind(platform.tenant.as_uuid()).bind(run.as_uuid()).bind(forward.step_id.parse::<uuid::Uuid>().expect("old forward UUID")).fetch_one(&platform.api.api_pool).await.expect("released reader exact retained forward output");
        assert_eq!(retained.as_ref().map(Vec::len),mode.retained_output().map(str::len));
        if let (Some(actual),Some(expected))=(&retained,mode.retained_output()) { assert_eq!(control::sha(actual),control::sha(expected.as_bytes())); }
        assert_eq!(super::immutable_prefix(&platform.api.db.pool,platform.tenant.as_uuid(),run.as_uuid(),cutoff).await,prefix);
        platform.api.executions.verify_run(platform.tenant,run).await.expect("released replay after current in-flight completion");
        control::send(&mut stream,&json!({"reported":true,"replayed":true,"command":applied.command_id,"requestSha256":request_hash})).await;
    }).await;
}

async fn drive(
    scheduler: &Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy>,
) {
    for _ in 0..32 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0);
        if !report.did_work() {
            return;
        }
    }
    panic!("old scheduler did not settle");
}

#[tokio::test]
async fn read_finished_inflight_profile_with_released_code() {
    crate::support::bounded::within("released terminal in-flight reader",Duration::from_secs(60),async {
        let mut stream=control::inherited_stream();
        let witness=control::receive(&mut stream).await;
        assert_eq!(witness["releaseCommit"],super::RELEASE);
        let server=std::env::var("SAGAT_TEST_DATABASE_URL").expect("owned server");
        let db=crate::support::attach_existing_db(&server,witness["database"].as_str().expect("old database")).await;
        let api=crate::support::api::start_on_db(db,Options {runtime_role:true,..Options::default()}).await;
        let tenant=sagat_domain::identity::TenantId::new(witness["tenant"].as_str().expect("tenant").parse().expect("UUID"));
        let run:RunId=witness["run"].as_str().expect("run").parse().expect("UUID");
        let response=api.as_user(reqwest::Method::GET,&format!("/runs/{run}"),"alice","acme").send().await.expect("actual old REST terminal reader");
        assert_eq!(response.status(),reqwest::StatusCode::OK);
        let expected=match witness["scenario"].as_str() {Some("inflight-worker"|"applied-lookup")=>"SUCCEEDED",Some("applied-output-quota")=>"COMPENSATED",_=>panic!("unknown released terminal profile")};
        assert_eq!(crate::support::api::json_of(response).await["status"],expected);
        api.executions.verify_run(tenant,run).await.expect("actual old native terminal replay");
        assert_eq!(super::immutable_prefix(&api.db.pool,tenant.as_uuid(),run.as_uuid(),witness["cutoff"].as_i64().expect("old prefix boundary")).await,witness["prefixSha256"].as_str().expect("old prefix hash"));
        control::send(&mut stream,&json!({"terminalOldReader":true,"prefixVerifiedAfterFinalization":true,"scenario":witness["scenario"]})).await;
    }).await;
}
