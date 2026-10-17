//! Collections whose items come from a blob (T014-016, D420-D428), against real PostgreSQL.
//!
//! The `items` expression yields the descriptor of a blob (a run input bound at start, or the
//! committed output of an earlier step); the scheduler reads the array once, at entry, and keeps it
//! as the immutable membership. These tests drive the public path: REST upload and start, the
//! scheduler, the worker protocol 1.5 over gRPC, and read what is persisted.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use reqwest::Method;
use sagat_application::blobs::{
    BackendFailure, BlobAccess, BlobBackend, BlobKey, BlobLimits, BlobService, BoxFuture,
    ByteStream, byte_stream,
};
use sagat_application::collection_source::CollectionItemSource;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_domain::blobs::BlobId;
use sagat_domain::identity::NamespaceId;
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgBlobCatalog, PgRunStore};
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use sqlx::Row;
use uuid::Uuid;

use crate::support::api::{Options, json_of};
use crate::support::worker::{HUMAN, NAMESPACE, Platform, TENANT, Worker, platform_with};

type Sched = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

const MIB: usize = 1024 * 1024;

async fn platform() -> Platform {
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

fn body_nodes() -> Value {
    json!([
        {"id": "line", "type": "activity", "activity": "src.line", "next": "returned"},
        {"id": "returned", "type": "return", "result": "steps.line.output"}
    ])
}

/// `items` over `expression`; the end node returns the aggregate.
fn collection_ir(name: &str, expression: &str, max_items: u32) -> Value {
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

fn sha(bytes: &[u8]) -> String {
    sagat_domain::workflow::Digest::new(Sha256::digest(bytes).into()).to_hex()
}

/// A run-input upload by `alice` of acme; returns the descriptor `{id,size,sha256,contentType}`.
async fn upload_as(p: &Platform, who: &str, tenant: &str, namespace: &str, data: &[u8]) -> Value {
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

async fn upload(p: &Platform, data: &[u8]) -> Value {
    upload_as(p, HUMAN, TENANT, NAMESPACE, data).await
}

async fn start_with_blob_as(
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

async fn start_with_blob(p: &Platform, workflow: &str, blob_id: &str) -> Uuid {
    start_with_blob_as(p, HUMAN, TENANT, NAMESPACE, workflow, blob_id).await
}

async fn start_inline(p: &Platform, workflow: &str, input: &Value) -> Uuid {
    p.start(workflow, input).await.parse().expect("uuid")
}

async fn scheduler(p: &Platform, source: Option<Arc<dyn CollectionItemSource>>) -> Sched {
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

fn blob_source(p: &Platform) -> Arc<dyn CollectionItemSource> {
    Arc::new(p.api.blobs.clone().expect("a blob service"))
}

async fn status_and_code(p: &Platform, run: Uuid) -> (String, Option<String>) {
    let row = sqlx::query("SELECT status, stop_code FROM runs WHERE id = $1")
        .bind(run)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("run row");
    (row.get("status"), row.get("stop_code"))
}

async fn scopes(p: &Platform, run: Uuid) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM iteration_scopes WHERE run_id = $1")
        .bind(run)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("scope count")
}

/// Ticks until the run is no longer `RUNNING`/`WAITING`/`CANCELLING` or `limit` passes were made.
async fn tick_until_settled(p: &Platform, scheduler: &Sched, run: Uuid, limit: usize) {
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
async fn scoped_worker<'a>(p: &'a Platform, name: &str, activities: &[&str]) -> Worker<'a> {
    let mut worker = p.worker(name).await;
    worker.register_version(activities, "1.5").await;
    worker
}

/// Completes every leased iteration with the `v` field of its item, last polled first, until the
/// scheduler has nothing more to do. Returns the items the worker saw, by iteration index.
async fn run_to_the_end(
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

async fn aggregate(p: &Platform, run: Uuid) -> Value {
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

fn descriptor_of(upload: &Value) -> Value {
    json!({"$blob": upload})
}

// ---------------------------------------------------------------------------------------------
// The happy paths
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_run_input_blob_array_is_the_membership_and_each_item_reaches_its_iteration() {
    let p = platform().await;
    let workflow = p
        .publish(
            "blob-source-input",
            &collection_ir("blob-source-input", "input", 10),
        )
        .await;
    // Whitespace, key order and duplicate values: none of it may leak into the membership.
    let source =
        "[ {\"v\": 10, \"k\": \"a\"},\n {\"k\": \"b\", \"v\": 20},\n {\"v\": 10, \"k\": \"a\"} ]";
    let blob = upload(&p, source.as_bytes()).await;
    let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    let mut worker = scoped_worker(&p, "blob-source-worker", &["src.line"]).await;

    let seen = run_to_the_end(&p, &scheduler, &mut worker, run).await;

    assert_eq!(
        seen,
        vec![
            (0, json!({"k": "a", "v": 10})),
            (1, json!({"k": "b", "v": 20})),
            (2, json!({"k": "a", "v": 10})),
        ],
        "each index receives its own element; equal values stay distinct"
    );
    assert_eq!(aggregate(&p, run).await, json!([10, 20, 10]));
    let (status, code) = status_and_code(&p, run).await;
    assert_eq!((status.as_str(), code), ("SUCCEEDED", None));

    let scope = sqlx::query(
        "SELECT item_count, source_blob_id, convert_from(payload.content, 'UTF8') AS snapshot \
         FROM iteration_scopes scope JOIN payloads payload \
           ON payload.tenant_id = scope.tenant_id AND payload.digest = scope.snapshot_digest \
         WHERE scope.run_id = $1",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("scope");
    assert_eq!(scope.get::<i32, _>("item_count"), 3);
    assert_eq!(
        scope.get::<Option<Uuid>, _>("source_blob_id"),
        Some(blob["id"].as_str().expect("id").parse().expect("uuid")),
        "the scope names the blob it was read from"
    );
    let snapshot: Value = serde_json::from_str(&scope.get::<String, _>("snapshot")).expect("json");
    assert!(
        snapshot["$collectionSource"]["blob"]["sha256"] == blob["sha256"]
            && snapshot["$collectionSource"]["count"] == 3,
        "the snapshot payload is the manifest of the read, not the array: {snapshot}"
    );
    let stored: Vec<String> = sqlx::query_scalar(
        "SELECT convert_from(payload.content, 'UTF8') FROM collection_scope_items item \
         JOIN payloads payload ON payload.tenant_id = item.tenant_id \
          AND payload.digest = item.item_digest \
         JOIN iteration_scopes scope ON scope.tenant_id = item.tenant_id \
          AND scope.run_id = item.run_id AND scope.scope_id = item.scope_id \
         WHERE scope.run_id = $1 ORDER BY item.iteration_index",
    )
    .bind(run)
    .fetch_all(&p.api.db.pool)
    .await
    .expect("stored items");
    assert_eq!(
        stored,
        vec![
            r#"{"k":"a","v":10}"#,
            r#"{"k":"b","v":20}"#,
            r#"{"k":"a","v":10}"#
        ],
        "items are stored as canonical compact JSON"
    );
    let reads: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 AND action = 'blob.read' \
           AND resource_id = $2",
    )
    .bind(p.tenant.as_uuid())
    .bind(blob["id"].as_str().expect("id"))
    .fetch_one(&p.api.db.pool)
    .await
    .expect("audit");
    assert_eq!(reads, 1, "the engine's read of the blob is audited once");
    p.api
        .executions
        .verify_run(p.tenant, sagat_domain::execution::RunId::new(run))
        .await
        .expect("journal and projection agree");
}

/// A step's committed output blob is a source of its own run and of no other. No published
/// workflow can put an activity before a collection yet (the public path starts at the collection
/// owner), so this drives the reader the scheduler uses, with a real committed step output.
#[tokio::test]
async fn a_step_output_blob_is_a_source_of_its_own_run_only() {
    let p = platform().await;
    let flow = |name: &str| {
        json!({
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": name}, "entrypoint": "fetch",
            "nodes": [{"id": "fetch", "type": "activity", "activity": "src.fetch"}]
        })
    };
    let workflow = p
        .publish("blob-source-step", &flow("blob-source-step"))
        .await;
    let run = start_inline(&p, &workflow, &json!({})).await;
    let other = start_inline(&p, &workflow, &json!({})).await;
    let mut worker = scoped_worker(&p, "blob-source-fetcher", &["src.fetch"]).await;
    let tasks = worker.poll(8).await;
    assert_eq!(tasks.len(), 2);
    let array = serde_json::to_vec(&json!([{"v": 1}, {"v": 2}, {"v": 3}, {"v": 4}])).expect("json");
    let mut produced = Vec::new();
    for task in &tasks {
        let written = worker
            .write_blob(
                task,
                sagat_api_grpc::pb::WriteBlobHeader {
                    content_type: "application/json".into(),
                    expected_size: array.len() as i64,
                    expected_sha256: sha(&array),
                    ..Default::default()
                },
                &array,
                64,
            )
            .await
            .expect("write the array as a blob");
        worker
            .complete_with_blob(task, &written.blob_id)
            .await
            .expect("complete with the blob");
        produced.push((task.run_id.parse::<Uuid>().expect("run"), written));
    }
    let (own_run, own_blob) = produced
        .iter()
        .find(|(id, _)| *id == run)
        .map(|(id, blob)| (*id, blob.clone()))
        .expect("the first run's blob");
    let other_blob = produced
        .iter()
        .find(|(id, _)| *id == other)
        .map(|(_, blob)| blob.clone())
        .expect("the second run's blob");

    let source = blob_source(&p);
    let access = BlobAccess {
        tenant_id: p.tenant,
        namespace_id: namespace_of(&p).await,
    };
    let request = |run: Uuid, blob: &sagat_api_grpc::pb::WriteBlobResponse, max_items| {
        sagat_application::collection_source::CollectionSourceRequest {
            access,
            run_id: sagat_domain::execution::RunId::new(run),
            run_input: None,
            descriptor: sagat_domain::blobs::BlobDescriptor::new(
                blob.blob_id.parse().expect("blob id"),
                u64::try_from(blob.size).expect("size"),
                sagat_domain::workflow::Digest::from_hex(&blob.sha256).expect("digest"),
                sagat_domain::blobs::ContentType::parse(&blob.content_type).expect("type"),
            ),
            max_items,
        }
    };
    let items = source
        .materialize(&request(own_run, &own_blob, 10))
        .await
        .expect("the run reads the output of its own step");
    assert_eq!(items.items.len(), 4);
    assert_eq!(items.items[3], br#"{"v":4}"#.to_vec());
    assert_eq!(
        source
            .materialize(&request(own_run, &other_blob, 10))
            .await
            .expect_err("the output of another run's step is not this run's"),
        sagat_application::collection_source::CollectionSourceError::NotFound
    );
    assert_eq!(
        source
            .materialize(&request(own_run, &own_blob, 3))
            .await
            .expect_err("more items than maxItems"),
        sagat_application::collection_source::CollectionSourceError::TooManyItems
    );
}

#[tokio::test]
async fn an_array_of_blob_references_larger_than_one_payload_is_materialized_and_delivered_as_references()
 {
    let p = platform().await;
    let workflow = p
        .publish(
            "blob-source-refs",
            &collection_ir("blob-source-refs", "input", 2000),
        )
        .await;
    // 1,500 references of about 190 bytes: ~290 KiB, over the 256 KiB a payload may hold.
    let references: Vec<Value> = (0..1500)
        .map(|n| {
            json!({"v": n, "$blob": {
                "id": Uuid::now_v7().to_string(), "size": 1_000_000 + n,
                "sha256": sha(format!("item-{n}").as_bytes()),
                "contentType": "application/pdf"}})
        })
        .collect();
    let bytes = serde_json::to_vec(&references).expect("json");
    assert!(
        bytes.len() > 256 * 1024,
        "the fixture must not fit one payload"
    );
    let blob = upload(&p, &bytes).await;
    let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    let mut worker = scoped_worker(&p, "blob-source-refs-worker", &["src.line"]).await;

    scheduler.tick().await;
    assert_eq!(scopes(&p, run).await, 1, "the array became a membership");
    let count: i32 =
        sqlx::query_scalar("SELECT item_count FROM iteration_scopes WHERE run_id = $1")
            .bind(run)
            .fetch_one(&p.api.db.pool)
            .await
            .expect("count");
    assert_eq!(count, 1500);
    let items: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collection_scope_items item JOIN iteration_scopes scope \
           ON scope.scope_id = item.scope_id WHERE scope.run_id = $1",
    )
    .bind(run)
    .fetch_one(&p.api.db.pool)
    .await
    .expect("items");
    assert_eq!(items, 1500);
    // Admit the first window and read what a worker is handed.
    for _ in 0..4 {
        scheduler.tick().await;
    }
    let tasks = worker.poll(8).await;
    assert!(!tasks.is_empty());
    for task in tasks {
        let input: Value = serde_json::from_str(&task.input_json).expect("task input");
        let index = task
            .instance
            .as_ref()
            .and_then(|i| i.instance_path.first())
            .map(|s| s.iteration_index)
            .expect("scoped");
        assert_eq!(
            input["item"], references[index as usize],
            "iteration {index} is handed its reference verbatim, never the bytes it points to"
        );
        assert!(task.input_json.len() < 4096, "the task context stays small");
    }
}

// ---------------------------------------------------------------------------------------------
// Immutability and garbage collection
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_membership_is_fixed_at_entry_and_the_blob_behind_it_is_protected() {
    let p = platform().await;
    let workflow = p
        .publish(
            "blob-source-pinned",
            &collection_ir("blob-source-pinned", "input", 10),
        )
        .await;
    let blob = upload(&p, br#"[{"v":1},{"v":2},{"v":3}]"#).await;
    let id = blob["id"].as_str().expect("id").to_owned();
    let run = start_with_blob(&p, &workflow, &id).await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    scheduler.tick().await;
    assert_eq!(scopes(&p, run).await, 1);

    // The row cannot be deleted while a scope refers to it (23503 on PostgreSQL 16, 23001 on 18).
    let deleted = sqlx::query("DELETE FROM blobs WHERE tenant_id = $1 AND id = $2")
        .bind(p.tenant.as_uuid())
        .bind(id.parse::<Uuid>().expect("uuid"))
        .execute(&p.api.db.pool)
        .await;
    let error = deleted.expect_err("a live snapshot keeps its blob");
    let code = error
        .as_database_error()
        .and_then(|e| e.code().map(|c| c.to_string()))
        .unwrap_or_default();
    assert!(
        code == "23503" || code == "23001",
        "foreign key violation, got {code}"
    );

    // The sweep, with every timer at zero, finds nothing to remove: the blob is committed.
    let sweeper = BlobService::new(
        Arc::new(PgBlobCatalog::new(p.api.db.pool.clone())),
        p.api.blob_backend.clone().expect("a backend"),
        BlobLimits {
            upload_ttl: Duration::ZERO,
            abandoned_after: Duration::ZERO,
            ..BlobLimits::default()
        },
    );
    assert_eq!(sweeper.sweep(100).await.expect("sweep"), 0);

    // The bytes may leave the store: membership was read once and is held in the database, so the
    // collection runs to the end on the same three items.
    let backend = p.api.blob_backend.clone().expect("a backend");
    backend
        .delete(&BlobKey::of(
            BlobAccess {
                tenant_id: p.tenant,
                namespace_id: namespace_of(&p).await,
            },
            id.parse::<BlobId>().expect("blob id"),
        ))
        .await
        .expect("remove the bytes");
    let mut worker = scoped_worker(&p, "blob-source-pinned-worker", &["src.line"]).await;
    let seen = run_to_the_end(&p, &scheduler, &mut worker, run).await;
    assert_eq!(seen.len(), 3);
    assert_eq!(aggregate(&p, run).await, json!([1, 2, 3]));
    assert_eq!(status_and_code(&p, run).await.0, "SUCCEEDED");
}

async fn namespace_of(p: &Platform) -> NamespaceId {
    NamespaceId::new(p.namespace_id().await.parse().expect("namespace id"))
}

// ---------------------------------------------------------------------------------------------
// Isolation: only the run's own, committed, same-tenant-and-namespace blob can be a source
// ---------------------------------------------------------------------------------------------

/// Publishes a one-activity workflow in `tenant` and returns its id.
async fn publish_as(p: &Platform, who: &str, tenant: &str, namespace: &str, name: &str) -> String {
    let created = p
        .api
        .as_user(Method::POST, "/workflows", who, tenant)
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", format!("create-{}", Uuid::now_v7()))
        .json(&json!({"name": name, "ir": {
            "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
            "metadata": {"name": name}, "entrypoint": "a",
            "nodes": [{"id": "a", "type": "activity", "activity": "act.a"}]}}))
        .send()
        .await
        .expect("create");
    let id = json_of(created).await["id"]
        .as_str()
        .expect("id")
        .to_owned();
    let published = p
        .api
        .as_user(
            Method::POST,
            &format!("/workflows/{id}/publish"),
            who,
            tenant,
        )
        .header("x-octopus-namespace", namespace)
        .header("idempotency-key", format!("publish-{}", Uuid::now_v7()))
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish");
    assert_eq!(published.status(), 201);
    id
}

#[tokio::test]
async fn a_descriptor_the_run_does_not_own_is_refused_with_blob_unavailable() {
    let p = platform().await;
    let workflow = p
        .publish(
            "blob-source-isolation",
            &collection_ir("blob-source-isolation", "input.items", 10),
        )
        .await;
    let array = br#"[{"v":1}]"#;

    // A committed blob of another tenant.
    let ws = p.api.create_workspace("bob", "globex", "other").await;
    p.api
        .create_namespace("bob", "globex", &ws, "payments", "dev")
        .await;
    let foreign = upload_as(&p, "bob", "globex", "payments", array).await;
    let foreign_flow = publish_as(&p, "bob", "globex", "payments", "foreign-flow").await;
    start_with_blob_as(
        &p,
        "bob",
        "globex",
        "payments",
        &foreign_flow,
        foreign["id"].as_str().expect("id"),
    )
    .await;

    // A committed blob of another namespace of the same tenant.
    let ws = p.api.create_workspace(HUMAN, TENANT, "elsewhere").await;
    p.api
        .create_namespace(HUMAN, TENANT, &ws, "billing", "dev")
        .await;
    let sibling = upload_as(&p, HUMAN, TENANT, "billing", array).await;
    let sibling_flow = publish_as(&p, HUMAN, TENANT, "billing", "sibling-flow").await;
    start_with_blob_as(
        &p,
        HUMAN,
        TENANT,
        "billing",
        &sibling_flow,
        sibling["id"].as_str().expect("id"),
    )
    .await;

    // A committed blob bound as the input of another run in the same namespace.
    let other_run = upload(&p, array).await;
    let owner_flow = publish_as(&p, HUMAN, TENANT, NAMESPACE, "owner-flow").await;
    start_with_blob(&p, &owner_flow, other_run["id"].as_str().expect("id")).await;

    // Uploaded for a run input but bound to no run.
    let unbound = upload(&p, array).await;

    // A descriptor whose size or digest disagrees with the catalog.
    let own = upload(&p, array).await;
    let mut wrong_digest = own.clone();
    wrong_digest["sha256"] = json!(sha(b"something else"));
    let mut wrong_size = own.clone();
    wrong_size["size"] = json!(own["size"].as_u64().expect("size") + 1);

    let cases = [
        ("another tenant", descriptor_of(&foreign)),
        ("another namespace", descriptor_of(&sibling)),
        ("another run's input", descriptor_of(&other_run)),
        ("an unbound upload", descriptor_of(&unbound)),
        (
            "an id that does not exist",
            descriptor_of(&json!({
                "id": Uuid::now_v7().to_string(), "size": 9, "sha256": sha(array),
                "contentType": "application/json"})),
        ),
        ("a different digest", descriptor_of(&wrong_digest)),
        ("a different size", descriptor_of(&wrong_size)),
    ];
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    for (label, descriptor) in cases {
        let run = start_inline(&p, &workflow, &json!({"items": descriptor})).await;
        tick_until_settled(&p, &scheduler, run, 6).await;
        let (status, code) = status_and_code(&p, run).await;
        assert_eq!(
            (status.as_str(), code.as_deref()),
            ("FAILED", Some("BLOB_UNAVAILABLE")),
            "{label}"
        );
        assert_eq!(scopes(&p, run).await, 0, "{label}: nothing was admitted");
    }
}

// ---------------------------------------------------------------------------------------------
// Typed failures: a blob that cannot be used fails the run, it does not crash or retry for ever
// ---------------------------------------------------------------------------------------------

async fn failing_run(
    p: &Platform,
    scheduler: &Sched,
    max_items: u32,
    bytes: &[u8],
) -> (Uuid, Value) {
    let name = format!("blob-source-fail-{}", Uuid::now_v7().simple());
    let workflow = p
        .publish(&name, &collection_ir(&name, "input", max_items))
        .await;
    let blob = upload(p, bytes).await;
    let run = start_with_blob(p, &workflow, blob["id"].as_str().expect("id")).await;
    tick_until_settled(p, scheduler, run, 6).await;
    (run, blob)
}

#[tokio::test]
async fn a_blob_that_is_not_a_usable_array_fails_the_run_with_the_matching_code() {
    let p = platform().await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    let big_item = format!("[{{\"v\":\"{}\"}}]", "x".repeat(300 * 1024));
    // 20 items of ~250 KiB are each under the payload ceiling and 5 MiB together over 4 MiB.
    let many_big = format!(
        "[{}]",
        (0..20)
            .map(|_| format!("{{\"v\":\"{}\"}}", "y".repeat(250 * 1024)))
            .collect::<Vec<_>>()
            .join(",")
    );
    let too_big_blob = format!("[\"{}\"]", " ".repeat(9 * MIB));
    let cases: Vec<(&str, Vec<u8>, u32, &str)> = vec![
        ("an object", br#"{"v":1}"#.to_vec(), 10, "EXPRESSION_ERROR"),
        ("text", b"not json at all".to_vec(), 10, "EXPRESSION_ERROR"),
        (
            "more items than maxItems",
            br#"[1,2,3,4,5]"#.to_vec(),
            3,
            "ITERATION_LIMIT",
        ),
        (
            "an item over 256 KiB",
            big_item.into_bytes(),
            10,
            "RUN_DATA_LIMIT",
        ),
        (
            "items over 4 MiB together",
            many_big.into_bytes(),
            100,
            "RUN_DATA_LIMIT",
        ),
        (
            "a blob over 8 MiB",
            too_big_blob.into_bytes(),
            10,
            "RUN_DATA_LIMIT",
        ),
    ];
    for (label, bytes, max_items, expected) in cases {
        let (run, _) = failing_run(&p, &scheduler, max_items, &bytes).await;
        let (status, code) = status_and_code(&p, run).await;
        assert_eq!(
            (status.as_str(), code.as_deref()),
            ("FAILED", Some(expected)),
            "{label}"
        );
        assert_eq!(scopes(&p, run).await, 0, "{label}: nothing was admitted");
    }
}

#[tokio::test]
async fn a_blob_whose_bytes_changed_or_vanished_fails_the_run_with_a_typed_code() {
    let p = platform().await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    let backend = p.api.blob_backend.clone().expect("a backend");
    let access = BlobAccess {
        tenant_id: p.tenant,
        namespace_id: namespace_of(&p).await,
    };
    let original = br#"[{"v":1},{"v":2}]"#;
    let workflow = p
        .publish(
            "blob-source-damaged",
            &collection_ir("blob-source-damaged", "input", 10),
        )
        .await;
    let mut cases = Vec::new();
    for label in ["same length, other bytes", "truncated", "extended", "gone"] {
        let blob = upload(&p, original).await;
        let id = blob["id"].as_str().expect("id").to_owned();
        let key = BlobKey::of(access, id.parse::<BlobId>().expect("blob id"));
        let replacement: Option<Vec<u8>> = match label {
            "same length, other bytes" => Some(br#"[{"v":9},{"v":9}]"#.to_vec()),
            "truncated" => Some(br#"[{"v":1}]"#.to_vec()),
            "extended" => Some(br#"[{"v":1},{"v":2},{"v":3}]"#.to_vec()),
            _ => None,
        };
        match replacement {
            Some(bytes) => backend
                .put(&key, byte_stream(vec![Ok(bytes.into())]))
                .await
                .expect("replace the bytes"),
            None => backend.delete(&key).await.expect("delete the bytes"),
        }
        let run = start_with_blob(&p, &workflow, &id).await;
        cases.push((label, run));
    }
    for (label, run) in cases {
        tick_until_settled(&p, &scheduler, run, 6).await;
        let (status, code) = status_and_code(&p, run).await;
        let expected = if label == "gone" {
            "BLOB_UNAVAILABLE"
        } else {
            "BLOB_CORRUPT"
        };
        assert_eq!(
            (status.as_str(), code.as_deref()),
            ("FAILED", Some(expected)),
            "{label}"
        );
        assert_eq!(scopes(&p, run).await, 0, "{label}: nothing was admitted");
    }
}

/// A backend that can be switched off, to look like an outage of the object store.
#[derive(Debug)]
struct Switchable {
    inner: Arc<dyn BlobBackend>,
    down: Arc<AtomicBool>,
}

impl BlobBackend for Switchable {
    fn provider(&self) -> &'static str {
        "switchable"
    }

    fn put<'a>(
        &'a self,
        key: &'a BlobKey,
        body: ByteStream,
    ) -> BoxFuture<'a, Result<(), BackendFailure>> {
        self.inner.put(key, body)
    }

    fn get<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<ByteStream, BackendFailure>> {
        Box::pin(async move {
            if self.down.load(Ordering::SeqCst) {
                Err(BackendFailure::Unavailable)
            } else {
                self.inner.get(key).await
            }
        })
    }

    fn delete<'a>(&'a self, key: &'a BlobKey) -> BoxFuture<'a, Result<(), BackendFailure>> {
        self.inner.delete(key)
    }
}

#[tokio::test]
async fn a_replica_without_a_blob_source_leaves_the_run_alone() {
    let p = platform().await;
    let workflow = p
        .publish(
            "blob-source-none",
            &collection_ir("blob-source-none", "input", 10),
        )
        .await;
    let blob = upload(&p, br#"[{"v":1},{"v":2}]"#).await;
    let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
    let scheduler = scheduler(&p, None).await;
    for _ in 0..2 {
        let report = scheduler.tick().await;
        assert_eq!(report.collections_advanced, 0, "{report:?}");
    }
    // It is never failed for being unable to read: another replica may.
    assert_eq!(status_and_code(&p, run).await, ("RUNNING".to_owned(), None));
    assert_eq!(scopes(&p, run).await, 0);
}

#[tokio::test]
async fn an_outage_of_the_object_store_leaves_the_run_waiting_not_failed() {
    let p = platform().await;
    let workflow = p
        .publish(
            "blob-source-outage",
            &collection_ir("blob-source-outage", "input", 10),
        )
        .await;
    let blob = upload(&p, br#"[{"v":1},{"v":2}]"#).await;
    let run = start_with_blob(&p, &workflow, blob["id"].as_str().expect("id")).await;
    let down = Arc::new(AtomicBool::new(true));
    let flaky = BlobService::new(
        Arc::new(PgBlobCatalog::new(p.api.db.pool.clone())),
        Arc::new(Switchable {
            inner: p.api.blob_backend.clone().expect("a backend"),
            down: down.clone(),
        }),
        BlobLimits::default(),
    );
    let scheduler = scheduler(&p, Some(Arc::new(flaky))).await;
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 1, "{report:?}");
    assert_eq!(status_and_code(&p, run).await, ("RUNNING".to_owned(), None));
    assert_eq!(scopes(&p, run).await, 0);

    // The store comes back: the collection is entered once and runs to the end.
    down.store(false, Ordering::SeqCst);
    let report = scheduler.tick().await;
    assert_eq!(report.failed, 0, "{report:?}");
    assert_eq!(scopes(&p, run).await, 1);
    let mut worker = scoped_worker(&p, "blob-source-outage-worker", &["src.line"]).await;
    run_to_the_end(&p, &scheduler, &mut worker, run).await;
    assert_eq!(aggregate(&p, run).await, json!([1, 2]));
}

#[tokio::test]
async fn an_inline_items_value_that_is_not_an_array_fails_the_run_instead_of_retrying_for_ever() {
    let p = platform().await;
    let workflow = p
        .publish(
            "blob-source-inline-bad",
            &collection_ir("blob-source-inline-bad", "input.items", 10),
        )
        .await;
    let scheduler = scheduler(&p, Some(blob_source(&p))).await;
    for (label, input) in [
        ("a number", json!({"items": 5})),
        ("an object", json!({"items": {"v": 1}})),
        ("a missing field", json!({})),
    ] {
        let run = start_inline(&p, &workflow, &input).await;
        tick_until_settled(&p, &scheduler, run, 6).await;
        let (status, code) = status_and_code(&p, run).await;
        assert_eq!(
            (status.as_str(), code.as_deref()),
            ("FAILED", Some("EXPRESSION_ERROR")),
            "{label}"
        );
    }
}
