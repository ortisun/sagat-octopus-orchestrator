//! Actual released callback writer/reader. Keys and signing values stay in this process.
use super::inflight::control;
use crate::support::{self, engine::{activity, document}, receiver::{LoopbackSender, Receiver}};
use sagat_application::{entropy::SystemEntropy, outbound::{Health, OutboundStore, SubscriptionRecord}};
use sagat_domain::{audit::{AuditAction, AuditRecord, ResourceType}, outbound::{DeliveryLimits, OutboundDestination, OutboundKind, SubscriptionId, SubscriptionStatus, TargetUrl}, secrets::SecretName};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{DeliverySettings, DeliveryWorker};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn deliver_http_callback_then_wait_for_cold_cutover() {
    assert_eq!(env!("SAGAT_RELEASE_PROFILE_COMMIT"),super::RELEASE);
    support::bounded::within("released callback writer",Duration::from_secs(120),async {
        let mut stream=control::inherited_stream();
        let w=support::engine::world().await;
        let before:i64=sqlx::query_scalar("SELECT max(version) FROM _sqlx_migrations WHERE success").fetch_one(&w.db.pool).await.expect("released schema boundary");assert_eq!(before,20261220000000);
        let receiver=Receiver::start().await;
        w.put_secret("callback-cutover-signing","fixture-only-callback-signing").await;
        sqlx::query("CREATE ROLE released_callback_writer LOGIN PASSWORD 'fixture-only-callback-writer' IN ROLE sagat_app")
            .execute(&w.db.pool).await.expect("dedicated released publisher login");
        let pool=sqlx::postgres::PgPoolOptions::new().max_connections(2)
            .connect(&w.db.url_for("released_callback_writer","fixture-only-callback-writer"))
            .await.expect("actual old publisher authenticates");
        let role:(String,bool)=sqlx::query_as("SELECT current_user,rolsuper FROM pg_roles WHERE rolname=current_user").fetch_one(&pool).await.expect("actual role");
        assert_eq!(role,("released_callback_writer".to_owned(),false));
        let store=PgRunStore::new(pool.clone(),w.cache.clone()).with_scoped_worker_protocol();
        let now=support::now();
        let mut subscription=SubscriptionRecord {
            id:SubscriptionId::generate(),tenant_id:w.admin.tenant_id(),namespace_id:w.namespace.id(),name:support::slug("old-callback"),
            destination:OutboundDestination::Webhook {url:TargetUrl::parse(&receiver.url).expect("owned receiver"),signing_secret:SecretName::parse("callback-cutover-signing").expect("secret name")},
            kinds:vec![OutboundKind::RunFinished],limits:DeliveryLimits::new(100,Duration::from_secs(2)).expect("limits"),status:SubscriptionStatus::Active,
            health:Health::default(),created_by:"test".to_owned(),created_at:now,updated_at:now,
        };
        let audit=AuditRecord::provisioning(subscription.tenant_id,AuditAction::WebhookSubscriptionCreated,ResourceType::WebhookSubscription,subscription.id.to_string(),"test",now);
        store.create_subscription(&subscription,100,&audit).await.expect("old native subscription writer");
        let (workflow,_)=w.publish(document("old-callback-run","work",vec![activity("work")])).await;
        let run=w.start(workflow,&json!({})).await;
        let session=w.session(&["act.work"]).await;
        let task=w.one_task(&session).await;
        w.complete(&session,&task,&json!({"old":true})).await.expect("old run completion");
        let worker=DeliveryWorker::new(store.clone(),LoopbackSender::new(),SystemEntropy,w.keys.cipher(),DeliverySettings::default());
        let round=worker.tick().await;assert_eq!((round.attempted,round.delivered,round.failed),(1,1,0));
        let seen=receiver.seen();assert_eq!(seen.len(),1);
        let deliveries=store.deliveries(w.admin.tenant_id(),subscription.id,None,None,10).await.expect("old native delivery reader");
        assert_eq!(deliveries.len(),1);assert_eq!(deliveries[0].status.as_str(),"DELIVERED");
        assert_eq!(seen[0].headers["x-octopus-delivery-id"],deliveries[0].id.to_string());
        assert_eq!(seen[0].body,deliveries[0].payload);
        subscription=store.subscription(w.admin.tenant_id(),subscription.id).await.expect("old native subscription reader").expect("old row");
        subscription.status=SubscriptionStatus::Paused;subscription.updated_at=support::now();
        let audit=AuditRecord::provisioning(subscription.tenant_id,AuditAction::WebhookSubscriptionChanged,ResourceType::WebhookSubscription,subscription.id.to_string(),"test",subscription.updated_at);
        assert!(store.update_subscription(&subscription,&audit).await.expect("audited native pause").is_some());
        store.verify_run(w.admin.tenant_id(),run).await.expect("released run replay");
        control::send(&mut stream,&json!({"releaseCommit":super::RELEASE,"database":w.db.name,"tenant":w.admin.tenant_id().to_string(),"otherTenant":w.other_admin.tenant_id().to_string(),"namespace":w.namespace.id().to_string(),"subscription":subscription.id.to_string(),"run":run.to_string(),"schemaBefore":before,"oldPublisherDelivered":true,"oldReaderDecoded":true,"sourceSha256":control::callback_source(&pool,w.admin.tenant_id().as_uuid(),subscription.id.as_uuid()).await,"oldBodySha256":control::sha(&seen[0].body),"oldLogin":role.0})).await;
        assert_eq!(control::receive(&mut stream).await["stop"],true);
        drop(worker);drop(store);pool.close().await;
        control::send(&mut stream,&json!({"stopped":true})).await;
    }).await;
}
