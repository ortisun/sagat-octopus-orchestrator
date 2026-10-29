//! Explicit cold callback cutover; no mixed-generation or automatic deployment fence claim.
use super::release_upgrade_inflight::control;
use crate::support::{
    self,
    api::Options,
    bounded,
    receiver::{LoopbackSender, OpenFactory},
    worker::Platform,
};
use sagat_application::{
    entropy::SystemEntropy,
    outbound::{BrokerFailure, BrokerPublisher, BrokerRequest, OutboundStore},
};
use sagat_domain::{
    execution::RunId,
    identity::TenantId,
    outbound::{BrokerDestination, SubscriptionId},
};
use sagat_infra_connectors::{guard::Unguarded, publish::Publisher, webhook::Clients};
use sagat_scheduler::{DeliverySettings, DeliveryWorker};
use serde_json::{Value, json};
use sqlx::Connection as _;
use std::{os::fd::OwnedFd, path::PathBuf, process::Stdio, time::Duration};
use tokio::net::UnixStream;
const RELEASE: &str = "bfda342073db71e84f30bdc79ccf3fbc97a7153c";

struct OwnedRedis {
    endpoint: String,
    inner: Publisher<Unguarded, sagat_infra_postgres::PgRunStore, OpenFactory>,
}
impl BrokerPublisher for OwnedRedis {
    async fn publish(&self, request: &BrokerRequest) -> Result<(), BrokerFailure> {
        assert_eq!(
            request.destination,
            BrokerDestination::redis("redis://redis.example.test:6379", "cutover:callbacks", 0)
                .expect("public destination")
        );
        assert!(request.credentials.is_none());
        self.inner
            .publish(&BrokerRequest {
                tenant_id: request.tenant_id,
                namespace_id: request.namespace_id,
                subscription_id: request.subscription_id,
                destination: BrokerDestination::redis(&self.endpoint, "cutover:callbacks", 0)
                    .expect("owned standing destination"),
                credentials: None,
                headers: request.headers.clone(),
                body: request.body.clone(),
                timeout: request.timeout,
            })
            .await
    }
}

