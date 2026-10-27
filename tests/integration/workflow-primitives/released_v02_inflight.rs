//! Released worker capability stays in the old process; only framed anonymous IPC crosses phases.
use crate::support::{self, api::Options, bounded};
use sagat_application::{entropy::SystemEntropy, execution::Engine};
use sagat_domain::{execution::RunId, identity::TenantId};
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use std::{os::fd::OwnedFd, path::PathBuf, process::Stdio, time::Duration};
use tokio::net::UnixStream;
use uuid::Uuid;

#[path = "../../fixtures/released-v02/control.rs"]
mod control;
const RELEASE: &str = "bfda342073db71e84f30bdc79ccf3fbc97a7153c";

fn verify_overlays(build: &Value) {
    let overlays = build["overlays"].as_array().expect("verified overlay list");
    let expected: [(&str, &[u8]); 4] = [
        (
            "tests/fixtures/released-v02/phase.rs",
            include_bytes!("../../fixtures/released-v02/phase.rs"),
        ),
        (
            "tests/fixtures/released-v02/wait-state.sql",
            include_bytes!("../../fixtures/released-v02/wait-state.sql"),
        ),
        (
            "tests/fixtures/released-v02/inflight.rs",
            include_bytes!("../../fixtures/released-v02/inflight.rs"),
        ),
        (
            "tests/fixtures/released-v02/control.rs",
            include_bytes!("../../fixtures/released-v02/control.rs"),
        ),
    ];
    assert_eq!(overlays.len(), expected.len() + 1);
    for (name, bytes) in expected {
        let matching: Vec<_> = overlays
            .iter()
            .filter(|entry| entry["path"] == name)
            .collect();
        assert_eq!(matching.len(), 1, "exact declared overlay");
        assert_eq!(matching[0]["sha256"], control::sha(bytes));
    }
    let main: Vec<_> = overlays
        .iter()
        .filter(|entry| entry["path"] == "tests/integration/workflow-primitives/main.rs")
        .collect();
    assert_eq!(main.len(), 1);
    // Actual immutable v0.2.0 main.rs plus the sole declared phase registration.
    assert_eq!(
        main[0]["sha256"],
        "6188765ccfa57f3ae61f0cf9b94a95f362e2acb3439eb191282ad9e4f587eafd"
    );
}

async fn live_lease(pool: &sqlx::PgPool, tenant: TenantId, run: RunId, witness: &Value) {
    let row:(String,i32,i64,i64,String,bool)=sqlx::query_as("SELECT status,attempt,generation,floor(extract(epoch FROM lease_expires_at)*1000)::bigint,encode(lease_token_hash,'hex'),lease_expires_at>clock_timestamp()+interval '600 seconds' FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3 AND node='charge' AND instance_path_nodes=ARRAY['lines']::text[] AND instance_path_indices=ARRAY[0]::int[]")
        .bind(tenant.as_uuid()).bind(run.as_uuid()).bind(witness["step"].as_str().expect("old physical identity").parse::<Uuid>().expect("step UUID")).fetch_one(pool).await.expect("actual old leased row");
    assert_eq!(row.0, "LEASED");
    assert_eq!(
        i64::from(row.1),
        witness["attempt"].as_i64().expect("old attempt")
    );
    assert_eq!(row.2, witness["generation"].as_i64().expect("old fence"));
    assert_eq!(
        row.3,
        witness["expiresAtMs"].as_i64().expect("old wire expiry")
    );
    assert_eq!(
        row.4,
        witness["tokenSha256"]
            .as_str()
            .expect("old token commitment")
    );
    assert!(row.5, "lease margin must exceed the entire bounded handoff");
}

