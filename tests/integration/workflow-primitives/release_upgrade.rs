//! Genuine released-source producer → normal migration/current progress → released reader.
use crate::support::{
    self,
    api::{Options, TestApi, json_of},
    bounded,
    scope_flow::worker_request,
};
use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::{
    CompleteStepRequest, PollTasksRequest, RegisterWorkerRequest, WorkerCapabilities,
    worker_service_client::WorkerServiceClient,
};
use sagat_application::{entropy::SystemEntropy, execution::Engine};
use sagat_domain::{execution::RunId, identity::TenantId};
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{
    EventDispatchSettings, EventDispatcher, Keeper, KeeperSettings, Placement, Scheduler,
    SchedulerSettings,
};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    io::Read as _,
    path::{Path, PathBuf},
    time::Duration,
};
use uuid::Uuid;

const RELEASE: &str = "bfda342073db71e84f30bdc79ccf3fbc97a7153c";

pub(super) fn file_sha(path: &Path) -> String {
    let mut input = std::fs::File::open(path).expect("open owned phase executable");
    let mut sha = Sha256::new();
    let mut buffer = [0_u8; 65536];
    loop {
        let count = input.read(&mut buffer).expect("read phase executable");
        if count == 0 {
            break;
        }
        sha.update(&buffer[..count]);
    }
    sha.finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

async fn phase(binary: &Path, name: &str, profile: &Path, database: &str) {
    let mut command = tokio::process::Command::new(binary);
    command
        .arg(format!("release_upgrade_phase::{name}"))
        .args(["--exact", "--nocapture"])
        .env("SAGAT_TEST_DATABASE_URL", database)
        .env("SAGAT_V02_PROFILE", profile)
        .env("RUST_TEST_THREADS", "1");
    let status = bounded::exit_status(
        format!("released v0.2.0 {name}"),
        Duration::from_secs(120),
        command,
    )
    .await;
    assert!(
        status.success(),
        "released phase {name} failed with {status}"
    );
}

async fn drive(
    scheduler: &Scheduler<sagat_infra_postgres::PgRunStore, JsonEvaluator, SystemEntropy>,
) {
    for _ in 0..32 {
        let report = scheduler.tick().await;
        assert_eq!(
            report.failed, 0,
            "current scheduler on old rows: {report:?}"
        );
        if !report.did_work() {
            return;
        }
    }
    panic!("current scheduler did not settle on old retained rows");
}

async fn complete_old_ready_activity(api: &TestApi, run: RunId) {
    let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
        .await
        .expect("current worker protocol endpoint");
    let registered = worker
        .register_worker(worker_request(
            api,
            RegisterWorkerRequest {
                worker_name: "release-upgrade-current-worker".into(),
                namespace: "payments".into(),
                capabilities: Some(WorkerCapabilities {
                    languages: vec!["rust".into()],
                    activities: crate::support::scope_flow::ACTIVITIES
                        .iter()
                        .map(|name| (*name).to_owned())
                        .collect(),
                    pools: Vec::new(),
                    max_inflight: 8,
                    protocol_version: "1.5".into(),
                }),
            },
        ))
        .await
        .expect("register current worker for old namespace/binding")
        .into_inner();
    assert_eq!(registered.accepted_protocol_version, "1.5");
    let tasks = worker
        .poll_tasks(worker_request(
            api,
            PollTasksRequest {
                worker_id: registered.worker_id,
                session_token: registered.session_token,
                max_tasks: 8,
            },
        ))
        .await
        .expect("current poll of old READY activity")
        .into_inner()
        .tasks;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.run_id, run.to_string());
    assert_eq!(crate::support::scope_flow::node(task), "charge");
    let response = worker
        .complete_step(worker_request(
            api,
            CompleteStepRequest {
                task_id: task.task_id.clone(),
                run_id: task.run_id.clone(),
                step_id: task.step_id.clone(),
                attempt: task.attempt,
                generation: task.generation,
                lease_token: task.lease_token.clone(),
                idempotency_key: "release-upgrade-current-completion".into(),
                output_json: json!({"consumer":"current"}).to_string(),
                output_blob_ref: String::new(),
                output_sha256: String::new(),
                traceparent: String::new(),
                instance: task.instance.clone(),
            },
        ))
        .await
        .expect("current gRPC completion of old admitted path")
        .into_inner();
    assert_eq!(response.step_status, "SUCCEEDED");
}

