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
use std::os::fd::AsFd as _;
use std::time::Duration;
use tokio::net::UnixStream;

#[path = "control.rs"]
mod control;

#[tokio::test]
async fn report_old_lease_to_current_server() {
    assert_eq!(env!("SAGAT_RELEASE_PROFILE_COMMIT"), super::RELEASE);
    crate::support::bounded::within("released in-flight client", Duration::from_secs(120), async {
        let inherited=std::io::stdin().as_fd().try_clone_to_owned().expect("inherited anonymous socket");
        let socket=std::os::unix::net::UnixStream::from(inherited);
        socket.set_nonblocking(true).expect("nonblocking inherited socket");
        let mut stream=UnixStream::from_std(socket).expect("async inherited socket");
        let platform=platform_with(Options {runtime_role:true, worker:WorkerSettings {lease_ttl:Duration::from_secs(3600),..WorkerSettings::default()},..Options::default()}).await;
        let (status,_)=platform.rest(reqwest::Method::PUT,"/quotas/iteration_concurrency",Some(&json!({"limit":1}))).await;
        assert_eq!(status,200);
        let workflow=platform.publish("released-inflight",&reserve_and_charge("released-inflight",1)).await;
        let run:RunId=platform.start(&workflow,&json!({"lines":[{"sku":"old-item"}]})).await.parse().expect("run id");
        let owner=format!("released-inflight-{}",run);
        let placement=Placement::new(owner.clone());
        Keeper::new(platform.api.executions.clone(),placement.clone(),KeeperSettings {ttl:Duration::from_secs(600)}).round().await.expect("old scheduler authority");
        let scheduler=Scheduler::new(platform.api.executions.clone(),Engine::new(JsonEvaluator),SystemEntropy,SchedulerSettings::default()).with_placement(placement);
        let mut worker=platform.worker("released-inflight-worker").await;
        worker.register_version(&crate::support::scope_flow::ACTIVITIES,"1.5").await;
        drive(&scheduler).await;
        let reserve=worker.one_task().await;
        assert_eq!(crate::support::scope_flow::node(&reserve),"reserve");
        worker.complete(&reserve,&json!({"producer":"v0.2.0"})).await.expect("old reserve completion");
        drive(&scheduler).await;
        let lease=worker.one_task().await;
        assert_eq!(crate::support::scope_flow::node(&lease),"charge");
        assert_eq!(lease.task_kind,"FORWARD");
        assert_eq!(lease.instance.as_ref().expect("scoped lease").instance_path.len(),1);
        let cutoff:i64=sqlx::query_scalar("SELECT run_version FROM runs WHERE tenant_id=$1 AND id=$2").bind(platform.tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&platform.api.db.pool).await.expect("old journal cutoff");
        let prefix=super::immutable_prefix(&platform.api.db.pool,platform.tenant.as_uuid(),run.as_uuid(),cutoff).await;
        platform.api.executions.verify_run(platform.tenant,run).await.expect("actual released leased-path replay");
        // This exact request and lease remain only in the old process memory.
        let mut request=Worker::complete_message(&lease,control::OUTPUT);
        request.idempotency_key="released-inflight-report".into();
        let request_hash=control::sha(&request.encode_to_vec());
        control::send(&mut stream,&json!({"requestSha256":request_hash,"tokenSha256":control::sha(lease.lease_token.as_bytes()),"expiresAtMs":lease.expires_at_epoch_ms,"releaseCommit":super::RELEASE,"database":platform.api.db.name,"tenant":platform.tenant.to_string(),"run":run.to_string(),"step":lease.step_id,"attempt":lease.attempt,"generation":lease.generation,"owner":owner})).await;
        let endpoint=control::receive(&mut stream).await;
        let mut client=WorkerServiceClient::connect(endpoint["url"].as_str().expect("current endpoint").to_owned()).await.expect("old client connects current endpoint");
        let authorization=tonic::metadata::MetadataValue::try_from(format!("Bearer {}",endpoint["token"].as_str().expect("refreshed test authentication"))).expect("valid authentication metadata");
        let mut first=tonic::Request::new(request.clone());
        first.metadata_mut().insert("authorization",authorization.clone());
        let applied=client.complete_step(first).await.expect("old in-flight report accepted by current service").into_inner();
        assert!(!applied.already_applied);
        assert_eq!(applied.step_status,"SUCCEEDED");
        let mut duplicate=tonic::Request::new(request);
        duplicate.metadata_mut().insert("authorization",authorization);
        let replay=client.complete_step(duplicate).await.expect("old exact duplicate to current service").into_inner();
        assert!(replay.already_applied);
        assert_eq!(replay.command_id,applied.command_id);
        assert_eq!(replay.step_status,applied.step_status);
        assert_eq!(replay.run_status,applied.run_status);
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