#[tokio::test]
#[ignore = "requires verified actual released source; use the released-v02 profile runner"]
async fn a_released_worker_reports_and_replays_its_old_lease_after_server_upgrade() {
    assert!(
        std::env::var_os("SAGAT_TEST_DATABASE_URL").is_none(),
        "coordinator owns disposable PostgreSQL"
    );
    let build_path =
        PathBuf::from(std::env::var_os("SAGAT_V02_BUILD").expect("verified build manifest"));
    let build: Value = serde_json::from_slice(&std::fs::read(&build_path).expect("build metadata"))
        .expect("build JSON");
    assert_eq!(build["releaseCommit"], RELEASE);
    assert!(
        build["productionFilesVerified"]
            .as_u64()
            .expect("source count")
            > 600
    );
    assert_eq!(
        build["coordinatorSha256"],
        control::sha(include_bytes!("released_v02_inflight.rs")),
        "runner must bind the actually executed coordinator"
    );
    assert_eq!(
        build["coordinatorPath"],
        "tests/integration/workflow-primitives/released_v02_inflight.rs"
    );
    verify_overlays(&build);
    let binary = PathBuf::from(build["binary"].as_str().expect("old binary"));
    assert_eq!(
        binary.canonicalize().expect("old binary").parent(),
        build_path.canonicalize().expect("build path").parent()
    );
    assert_eq!(
        super::released_v02_upgrade::file_sha(&binary),
        build["binarySha256"].as_str().expect("binary hash")
    );
    bounded::lifecycle("released in-flight worker report",Duration::from_secs(240),|progress|async move {
        progress.enter("owned PostgreSQL and anonymous phase socket");
        let owner=support::fresh_db().await;
        let (parent,old)=std::os::unix::net::UnixStream::pair().expect("anonymous socket pair");
        parent.set_nonblocking(true).expect("nonblocking parent phase socket");
        let mut stream=UnixStream::from_std(parent).expect("async parent phase socket");
        let mut command=tokio::process::Command::new(&binary);
        command.arg("released_v02_phase::inflight::report_old_lease_to_current_server").args(["--exact","--nocapture"])
            .env("SAGAT_TEST_DATABASE_URL",&owner.admin_url).env("RUST_TEST_THREADS","1")
            .stdin(Stdio::from(OwnedFd::from(old))).kill_on_drop(true);
        let mut child=command.spawn().expect("actual released worker process");
        progress.enter("old public producer retains live scoped capability");
        let witness=bounded::within("old leased-task barrier",Duration::from_secs(60),control::receive(&mut stream)).await;
        assert_eq!(witness["releaseCommit"],RELEASE);
        let tenant=TenantId::new(witness["tenant"].as_str().expect("tenant").parse().expect("tenant UUID"));
        let run:RunId=witness["run"].as_str().expect("run").parse().expect("run UUID");
        let db=support::attach_existing_db(&owner.admin_url,witness["database"].as_str().expect("old owned database")).await;
        let before:i64=sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success").fetch_one(&db.pool).await.expect("old migration boundary");
        assert_eq!(before,20261220000000);
        live_lease(&db.pool,tenant,run,&witness).await;
        progress.enter("normal migration and current runtime-role endpoint");
        sagat_infra_postgres::migrate(&db.pool).await.expect("current normal migration");
        let after:i64=sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success").fetch_one(&db.pool).await.expect("current migration boundary");
        assert_eq!(after,20261223000000);
        let api=support::api::start_on_db(db,Options {runtime_role:true,..Options::default()}).await;
        let role:String=sqlx::query_scalar("SELECT current_user").fetch_one(&api.api_pool).await.expect("current service role");
        assert_eq!(role,"sagat_app");
        api.executions.verify_run(tenant,run).await.expect("current reader preserves old leased projection");
        let logical_before:i64=sqlx::query_scalar("SELECT logical_output_bytes FROM run_instance_usage WHERE tenant_id=$1 AND run_id=$2").bind(tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&api.db.pool).await.expect("retained logical bytes before report");
        // Check again immediately before GO; inline report acceptance alone does not prove liveness.
        live_lease(&api.db.pool,tenant,run,&witness).await;
        let token=api.idp.token("released-inflight-worker","acme").build();
        control::send(&mut stream,&json!({"url":api.grpc_url,"token":token})).await;
        progress.enter("old generated client reports and replays against current service");
        let acknowledged=bounded::within("old client completion barrier",Duration::from_secs(60),control::receive(&mut stream)).await;
        assert_eq!(acknowledged["reported"],true);assert_eq!(acknowledged["replayed"],true);
        assert_eq!(acknowledged["requestSha256"],witness["requestSha256"]);
        let status=bounded::within("join actual old worker",Duration::from_secs(30),child.wait()).await.expect("join old worker");
        assert!(status.success(),"released worker phase failed");
        let (head,run_status):(i64,String)=sqlx::query_as("SELECT run_version,status FROM runs WHERE tenant_id=$1 AND id=$2").bind(tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&api.db.pool).await.expect("committed report head");
        assert_eq!(acknowledged["command"],format!("{run}:{head}"));
        assert_eq!(run_status,"RUNNING","worker report must leave actual work for the current scheduler");
        let physical:(i32,i64,String,bool)=sqlx::query_as("SELECT attempt,generation,encode(reported_lease_token_hash,'hex'),lease_token_hash IS NULL AND lease_expires_at IS NULL AND leased_by IS NULL AND attempt_deadline_at IS NULL FROM steps WHERE tenant_id=$1 AND run_id=$2 AND step_id=$3").bind(tenant.as_uuid()).bind(run.as_uuid()).bind(witness["step"].as_str().expect("step").parse::<Uuid>().expect("UUID")).fetch_one(&api.db.pool).await.expect("durable original capability receipt");
        assert_eq!(i64::from(physical.0),witness["attempt"].as_i64().expect("attempt"));
        assert_eq!(physical.1,witness["generation"].as_i64().expect("generation"));
        assert_eq!(physical.2,witness["tokenSha256"].as_str().expect("token commitment"));assert!(physical.3);
        let attempt=u32::try_from(physical.0).expect("positive old attempt").to_be_bytes();
        let generation=u64::try_from(physical.1).expect("positive old fence").to_be_bytes();
        let mut fingerprint=Vec::new();
        for part in [b"complete".as_slice(),attempt.as_slice(),generation.as_slice(),control::OUTPUT.as_bytes()] {
            fingerprint.extend_from_slice(&u64::try_from(part.len()).expect("bounded fingerprint part").to_be_bytes());
            fingerprint.extend_from_slice(part);
        }
        let request_hash=control::sha(&fingerprint);
        let token_prefix=witness["tokenSha256"].as_str().expect("validated old token commitment").get(..16).expect("token hash prefix");
        let receipt_scope=format!("step:{run}:scoped:5:lines:0;6:charge:{token_prefix}");
        let receipt_resource=format!("SUCCEEDED|{run_status}|{head}");
        let report:(String,Vec<u8>,i64,i64,i64)=sqlx::query_as("SELECT s.status,p.content,u.logical_output_bytes,(SELECT count(*) FROM run_events e WHERE e.tenant_id=s.tenant_id AND e.run_id=s.run_id AND e.node=s.node AND e.instance_path_nodes=s.instance_path_nodes AND e.instance_path_indices=s.instance_path_indices AND e.event_type='step.completed'),(SELECT count(*) FROM command_dedupe d WHERE d.tenant_id=s.tenant_id AND d.idempotency_key='released-inflight-report' AND d.scope=$4 AND d.resource_id=$5 AND encode(d.request_hash,'hex')=$6) FROM steps s JOIN payloads p ON p.tenant_id=s.tenant_id AND p.digest=s.output_digest JOIN run_instance_usage u ON u.tenant_id=s.tenant_id AND u.run_id=s.run_id WHERE s.tenant_id=$1 AND s.run_id=$2 AND s.step_id=$3")
            .bind(tenant.as_uuid()).bind(run.as_uuid()).bind(witness["step"].as_str().expect("old step").parse::<Uuid>().expect("UUID")).bind(&receipt_scope).bind(&receipt_resource).bind(&request_hash).fetch_one(&api.db.pool).await.expect("one durable report and exact retained output");
        assert_eq!(report.0,"SUCCEEDED");assert_eq!(report.1,control::OUTPUT.as_bytes());
        assert_eq!(report.2-logical_before,i64::try_from(control::OUTPUT.len()).expect("bounded output length"));
        assert_eq!((report.3,report.4),(1,1));
        progress.enter("stopped old owner expires and current scheduler finishes");
        let old_owner=witness["owner"].as_str().expect("old owner");
        let old_generation:i64=sqlx::query_scalar("SELECT generation FROM scheduler_shards WHERE shard=run_shard($1) AND owner=$2 AND expires_at>clock_timestamp()").bind(run.as_uuid()).bind(old_owner).fetch_one(&api.db.pool).await.expect("stopped old owner's still-live relevant shard");
        let member=sqlx::query("UPDATE scheduler_members SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE owner=$1").bind(old_owner).execute(&api.db.pool).await.expect("expire stopped member").rows_affected();assert_eq!(member,1);
        let shards=sqlx::query("UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE owner=$1").bind(old_owner).execute(&api.db.pool).await.expect("expire stopped shards").rows_affected();assert!(shards>0);
        let placement=Placement::new(format!("released-inflight-current-{run}"));
        Keeper::new(api.executions.clone(),placement.clone(),KeeperSettings {ttl:Duration::from_secs(120)}).round().await.expect("current takeover");
        let takeover:(String,i64,bool)=sqlx::query_as("SELECT owner,generation,expires_at>clock_timestamp() FROM scheduler_shards WHERE shard=run_shard($1)").bind(run.as_uuid()).fetch_one(&api.db.pool).await.expect("current exact shard");
        assert_eq!(takeover.0,placement.owner());assert!(takeover.1>old_generation && takeover.2);
        let scheduler=Scheduler::new(api.executions.clone(),Engine::new(JsonEvaluator),SystemEntropy,SchedulerSettings::default()).with_placement(placement);
        let mut settled=false;
        let mut worked=0;
        for _ in 0..32 {
            let report=scheduler.tick().await;assert_eq!(report.failed,0);
            if !report.did_work(){settled=true;break;}
            worked+=1;
        }
        assert!(settled,"current scheduler exceeded the bounded finish loop");
        assert!(worked>0,"current scheduler must do actual post-report work");
        let status:String=sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2").bind(tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&api.db.pool).await.expect("terminal run");assert_eq!(status,"SUCCEEDED");
        api.executions.verify_run(tenant,run).await.expect("full terminal replay after old in-flight report");
        let result=json!({"releaseCommit":RELEASE,"oldWorkerReport":true,"exactReceiptReplay":true,"singleDurableCompletion":true,"logicalBytesChargedOnce":true,"oldImmutablePrefix":true,"schemaBefore":before,"schemaAfter":after,"currentSchedulerWorkingTicks":worked,"run":"SUCCEEDED"});
        let directory=tempfile::Builder::new().prefix("inflight-result-").tempdir_in(build_path.parent().expect("owned artifact parent")).expect("metadata directory").keep();
        std::fs::write(directory.join("result.json"),serde_json::to_vec_pretty(&result).expect("result metadata")).expect("write non-sensitive result");
        println!("released-v02 old lease: actual released client report/replay, one durable completion and current final replay passed");
    }).await;
}
