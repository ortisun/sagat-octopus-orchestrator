//! Public management and gRPC completion use production handlers. Only broker networking uses a
//! standing fixture: production egress cannot permit the test container's loopback address.
use super::support;
use reqwest::Method;
use sagat_application::outbound::OutboundStore as _;
use sagat_application::{
    entropy::SystemEntropy,
    outbound::{BrokerFailure, BrokerPublisher, BrokerRequest},
};
use sagat_domain::outbound::BrokerDestination;
use sagat_infra_connectors::{guard::Unguarded, publish::Publisher, webhook::Clients};
use sagat_scheduler::{DeliverySettings, DeliveryWorker};
use serde_json::json;
use support::{
    api::Options,
    receiver::{LoopbackSender, OpenFactory},
    worker::{platform_with, single_ir},
};

struct Standing {
    inner: Publisher<Unguarded, sagat_infra_postgres::PgRunStore, OpenFactory>,
    endpoint: String,
}
impl BrokerPublisher for Standing {
    async fn publish(&self, request: &BrokerRequest) -> Result<(), BrokerFailure> {
        assert_eq!(
            request.destination,
            BrokerDestination::redis("redis://redis.example.test:6379", "api:callbacks", 0)
                .expect("declared destination")
        );
        assert!(request.credentials.is_none());
        self.inner
            .publish(&BrokerRequest {
                tenant_id: request.tenant_id,
                namespace_id: request.namespace_id,
                subscription_id: request.subscription_id,
                destination: BrokerDestination::redis(&self.endpoint, "api:callbacks", 0)
                    .expect("standing destination"),
                credentials: None,
                headers: request.headers.clone(),
                body: request.body.clone(),
                timeout: request.timeout,
            })
            .await
    }
}

#[tokio::test]
async fn public_redis_subscription_and_grpc_run_completion_reach_the_real_stream() {
    let (_redis, url, client) = super::redis().await;
    let p = platform_with(Options {
        redis_callbacks: true,
        ..Options::default()
    })
    .await;
    let ns = p.namespace_id().await;
    let (status, policy) = p
        .rest(
            Method::PUT,
            &format!("/namespaces/{ns}/egress-policy"),
            Some(&json!({"rules":["redis.example.test"],"ports":[6379],"allowHttp":true})),
        )
        .await;
    assert_eq!(status, 200, "{policy}");
    let path = format!("/namespaces/{ns}/broker-subscriptions");
    let body = json!({"name":"redis-public", "kind":"REDIS", "settings":{"server":"redis://redis.example.test:6379", "stream":"api:callbacks"}, "events":["run.finished"]});
    let (status, subscription) = p.rest(Method::POST, &path, Some(&body)).await;
    assert_eq!(status, 201, "{subscription}");
    assert_eq!(subscription["kind"], "REDIS");
    assert_eq!(subscription["settings"]["database"], 0);
    let id = subscription["id"].as_str().expect("subscription id");
    let response = p.api.get_as(&format!("{path}/{id}"), "bob", "globex").await;
    assert_eq!(response.status(), 404);
    let spec = support::contract::OpenApi::load();
    spec.validate(
        &json!({"$ref":"#/components/schemas/BrokerSubscription"}),
        &subscription,
    )
    .expect("response conforms to canonical schema");
    let workflow = p
        .publish("redis-api-run", &single_ir("redis-api-run", None))
        .await;
    let run = p.start(&workflow, &json!({})).await;
    let mut worker = p.worker("redis-worker").await;
    worker.register(&["act.a"]).await;
    let task = worker.one_task().await;
    worker
        .complete(&task, &json!({"ok":true}))
        .await
        .expect("gRPC completion");
    assert_eq!(p.status(&run).await, "SUCCEEDED");
    let deliveries = DeliveryWorker::with_broker(
        p.api.executions.clone(),
        LoopbackSender::new(),
        Standing {
            inner: Publisher::with_parts(
                Unguarded,
                Clients::with_factory(p.api.executions.clone(), OpenFactory),
            ),
            endpoint: url.trim_end_matches("/0").to_owned(),
        },
        SystemEntropy,
        p.api.keys.cipher(),
        DeliverySettings::default(),
    );
    assert_eq!(deliveries.tick().await.delivered, 1);
    let entries = super::read(&client, "api:callbacks").await.ids;
    assert_eq!(entries.len(), 1);
    let (status, page) = p
        .rest(Method::GET, &format!("{path}/{id}/deliveries"), None)
        .await;
    assert_eq!(status, 200);
    assert_eq!(page["items"][0]["status"], "DELIVERED");
    assert_eq!(
        entries[0].get::<String>("delivery-id").expect("id"),
        page["items"][0]["id"].as_str().expect("id")
    );
}

#[tokio::test]
async fn redis_migration_down_refuses_retained_records_and_is_reversible_before_use() {
    let w = support::engine::world().await;
    let subscription = super::callbacks::subscribe(&w, "redis://127.0.0.1:6379/0").await;
    let error = sagat_infra_postgres::MIGRATOR
        .undo(&w.db.pool, 20261220000000)
        .await
        .expect_err("retained Redis rows prevent rollback");
    assert!(
        error
            .to_string()
            .contains("cannot roll back Redis callbacks")
    );
    assert!(
        w.store
            .subscription(w.admin.tenant_id(), subscription.id)
            .await
            .expect("query")
            .is_some()
    );
    let db = support::migrated_db().await;
    sagat_infra_postgres::MIGRATOR
        .undo(&db.pool, 20261220000000)
        .await
        .expect("empty migration down");
    sagat_infra_postgres::migrate(&db.pool)
        .await
        .expect("up again");
}
