//! Cold cutover is an operator sequence, not a mixed-version or automatic fencing promise.
use crate::support::{
    self,
    api::{Options, json_of},
    bounded,
    scope_flow::publish,
};
use sagat_application::{
    entropy::SystemEntropy,
    execution::{DefinitionCache, Engine},
};
use sagat_domain::{execution::RunId, identity::TenantId};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_scheduler::{
    EventDispatchSettings, EventDispatcher, Keeper, KeeperSettings, Placement, Scheduler,
    SchedulerSettings,
};
use serde_json::{Value, json};
use sqlx::Connection as _;
use std::{os::fd::OwnedFd, path::PathBuf, process::Stdio, sync::Arc, time::Duration};
use tokio::net::UnixStream;

use super::released_v02_inflight::control;
const RELEASE: &str = "bfda342073db71e84f30bdc79ccf3fbc97a7153c";

#[tokio::test]
#[ignore = "requires verified actual released source; use the released-v02 profile runner"]
async fn released_dispatcher_credentials_and_process_are_fenced_before_public_scoped_events() {
    assert!(
        std::env::var_os("SAGAT_TEST_DATABASE_URL").is_none(),
        "owns its disposable server"
    );
    let manifest =
        PathBuf::from(std::env::var_os("SAGAT_V02_BUILD").expect("verified build manifest"));
    let build: Value = serde_json::from_slice(&std::fs::read(&manifest).expect("build manifest"))
        .expect("build JSON");
    assert_eq!(build["releaseCommit"], RELEASE);
    assert_eq!(build["scenario"], "scoped-event-cutover");
    assert_eq!(
        build["coordinatorPath"],
        "tests/integration/workflow-primitives/released_v02_cutover.rs"
    );
    assert_eq!(
        build["coordinatorSha256"],
        control::sha(include_bytes!("released_v02_cutover.rs"))
    );
    super::released_v02_inflight::verify_overlays(&build);
    let binary = PathBuf::from(build["binary"].as_str().expect("released binary"));
    assert_eq!(
        binary.canonicalize().expect("binary").parent(),
        manifest.canonicalize().expect("manifest").parent()
    );
    assert_eq!(
        super::released_v02_upgrade::file_sha(&binary),
        build["binarySha256"]
    );
    assert!(
        build["productionFilesVerified"]
            .as_u64()
            .expect("verified source count")
            > 600
    );
    bounded::lifecycle("released dispatcher cold cutover",Duration::from_secs(240),|progress|async move {
        let owner=support::fresh_db().await;
        let (parent,old)=std::os::unix::net::UnixStream::pair().expect("private phase socket");
        parent.set_nonblocking(true).expect("nonblocking parent");
        let mut stream=UnixStream::from_std(parent).expect("async parent");
        let mut command=tokio::process::Command::new(binary);
        command.arg("released_v02_phase::cutover::dispatch_root_event_then_wait_for_cold_cutover")
            .args(["--exact","--nocapture"]).env("SAGAT_TEST_DATABASE_URL",&owner.admin_url)
            .env("RUST_TEST_THREADS","1").stdin(Stdio::from(OwnedFd::from(old))).kill_on_drop(true);
        let mut old_process=command.spawn().expect("actual released dispatcher process");
        progress.enter("actual released root delivery and old login barrier");
        let witness=bounded::within("released dispatcher ready",Duration::from_secs(90),control::receive(&mut stream)).await;
        assert_eq!(witness["releaseCommit"],RELEASE);
        assert_eq!(witness["oldDispatcherWrote"],true);
        assert_eq!(witness["oldLogin"],"released_cutover_dispatcher");
        let db=support::attach_existing_db(&owner.admin_url,witness["database"].as_str().expect("old database")).await;
        let old_url=db.url_for("released_cutover_dispatcher","fixture-only-dispatcher");
        let mut old_connection=sqlx::PgConnection::connect(&old_url).await.expect("old login still works before fence");
        let role:String=sqlx::query_scalar("SELECT current_user").fetch_one(&mut old_connection).await.expect("actual pre-fence old identity");
        assert_eq!(role,"released_cutover_dispatcher");
        progress.enter("revoke old login and terminate its already authenticated sessions");
        sqlx::query("ALTER ROLE released_cutover_dispatcher NOLOGIN").execute(&db.pool).await.expect("explicit old-generation credential fence");
        let terminated:Vec<bool>=sqlx::query_scalar("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename='released_cutover_dispatcher'")
            .fetch_all(&db.pool).await.expect("terminate only this fixture's old dispatcher sessions");
        assert!(terminated.len()>=2 && terminated.iter().all(|terminated|*terminated));
        assert!(sqlx::query("SELECT 1").execute(&mut old_connection).await.is_err(),"a pre-existing old connection must lose write access too");
        let Err(refused)=sqlx::PgConnection::connect(&old_url).await else {
            panic!("a fresh old login must be refused");
        };
        assert_eq!(refused.as_database_error().and_then(|error|error.code()).as_deref(),Some("28000"));
        progress.enter("stop acknowledgement and actual old process join before activation");
        control::send(&mut stream,&json!({"stop":true})).await;
        assert_eq!(bounded::within("old dispatcher stop acknowledgement",Duration::from_secs(30),control::receive(&mut stream)).await["stopped"],true);
        let exit=bounded::within("join stopped released dispatcher",Duration::from_secs(30),old_process.wait()).await.expect("join old process");
        assert!(exit.success());
        let sessions:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE usename='released_cutover_dispatcher'")
            .fetch_one(&db.pool).await.expect("no old sessions after join");
        assert_eq!(sessions,0);
        sagat_infra_postgres::migrate(&db.pool).await.expect("normal current migrations after old shutdown");
        let api=support::api::start_on_db(db,Options {runtime_role:true,..Options::default()}).await;
        let tenant=TenantId::new(witness["tenant"].as_str().expect("tenant").parse().expect("tenant UUID"));
        let old_run:RunId=witness["run"].as_str().expect("old run").parse().expect("run UUID");
        api.executions.verify_run(tenant,old_run).await.expect("old terminal run survives cutover");
        let role:String=sqlx::query_scalar("SELECT current_user").fetch_one(&api.api_pool).await.expect("current role");
        assert_eq!(role,"sagat_app");
        progress.enter("public scoped event publication only after credential fence and join");
        let name="public-event-after-released-cutover";
        let workflow=publish(&api,name,super::collection_events::document(name,false,true,60000)).await.expect("new scoped event capability publishes after cutover");
        let response=api.as_user(reqwest::Method::POST,"/runs","alice","acme")
            .header("x-octopus-namespace","payments").header("idempotency-key","scoped-event-cutover-start")
            .json(&json!({"workflowId":workflow,"input":{"lines":[{"sku":"new-first"},{"sku":"new-second"}]}}))
            .send().await.expect("new public run");
        assert_eq!(response.status(),reqwest::StatusCode::ACCEPTED);
        let run:RunId=json_of(response).await["runId"].as_str().expect("new run").parse().expect("run UUID");
        // The old owner is stopped; explicit expiry permits the current Keeper takeover.
        for table in ["scheduler_members","scheduler_shards"] {
            let statement=match table {
                "scheduler_members"=>"UPDATE scheduler_members SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE owner=$1",
                _=>"UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 microsecond' WHERE owner=$1",
            };
            assert!(sqlx::query(statement).bind(witness["owner"].as_str().expect("stopped owner")).execute(&api.db.pool).await.expect("expire stopped old authority").rows_affected()>0);
        }
        let placement=Placement::new("released-event-cutover-current");
        Keeper::new(api.executions.clone(),placement.clone(),KeeperSettings {ttl:Duration::from_secs(120)})
            .round().await.expect("current scheduler takeover");
        let scheduler=Scheduler::new(api.executions.clone(),Engine::new(JsonEvaluator),SystemEntropy,SchedulerSettings::default()).with_placement(placement);
        for _ in 0..16 {
            let round=scheduler.tick().await; assert_eq!(round.failed,0);
            if !round.did_work(){break;}
        }
        let waiting:i64=sqlx::query_scalar("SELECT count(*) FROM steps WHERE tenant_id=$1 AND run_id=$2 AND waiting_kind='EVENT' AND instance_path_nodes=ARRAY['lines']::text[]")
            .bind(tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&api.db.pool).await.expect("new scoped event rows");
        assert_eq!(waiting,2);
        for key in ["new-first","new-second"] {
            let response=api.as_user(reqwest::Method::POST,"/events","alice","acme")
                .header("x-octopus-namespace","payments")
                .json(&json!({"eventId":key,"type":"payment.confirmed","correlationKey":key,"data":{"new":true}}))
                .send().await.expect("new scoped event ingress");
            assert_eq!(response.status(),reqwest::StatusCode::ACCEPTED);
        }
        let receiver=EventDispatcher::new(api.executions.clone(),Engine::new(JsonEvaluator),SystemEntropy,
            DefinitionCache::new(Arc::new(JsonIrCodec),8),EventDispatchSettings::default());
        let round=receiver.tick().await;
        assert_eq!((round.steps_woken,round.retried),(2,0));
        for _ in 0..16 {
            let round=scheduler.tick().await; assert_eq!(round.failed,0);
            if !round.did_work(){break;}
        }
        let status:String=sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant.as_uuid()).bind(run.as_uuid()).fetch_one(&api.db.pool).await.expect("new terminal run");
        assert_eq!(status,"SUCCEEDED");
        api.executions.verify_run(tenant,run).await.expect("new scoped run replay after cutover");
        api.executions.verify_run(tenant,old_run).await.expect("unchanged old run replay after new execution");
        let result=json!({"scenario":"scoped-event-cutover","releaseCommit":RELEASE,"oldDispatcherWrote":true,"oldLoginRevoked":true,"existingOldSessionsTerminated":true,"oldProcessJoinedBeforePublication":true,"newScopedEventsDelivered":2,"oldAndNewRunsReplay":true,"mixedVersionSafety":false});
        std::fs::write(std::env::var_os("SAGAT_V02_RESULT").expect("selected phase result path"),serde_json::to_vec_pretty(&result).expect("result JSON")).expect("bounded result evidence");
    }).await;
}