#[tokio::test]
#[ignore = "requires the verified v0.2.0 phase executable; run the release-upgrade profile script"]
async fn release_upgrade_active_rows_upgrade_progress_and_remain_readable_by_the_release() {
    assert!(
        std::env::var_os("SAGAT_TEST_DATABASE_URL").is_none(),
        "coordinator must own its disposable PostgreSQL server"
    );
    let build_path = PathBuf::from(
        std::env::var_os("SAGAT_V02_BUILD").expect("verified private build manifest"),
    );
    let build: Value =
        serde_json::from_slice(&std::fs::read(&build_path).expect("read build manifest"))
            .expect("build metadata");
    assert_eq!(build["releaseCommit"], RELEASE);
    assert!(
        build["productionFilesVerified"]
            .as_u64()
            .expect("verified source count")
            > 600
    );
    let binary = PathBuf::from(build["binary"].as_str().expect("phase executable"));
    assert_eq!(
        binary.canonicalize().expect("owned executable").parent(),
        build_path
            .canonicalize()
            .expect("owned build manifest")
            .parent()
    );
    assert_eq!(
        file_sha(&binary),
        build["binarySha256"].as_str().expect("phase hash")
    );
    let directory = tempfile::Builder::new()
        .prefix("profile-")
        .tempdir_in(build_path.parent().expect("owned artifact parent"))
        .expect("new private runtime witness directory")
        .keep();
    let profile = directory.join("profile.json");
    bounded::lifecycle("released v0.2.0 active-data upgrade",Duration::from_secs(300),|progress|async move {
        progress.enter("start owned disposable PostgreSQL");
        let owner=support::fresh_db().await;
        progress.enter("actual old public producer process");
        phase(&binary,"write_release_upgrade_profile",&profile,&owner.admin_url).await;
        let witness:Value=serde_json::from_slice(&std::fs::read(&profile).expect("old metadata witness")).expect("old witness");
        assert_eq!(witness["releaseCommit"],RELEASE);
        let tenant=TenantId::new(witness["tenant"].as_str().expect("tenant").parse().expect("tenant UUID"));
        let collection:RunId=witness["collectionRun"].as_str().expect("collection").parse().expect("run");
        let wait:RunId=witness["rootWaitRun"].as_str().expect("root wait").parse().expect("run");
        progress.enter("attach exact old database and apply current immutable migrations");
        let db=support::attach_existing_db(&owner.admin_url,witness["database"].as_str().expect("owned database")).await;
        let before:i64=sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success").fetch_one(&db.pool).await.expect("old schema identity");
        assert_eq!(before,20261220000000);
        sagat_infra_postgres::migrate(&db.pool).await.expect("normal current migration of actual released data");
        let after:i64=sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success").fetch_one(&db.pool).await.expect("new schema identity");
        assert_eq!(after,20261226000000);
        let wait_state:Value=sqlx::query_scalar(include_str!("../../fixtures/release-upgrade/wait-state.sql")).bind(tenant.as_uuid()).bind(wait.as_uuid()).fetch_one(&db.pool).await.expect("unchanged legacy wait metadata after migration");
        let wait_state_hash:String=Sha256::digest(serde_json::to_vec(&wait_state).expect("wait metadata")).iter().map(|byte|format!("{byte:02x}")).collect();
        assert_eq!(wait_state_hash,witness["waitStateSha256"]);
        let old_shards:Vec<(i16,i64,bool)>=sqlx::query_as("SELECT shard,generation,expires_at>clock_timestamp() FROM scheduler_shards WHERE owner=$1 ORDER BY shard").bind(witness["oldOwner"].as_str().expect("old owner")).fetch_all(&db.pool).await.expect("actual released owner leases");
        assert!(!old_shards.is_empty() && old_shards.iter().all(|(_,_,live)|*live));
        // Explicit expiry injection after old-process exit, not a sleep or live-owner race proof.
        let members=sqlx::query("UPDATE scheduler_members SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE owner=$1").bind(witness["oldOwner"].as_str().expect("old owner")).execute(&db.pool).await.expect("stopped member expiry").rows_affected();
        assert_eq!(members,1);
        let shards=sqlx::query("UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE owner=$1").bind(witness["oldOwner"].as_str().expect("old owner")).execute(&db.pool).await.expect("stopped shard expiry").rows_affected();
        assert_eq!(shards,old_shards.len() as u64);
        let api=support::api::start_on_db(db,Options {runtime_role:true,..Options::default()}).await;
        let role:String=sqlx::query_scalar("SELECT current_user").fetch_one(&api.api_pool).await.expect("current services role");assert_eq!(role,"sagat_app");
        api.executions.verify_run(tenant,collection).await.expect("current full read before resuming old data");
        api.executions.verify_run(tenant,wait).await.expect("current full read of old wait before progressing it");
        let placement=Placement::new(format!("release-upgrade-current-{}",Uuid::now_v7()));
        let current_owner=placement.owner().to_owned();
        Keeper::new(api.executions.clone(),placement.clone(),KeeperSettings {ttl:Duration::from_secs(120)}).round().await.expect("new current scheduler owner");
        for run in [collection,wait] {
            let (shard,owner,generation,live):(i16,String,i64,bool)=sqlx::query_as("SELECT shard,owner,generation,expires_at>clock_timestamp() FROM scheduler_shards WHERE shard=run_shard($1)").bind(run.as_uuid()).fetch_one(&api.db.pool).await.expect("current exact shard takeover");
            assert_eq!(owner,current_owner);assert!(live);
            let prior=old_shards.iter().find(|(old,_,_)|*old==shard).expect("relevant shard was held by stopped old owner");assert!(generation>prior.1);
        }
        let scheduler=Scheduler::new(api.executions.clone(),Engine::new(JsonEvaluator),SystemEntropy,SchedulerSettings::default()).with_placement(placement);
        progress.enter("current worker/scheduler complete old admitted collection");
        complete_old_ready_activity(&api,collection).await;
        drive(&scheduler).await;
        progress.enter("current public event completes old root subscription");
        let event=api.as_user(Method::POST,"/events","alice","acme").header("x-octopus-namespace","payments").json(&json!({"eventId":"release-upgrade-current-event","type":"payment.confirmed","correlationKey":"release-upgrade-order","data":{"accepted":true}})).send().await.expect("current REST event admission");
        assert_eq!(event.status(),StatusCode::ACCEPTED);
        let dispatcher=EventDispatcher::new(api.executions.clone(),Engine::new(JsonEvaluator),SystemEntropy,sagat_application::execution::DefinitionCache::new(std::sync::Arc::new(sagat_infra_ir::JsonIrCodec), 8),EventDispatchSettings::default());
        let round=dispatcher.tick().await;
        assert_eq!((round.steps_woken,round.retried),(1,0));
        drive(&scheduler).await;
        for run in [collection,wait] {
            let response=api.as_user(Method::GET,&format!("/runs/{run}"),"alice","acme").send().await.expect("current terminal read");
            assert_eq!(response.status(),StatusCode::OK);assert_eq!(json_of(response).await["status"],"SUCCEEDED");
            api.executions.verify_run(tenant,run).await.expect("current final retained history replay");
        }
        let legacy_receipt:String=sqlx::query_scalar("SELECT consumer FROM event_deliveries WHERE tenant_id=$1 AND run_id=$2").bind(tenant.as_uuid()).bind(wait.as_uuid()).fetch_one(&api.db.pool).await.expect("unchanged legacy root receipt identity");
        assert_eq!(legacy_receipt,format!("wait:{wait}/payment"));
        progress.enter("actual released REST and native reader after current progress");
        phase(&binary,"read_upgraded_profile_with_release_upgrade_code",&profile,&owner.admin_url).await;
        let result=json!({"scenario":"active-data","releaseCommit":RELEASE,"schemaBefore":before,"schemaAfter":after,"collection":"SUCCEEDED","rootWait":"SUCCEEDED","legacyConsumerUnchanged":true,"oldImmutablePrefixVerified":true});
        std::fs::write(std::env::var_os("SAGAT_V02_RESULT").expect("selected phase result path"),serde_json::to_vec_pretty(&result).expect("result metadata")).expect("write result metadata");
        println!("release-upgrade compatibility: schema {before} -> {after}; old/current producer-reader checks passed");
    }).await;
}
