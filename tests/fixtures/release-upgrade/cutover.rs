//! Real released dispatcher, dedicated revocable login, then explicit process stop/join.
use super::inflight::control;
use crate::support::scope_flow::Flow;
use sagat_application::{
    entropy::SystemEntropy,
    execution::{DefinitionCache, Engine},
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_scheduler::{EventDispatchSettings, EventDispatcher};
use serde_json::json;
use std::{sync::Arc, time::Duration};

#[tokio::test]
async fn dispatch_root_event_then_wait_for_cold_cutover() {
    assert_eq!(env!("SAGAT_RELEASE_PROFILE_COMMIT"), super::RELEASE);
    crate::support::bounded::within("released dispatcher cutover",Duration::from_secs(120),async {
        let mut stream=control::inherited_stream();
        let name="released-cutover-root-event";
        let flow=Flow::start(name,json!({
            "apiVersion":"sagat-octopus-orchestrator.io/v1","kind":"Workflow",
            "metadata":{"name":name},"entrypoint":"payment","nodes":[
                {"id":"payment","type":"waitEvent","eventType":"payment.confirmed","correlationExpr":"input.lines[0].sku","timeoutMs":60000,"next":"done"},
                {"id":"done","type":"end"}
            ]
        }),json!([{"sku":"old-root"}])).await;
        flow.drive().await;
        // Static fixture-only credentials in an owned disposable server; never production secrets.
        sqlx::query("CREATE ROLE released_cutover_dispatcher LOGIN PASSWORD 'fixture-only-dispatcher' IN ROLE sagat_app")
            .execute(&flow.api.db.pool).await.expect("dedicated old-generation login");
        let pool=sqlx::postgres::PgPoolOptions::new().max_connections(2)
            .connect(&flow.api.db.url_for("released_cutover_dispatcher","fixture-only-dispatcher"))
            .await.expect("old dispatcher authenticates its own login");
        let role:(String,bool)=sqlx::query_as("SELECT current_user,rolsuper FROM pg_roles WHERE rolname=current_user")
            .fetch_one(&pool).await.expect("actual old role");
        assert_eq!(role,("released_cutover_dispatcher".into(),false));
        let store=sagat_infra_postgres::PgRunStore::new(pool.clone(),DefinitionCache::new(Arc::new(JsonIrCodec),8)).with_scoped_worker_protocol();
        let response=flow.api.as_user(reqwest::Method::POST,"/events","alice","acme")
            .header("x-octopus-namespace","payments")
            .json(&json!({"eventId":"released-cutover-old-event","type":"payment.confirmed","correlationKey":"old-root","data":{"old":true}}))
            .send().await.expect("old event ingress");
        assert_eq!(response.status(),reqwest::StatusCode::ACCEPTED);
        let receiver=EventDispatcher::new(store,Engine::new(JsonEvaluator),SystemEntropy,
            DefinitionCache::new(Arc::new(JsonIrCodec),8),EventDispatchSettings::default());
        let round=receiver.tick().await;
        assert_eq!((round.dispatched,round.steps_woken,round.retried),(1,1,0));
        flow.drive().await;
        assert_eq!(flow.run_status().await,"SUCCEEDED");
        flow.verify().await;
        control::send(&mut stream,&json!({"releaseCommit":super::RELEASE,"database":flow.api.db.name,"tenant":flow.tenant.to_string(),"run":flow.run.to_string(),"oldDispatcherWrote":true,"oldLogin":role.0,"owner":format!("scope-compensation-{name}")})).await;
        assert_eq!(control::receive(&mut stream).await["stop"],true);
        // No tick, reconnect or write is attempted after the parent revokes the login.
        drop(receiver);
        pool.close().await;
        control::send(&mut stream,&json!({"stopped":true})).await;
    }).await;
}