#[tokio::test]
#[ignore = "requires verified actual released source; use the release-upgrade profile runner"]
async fn released_callback_writer_is_fenced_before_redis_activation() {
    assert!(
        std::env::var_os("SAGAT_TEST_DATABASE_URL").is_none(),
        "owns its disposable server"
    );
    let manifest =
        PathBuf::from(std::env::var_os("SAGAT_V02_BUILD").expect("verified build manifest"));
    let build: Value =
        serde_json::from_slice(&std::fs::read(&manifest).expect("manifest")).expect("JSON");
    assert_eq!(build["releaseCommit"], RELEASE);
    assert_eq!(build["scenario"], "redis-callback-cutover");
    assert_eq!(
        build["coordinatorPath"],
        "tests/integration/workflow-primitives/release_upgrade_callback_cutover.rs"
    );
    assert_eq!(
        build["coordinatorSha256"],
        control::sha(include_bytes!("release_upgrade_callback_cutover.rs"))
    );
    super::release_upgrade_inflight::verify_overlays(&build);
    let binary = PathBuf::from(build["binary"].as_str().expect("old executable"));
    assert_eq!(
        binary.canonicalize().expect("binary").parent(),
        manifest.canonicalize().expect("manifest").parent()
    );
    assert_eq!(
        super::release_upgrade::file_sha(&binary),
        build["binarySha256"]
    );
    assert_eq!(build["productionFilesVerified"], 649);
    bounded::lifecycle("released callback publisher cold cutover",Duration::from_secs(240),|progress|async move {
        let owner=support::fresh_db().await;
        let (parent,old)=std::os::unix::net::UnixStream::pair().expect("private phase socket");
        parent.set_nonblocking(true).expect("nonblocking parent");
        let mut stream=UnixStream::from_std(parent).expect("async parent");
        let mut command=tokio::process::Command::new(binary);
        command.arg("release_upgrade_phase::callback_cutover::deliver_http_callback_then_wait_for_cold_cutover")
            .args(["--exact","--nocapture"]).env("SAGAT_TEST_DATABASE_URL",&owner.admin_url)
            .env("RUST_TEST_THREADS","1").stdin(Stdio::from(OwnedFd::from(old))).kill_on_drop(true);
        let mut old_process=command.spawn().expect("actual released callback publisher");
        progress.enter("actual released HTTP delivery and native reader");
        let witness=bounded::within("released callback writer ready",Duration::from_secs(120),control::receive(&mut stream)).await;
        assert_eq!(witness["releaseCommit"],RELEASE);assert_eq!(witness["oldPublisherDelivered"],true);assert_eq!(witness["oldReaderDecoded"],true);
        assert_eq!(witness["oldLogin"],"released_callback_writer");assert_eq!(witness["schemaBefore"].as_i64(),Some(20261220000000));
        assert_eq!(witness["oldBodySha256"].as_str().expect("old body digest").len(),64);
        let db=support::attach_existing_db(&owner.admin_url,witness["database"].as_str().expect("old DB")).await;
        let tenant=TenantId::new(witness["tenant"].as_str().expect("tenant").parse().expect("UUID"));
        let old_subscription:SubscriptionId=witness["subscription"].as_str().expect("old subscription").parse().expect("UUID");
        assert_eq!(control::callback_source(&db.pool,tenant.as_uuid(),old_subscription.as_uuid()).await,witness["sourceSha256"]);
        let old_url=db.url_for("released_callback_writer","fixture-only-callback-writer");
        let mut existing=sqlx::PgConnection::connect(&old_url).await.expect("old login works before fence");
        let role:String=sqlx::query_scalar("SELECT current_user").fetch_one(&mut existing).await.expect("old identity");assert_eq!(role,"released_callback_writer");
        progress.enter("revoke old login and terminate authenticated sessions");
        sqlx::query("ALTER ROLE released_callback_writer NOLOGIN").execute(&db.pool).await.expect("operator login revocation");
        let terminated:Vec<bool>=sqlx::query_scalar("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename='released_callback_writer'").fetch_all(&db.pool).await.expect("terminate only owned old-generation sessions");
        assert!(terminated.len()>=2 && terminated.iter().all(|v|*v));
        assert!(sqlx::query("SELECT 1").execute(&mut existing).await.is_err());
        let Err(refused)=sqlx::PgConnection::connect(&old_url).await else {panic!("old login must be refused");};
        assert_eq!(refused.as_database_error().and_then(|e|e.code()).as_deref(),Some("28000"));
        progress.enter("acknowledge stop and join actual old callback process");
        control::send(&mut stream,&json!({"stop":true})).await;
        assert_eq!(bounded::within("old publisher stop acknowledgement",Duration::from_secs(30),control::receive(&mut stream)).await["stopped"],true);
        assert!(bounded::within("join old callback publisher",Duration::from_secs(30),old_process.wait()).await.expect("join").success());
        let sessions:i64=sqlx::query_scalar("SELECT count(*) FROM pg_stat_activity WHERE usename='released_callback_writer'").fetch_one(&db.pool).await.expect("old sessions");assert_eq!(sessions,0);
        progress.enter("normal migration and explicit Redis activation after old process join");
        sagat_infra_postgres::migrate(&db.pool).await.expect("current normal migration");
        let after:i64=sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success").fetch_one(&db.pool).await.expect("current schema boundary");assert_eq!(after,20261226000000);
        let api=support::api::start_on_db(db,Options {runtime_role:true,redis_callbacks:true,..Options::default()}).await;
        let old_run:RunId=witness["run"].as_str().expect("old run").parse().expect("UUID");
        api.executions.verify_run(tenant,old_run).await.expect("old run replay after migration");
        let role:String=sqlx::query_scalar("SELECT current_user").fetch_one(&api.api_pool).await.expect("current role");assert_eq!(role,"sagat_app");
        let p=Platform {api,tenant,other_tenant:TenantId::new(witness["otherTenant"].as_str().expect("other tenant").parse().expect("UUID"))};
        let ns=witness["namespace"].as_str().expect("namespace");
        let (status,policy)=p.rest(reqwest::Method::PUT,&format!("/namespaces/{ns}/egress-policy"),Some(&json!({"rules":["redis.example.test"],"ports":[6379],"allowHttp":true}))).await;assert_eq!(status,200,"{policy}");
        let path=format!("/namespaces/{ns}/broker-subscriptions");
        let (status,created)=p.rest(reqwest::Method::POST,&path,Some(&json!({"name":"post-cutover-redis","kind":"REDIS","settings":{"server":"redis://redis.example.test:6379","stream":"cutover:callbacks"},"events":["run.finished"]}))).await;assert_eq!(status,201,"{created}");
        let subscription:SubscriptionId=created["id"].as_str().expect("new subscription").parse().expect("UUID");
        let workflow=p.publish("redis-after-released-callback-cutover",&support::worker::single_ir("redis-after-released-callback-cutover",None)).await;
        let run=p.start(&workflow,&json!({})).await;
        let mut worker=p.worker("current-cutover-worker").await;worker.register(&["act.a"]).await;
        let task=worker.one_task().await;worker.complete(&task,&json!({"current":true})).await.expect("current gRPC completion");
        assert_eq!(p.status(&run).await,"SUCCEEDED");
        let (_redis,url,client)=support::brokers::redis_stream().await;
        let deliveries=DeliveryWorker::with_broker(p.api.executions.clone(),LoopbackSender::new(),OwnedRedis {endpoint:url.trim_end_matches("/0").to_owned(),inner:Publisher::with_parts(Unguarded,Clients::with_factory(p.api.executions.clone(),OpenFactory))},SystemEntropy,p.api.keys.cipher(),DeliverySettings::default());
        let round=deliveries.tick().await;assert_eq!((round.attempted,round.delivered,round.failed),(1,1,0));
        let stored=p.api.executions.deliveries(tenant,subscription,None,None,10).await.expect("current delivery reader");assert_eq!(stored.len(),1);assert_eq!(stored[0].status.as_str(),"DELIVERED");
        let entries=support::brokers::read_redis_stream(&client,"cutover:callbacks").await.ids;assert_eq!(entries.len(),1);
        assert_eq!(entries[0].get::<String>("delivery-id").expect("delivery identity"),stored[0].id.to_string());
        assert_eq!(entries[0].get::<String>("event-id").expect("event identity"),stored[0].event_id);
        assert_eq!(entries[0].get::<Vec<u8>>("body").expect("exact event body"),stored[0].payload);
        assert_eq!(deliveries.tick().await.attempted,0);assert_eq!(support::brokers::read_redis_stream(&client,"cutover:callbacks").await.ids.len(),1);
        assert_eq!(control::callback_source(&p.api.api_pool,tenant.as_uuid(),old_subscription.as_uuid()).await,witness["sourceSha256"],"old HTTP settings, delivery and attempts unchanged after Redis activation");
        p.api.executions.verify_run(tenant,run.parse().expect("current run UUID")).await.expect("new run replay");
        let result=json!({"scenario":"redis-callback-cutover","releaseCommit":RELEASE,"schemaBefore":witness["schemaBefore"],"schemaAfter":after,"oldPublisherDelivered":true,"oldReaderDecoded":true,"oldLoginRevoked":true,"existingOldSessionsTerminated":true,"oldProcessJoinedBeforeActivation":true,"oldCallbackMetadataUnchanged":true,"newRedisDeliveries":1,"stableDeliveryEventBody":true,"oldAndNewRunsReplay":true,"mixedVersionSafety":false});
        std::fs::write(std::env::var_os("SAGAT_V02_RESULT").expect("selected result path"),serde_json::to_vec_pretty(&result).expect("JSON")).expect("non-sensitive result");
    }).await;
}
