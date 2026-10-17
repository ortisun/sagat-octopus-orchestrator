//! Helpers for the tests of collections whose items come from a blob (T014-016, D420-D428): a
//! platform with an object store, REST uploads and starts, a scheduler with a blob source and a
//! scoped (protocol 1.5) worker. Shared by the integration and fault suites.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use reqwest::Method;
use sagat_application::blobs::BlobLimits;
use sagat_application::collection_source::CollectionItemSource;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_domain::identity::NamespaceId;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use uuid::Uuid;

use super::api::{Options, json_of};
use super::worker::{HUMAN, NAMESPACE, Platform, TENANT, Worker, platform_with};

pub type Sched = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

pub const MIB: usize = 1024 * 1024;

pub async fn platform() -> Platform {
    let platform = platform_with(Options {
        runtime_role: true,
        blobs: Some(BlobLimits {
            max_bytes: 16 * MIB as u64,
            max_pending: 64,
            ..BlobLimits::default()
        }),
        ..Options::default()
    })
    .await;
    let (status, body) = platform
        .rest(
            Method::PUT,
            "/quotas/iteration_concurrency",
            Some(&json!({"limit": 8})),
        )
        .await;
    assert_eq!(status, 200, "{body}");
    platform
}

pub fn body_nodes() -> Value {
    json!([
        {"id": "line", "type": "activity", "activity": "src.line", "next": "returned"},
        {"id": "returned", "type": "return", "result": "steps.line.output"}
    ])
}

/// `items` over `expression`; the end node returns the aggregate.
pub fn collection_ir(name: &str, expression: &str, max_items: u32) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "items",
        "bodies": {"line": {"entrypoint": "line", "nodes": body_nodes()}},
        "nodes": [
            {"id": "items", "type": "forEach", "items": expression, "body": "line",
             "result": "bodyResult", "maxItems": max_items, "maxConcurrency": 4, "next": "done"},
            {"id": "done", "type": "end", "outputExpr": "steps.items.output"}
        ]
    })
}

pub fn sha(bytes: &[u8]) -> String {
    sagat_domain::workflow::Digest::new(Sha256::digest(bytes).into()).to_hex()
}

/// A run-input upload by `alice` of acme; returns the descriptor `{id,size,sha256,contentType}`.
pub async fn upload_as(
    p: &Platform,
    who: &str,
    tenant: &str,
    namespace: &str,
    data: &[u8],
) -> Value {
    let response = p
        .api
        .as_user(Method::POST, "/run-input-blobs", who, tenant)
        .header("x-octopus-namespace", namespace)
        .header("x-octopus-blob-size", data.len())
        .header("x-octopus-blob-sha256", sha(data))
        .body(data.to_vec())
        .send()
        .await
        .expect("upload");
    let status = response.status();
    let body = json_of(response).await;
    assert_eq!(status, 201, "{body}");
    body
}

pub async fn upload(p: &Platform, data: &[u8]) -> Value {
    upload_as(p, HUMAN, TENANT, NAMESPACE, data).await
}

pub async fn start_with_blob_as(
    p: &Platform,
    who: &str,
    tenant: &str,
    namespace: &str,
    workflow: &str,
    blob_id: &str,
) -> Uuid {
    let response = p
        .api
        .as_user(Method::POST, "/runs", who, tenant)
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", format!("start-{}", Uuid::now_v7()))
        .json(&json!({"workflowId": workflow, "inputBlobRef": blob_id}))
        .send()
        .await
        .expect("start with a blob");
    let status = response.status();
    let body = json_of(response).await;
    assert_eq!(status, 202, "{body}");
    body["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("uuid")
}

pub async fn start_with_blob(p: &Platform, workflow: &str, blob_id: &str) -> Uuid {
    start_with_blob_as(p, HUMAN, TENANT, NAMESPACE, workflow, blob_id).await
}

pub async fn start_inline(p: &Platform, workflow: &str, input: &Value) -> Uuid {
    p.start(workflow, input).await.parse().expect("uuid")
}

pub async fn scheduler(p: &Platform, source: Option<Arc<dyn CollectionItemSource>>) -> Sched {
    let placement = Placement::new(format!("sources-{}", Uuid::now_v7()));
    Keeper::new(
        p.api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire shards");
    let scheduler = Scheduler::new(
        p.api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_children(DefinitionCache::new(Arc::new(JsonIrCodec), 64))
    .with_placement(placement);
    match source {
        Some(source) => scheduler.with_item_source(source),
        None => scheduler,
    }
}

pub fn blob_source(p: &Platform) -> Arc<dyn CollectionItemSource> {
    Arc::new(p.api.blobs.clone().expect("a blob service"))
}

pub async fn status_and_code(p: &Platform, run: Uuid) -> (String, Option<String>) {
    let row = sqlx::query("SELECT status, stop_code FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("run row");
    (row.get("status"), row.get("stop_code"))
}

pub async fn scopes(p: &Platform, run: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM iteration_scopes WHERE run_id = $1")
        .bind(run)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("scope count")
}

/// Ticks until the run is no longer `RUNNING`/`WAITING`/`CANCELLING` or `limit` passes were made.
pub async fn tick_until_settled(p: &Platform, scheduler: &Sched, run: Uuid, limit: usize) {
    for _ in 0..limit {
        scheduler.tick().await;
        let (status, _) = status_and_code(p, run).await;
        if matches!(
            status.as_str(),
            "SUCCEEDED" | "FAILED" | "CANCELLED" | "COMPENSATED" | "PARTIAL"
        ) {
            return;
        }
    }
}

/// A worker that speaks protocol 1.5, so it may be leased scoped work.
pub async fn scoped_worker<'a>(p: &'a Platform, name: &str, activities: &[&str]) -> Worker<'a> {
    let mut worker = p.worker(name).await;
    worker.register_version(activities, "1.5").await;
    worker
}

/// Completes every leased iteration with the `v` field of its item, last polled first, until the
/// scheduler has nothing more to do. Returns the items the worker saw, by iteration index.
pub async fn run_to_the_end(
    p: &Platform,
    scheduler: &Sched,
    worker: &mut Worker<'_>,
    run: Uuid,
) -> Vec<(u32, Value)> {
    let mut seen = Vec::new();
    for _ in 0..200 {
        scheduler.tick().await;
        let mut tasks = worker.poll(8).await;
        tasks.reverse();
        for task in tasks {
            let input: Value = serde_json::from_str(&task.input_json).expect("task input");
            let index = task
                .instance
                .as_ref()
                .and_then(|instance| instance.instance_path.first())
                .map(|segment| segment.iteration_index)
                .expect("a scoped task");
            seen.push((index, input["item"].clone()));
            worker
                .complete(&task, &input["item"]["v"])
                .await
                .expect("complete the iteration");
        }
        let (status, _) = status_and_code(p, run).await;
        if status == "SUCCEEDED" {
            break;
        }
    }
    seen.sort_by_key(|(index, _)| *index);
    seen
}

pub async fn aggregate(p: &Platform, run: Uuid) -> Value {
    sqlx::query_scalar(
        "SELECT convert_from(payload.content, 'UTF8')::jsonb \
         FROM iteration_scopes scope JOIN payloads payload \
           ON payload.tenant_id = scope.tenant_id AND payload.digest = scope.result_digest \
         WHERE scope.run_id = $1 AND scope.owner_node = 'items'",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("the aggregate of the collection")
}

pub fn descriptor_of(upload: &Value) -> Value {
    json!({"$blob": upload})
}

pub async fn namespace_of(p: &Platform) -> NamespaceId {
    NamespaceId::new(p.namespace_id().await.parse().expect("namespace id"))
}
