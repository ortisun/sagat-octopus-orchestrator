//! Public bounded-collection ordering and restart recovery against real PostgreSQL.
#![allow(clippy::expect_used, clippy::panic)]

use std::time::Duration;

use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, PollTasksRequest, RegisterWorkerRequest, WorkerCapabilities,
};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Membership, Role, Subject, TenantId};
use sagat_infra_ir::JsonEvaluator;
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use tonic::Request;
use tonic::metadata::MetadataValue;
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};

fn collection(name: &str, result: &str, max_concurrency: u16) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "lines",
        "bodies": {
            "line": {
                "entrypoint": "process",
                "nodes": [
                    {
                        "id": "process",
                        "type": "activity",
                        "activity": "line.process",
                        "next": "return-result"
                    },
                    {
                        "id": "return-result",
                        "type": "return",
                        "result": "steps.process.output"
                    }
                ]
            }
        },
        "nodes": [
            {
                "id": "lines",
                "type": "forEach",
                "items": "input.lines",
                "body": "line",
                "result": result,
                "maxItems": 17,
                "maxConcurrency": max_concurrency,
                "next": "done"
            },
            {"id": "done", "type": "end", "outputExpr": "steps.lines.output"}
        ]
    })
}

fn worker_request<T>(api: &TestApi, message: T) -> Request<T> {
    let token = api.idp.token("collection-worker", "acme").build();
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("worker authorization"),
    );
    request
}

fn scheduler(
    api: &TestApi,
    placement: Placement,
) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    let settings = SchedulerSettings {
        discovery_rest: Duration::ZERO,
        ..SchedulerSettings::default()
    };
    Scheduler::new(
        api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        settings,
    )
    .with_placement(placement)
}

async fn acquire_scheduler(api: &TestApi, label: &str) -> Placement {
    let placement = Placement::new(format!("collection-fault-{label}-{}", Uuid::now_v7()));
    Keeper::new(
        api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    placement
}

async fn active_count(api: &TestApi, run: RunId) -> i32 {
    sqlx::query_scalar(
        "SELECT active_count FROM iteration_scopes \
         WHERE run_id=$1 AND owner_node='lines' AND cardinality(owner_path_nodes)=0",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("collection active count")
}

async fn public_run() -> (TestApi, TenantId, String, RunId) {
    public_run_with("bodyResult", 17, 3).await
}

/// A published collection run over `count` positions, `max_concurrency` at a time, whose aggregate is the
/// `result` expression applied to each iteration.
async fn public_run_with(
    result: &str,
    count: u32,
    max_concurrency: u16,
) -> (TestApi, TenantId, String, RunId) {
    let api = support::api::start_with(support::api::Options {
        runtime_role: true,
        ..support::api::Options::default()
    })
    .await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "collections").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let name = format!("bounded-collection-{}", Uuid::now_v7());
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": collection(&name, result, max_concurrency)}))
        .send()
        .await
        .expect("create collection workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    let workflow = json_of(created).await["id"]
        .as_str()
        .expect("workflow id")
        .to_owned();
    let published = api
        .as_user(
            Method::POST,
            &format!("/workflows/{workflow}/publish"),
            "alice",
            "acme",
        )
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"draftRevision": 1}))
        .send()
        .await
        .expect("publish collection workflow");
    let published_status = published.status();
    let published_body = json_of(published).await;
    assert_eq!(
        published_status,
        StatusCode::CREATED,
        "publish collection: {published_body}"
    );
    let capacity = api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": max_concurrency}))
        .send()
        .await
        .expect("set iteration capacity");
    assert_eq!(capacity.status(), StatusCode::OK);
    let lines = (0..count)
        .map(|position| json!({"position": position}))
        .collect::<Vec<_>>();
    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": {"lines": lines}}))
        .send()
        .await
        .expect("start collection run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let run = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("valid run id");
    (api, tenant, namespace, run)
}

#[tokio::test]
async fn bounded_public_collection_finishes_in_input_order_after_restart_and_lost_hints() {
    let (api, tenant, namespace, run) = public_run().await;
    let initial_placement = acquire_scheduler(&api, "initial").await;
    let first_owner = initial_placement.owner().to_owned();
    let first = scheduler(&api, initial_placement);
    let entered = first.tick().await;
    assert_eq!(entered.collections_advanced, 1, "{entered:?}");
    assert_eq!(entered.failed, 0, "{entered:?}");

    let mut highest_active = 0;
    for admission in 0..3 {
        let report = first.tick().await;
        assert_eq!(
            report.collections_advanced, 1,
            "admission {admission}: {report:?}"
        );
        assert_eq!(report.failed, 0, "admission {admission}: {report:?}");
        let active = active_count(&api, run).await;
        highest_active = highest_active.max(active);
        assert!(active <= 3, "active collection iterations: {active}");
    }
    assert_eq!(highest_active, 3);

    let lost: u64 = sqlx::query(
        "UPDATE event_outbox SET delivered_at=clock_timestamp() \
         WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready' \
           AND routing_key='line.process' AND delivered_at IS NULL",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .execute(&api.db.pool)
    .await
    .expect("lose admitted task hints")
    .rows_affected();
    assert_eq!(lost, 3);
    sqlx::query(
        "UPDATE scheduler_shards SET expires_at=clock_timestamp()-interval '1 second' \
         WHERE expires_at>clock_timestamp()",
    )
    .execute(&api.db.pool)
    .await
    .expect("expire stopped scheduler leases");
    drop(first);

    let restarted_placement = Placement::new(first_owner);
    Keeper::new(
        api.executions.clone(),
        restarted_placement.clone(),
        KeeperSettings {
            ttl: Duration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("restore scheduler shards");
    let restarted = scheduler(&api, restarted_placement);
    let recovered = restarted.tick().await;
    assert_eq!(recovered.failed, 0, "{recovered:?}");
    let pending_hints: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM event_outbox \
         WHERE tenant_id=$1 AND aggregate_id=$2 AND event_type='task.ready' \
           AND routing_key='line.process' AND delivered_at IS NULL",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("lost task hints remain absent");
    assert_eq!(pending_hints, 0);

    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            Subject::parse("collection-worker").expect("worker subject"),
            Role::Worker,
            support::now(),
        ))
        .await
        .expect("worker membership");
    let binding = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/collection-worker"),
            "alice",
            "acme",
        )
        .json(&json!({"activities": ["line.process"]}))
        .send()
        .await
        .expect("worker binding");
    assert_eq!(binding.status(), StatusCode::OK);
    let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
        .await
        .expect("connect collection worker");
    let registration = worker
        .register_worker(worker_request(
            &api,
            RegisterWorkerRequest {
                worker_name: format!("bounded-collection-worker-{}", Uuid::now_v7()),
                namespace: "payments".into(),
                capabilities: Some(WorkerCapabilities {
                    languages: vec!["rust".into()],
                    activities: vec!["line.process".into()],
                    pools: Vec::new(),
                    max_inflight: 32,
                    protocol_version: "1.5".into(),
                }),
            },
        ))
        .await
        .expect("register collection worker")
        .into_inner();

    let mut completed = 0_usize;
    let mut completion_order = Vec::with_capacity(17);
    while completed < 17 {
        let mut tasks = worker
            .poll_tasks(worker_request(
                &api,
                PollTasksRequest {
                    worker_id: registration.worker_id.clone(),
                    session_token: registration.session_token.clone(),
                    max_tasks: 17,
                },
            ))
            .await
            .expect("poll admitted collection tasks")
            .into_inner()
            .tasks;
        let expected_wave = (17 - completed).min(3);
        assert_eq!(tasks.len(), expected_wave);
        tasks.sort_by_key(|task| {
            std::cmp::Reverse(
                task.instance
                    .as_ref()
                    .and_then(|instance| instance.instance_path.first())
                    .map_or(u32::MAX, |segment| segment.iteration_index),
            )
        });
        for task in &tasks {
            let index = task
                .instance
                .as_ref()
                .and_then(|instance| instance.instance_path.first())
                .map(|segment| segment.iteration_index)
                .expect("collection iteration index");
            let input: Value =
                serde_json::from_str(&task.input_json).expect("collection task input");
            assert_eq!(input["item"]["position"], index);
            completion_order.push(index);
            let response = worker
                .complete_step(worker_request(
                    &api,
                    CompleteStepRequest {
                        task_id: task.task_id.clone(),
                        run_id: task.run_id.clone(),
                        step_id: task.step_id.clone(),
                        attempt: task.attempt,
                        generation: task.generation,
                        lease_token: task.lease_token.clone(),
                        idempotency_key: format!("complete-collection-{index}"),
                        output_json: json!({"position": index}).to_string(),
                        output_blob_ref: String::new(),
                        output_sha256: String::new(),
                        traceparent: String::new(),
                        instance: task.instance.clone(),
                    },
                ))
                .await
                .expect("complete collection member")
                .into_inner();
            assert_eq!(response.step_status, "SUCCEEDED");
        }
        completed += tasks.len();

        for checkpoint in 0..tasks.len() {
            let report = restarted.tick().await;
            assert_eq!(
                report.collections_advanced, 1,
                "checkpoint {checkpoint}: {report:?}"
            );
            assert_eq!(report.failed, 0, "checkpoint {checkpoint}: {report:?}");
            let active = active_count(&api, run).await;
            highest_active = highest_active.max(active);
            assert!(active <= 3, "active collection iterations: {active}");
        }
        if completed < 17 {
            let admissions = (17 - completed).min(3);
            for admission in 0..admissions {
                let report = restarted.tick().await;
                assert_eq!(
                    report.collections_advanced, 1,
                    "replacement admission {admission}: {report:?}"
                );
                assert_eq!(
                    report.failed, 0,
                    "replacement admission {admission}: {report:?}"
                );
                let active = active_count(&api, run).await;
                highest_active = highest_active.max(active);
                assert!(active <= 3, "active collection iterations: {active}");
            }
        }
    }
    assert_eq!(highest_active, 3);
    for wave in completion_order.chunks(3) {
        assert!(wave.windows(2).all(|pair| pair[0] > pair[1]), "{wave:?}");
    }

    let finished = restarted.tick().await;
    assert_eq!(finished.collections_advanced, 1, "{finished:?}");
    assert_eq!(finished.failed, 0, "{finished:?}");
    let projection: (String, String, i32, i32, i32, i32, i64, i64, Vec<u8>) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,scope.item_count,scope.admitted_count, \
                scope.completed_count,scope.active_count, \
                (SELECT count(DISTINCT iteration.iteration_index) \
                   FROM collection_iterations iteration \
                  WHERE iteration.tenant_id=scope.tenant_id AND iteration.run_id=scope.run_id \
                    AND iteration.scope_id=scope.scope_id), \
                (SELECT count(DISTINCT iteration.entry_instance_id) \
                   FROM collection_iterations iteration \
                  WHERE iteration.tenant_id=scope.tenant_id AND iteration.run_id=scope.run_id \
                    AND iteration.scope_id=scope.scope_id),payload.content \
           FROM runs run JOIN iteration_scopes scope \
             ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id \
           JOIN payloads payload \
             ON payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest \
          WHERE run.tenant_id=$1 AND run.id=$2 AND scope.owner_node='lines'",
    )
    .bind(tenant.as_uuid())
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("terminal ordered collection");
    assert_eq!(
        (
            projection.0.as_str(),
            projection.1.as_str(),
            projection.2,
            projection.3,
            projection.4,
            projection.5,
            projection.6,
            projection.7,
        ),
        ("SUCCEEDED", "SUCCEEDED", 17, 17, 17, 0, 17, 17)
    );
    let aggregate: Value = serde_json::from_slice(&projection.8).expect("ordered aggregate JSON");
    assert_eq!(
        aggregate,
        Value::Array(
            (0..17)
                .map(|position| json!({"position": position}))
                .collect()
        )
    );
    api.executions
        .verify_run(tenant, run)
        .await
        .expect("collection journal and projection agree");
}

/// Serves every task of the run with a worker that answers `{"position": index}` and ticks the
/// scheduler between waves, until the run ends or `rounds` ran out.
async fn serve_until_finished(
    api: &TestApi,
    tenant: TenantId,
    namespace: &str,
    run: RunId,
    scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
    rounds: usize,
) -> String {
    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            Subject::parse("collection-worker").expect("worker subject"),
            Role::Worker,
            support::now(),
        ))
        .await
        .expect("worker membership");
    let binding = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/collection-worker"),
            "alice",
            "acme",
        )
        .json(&json!({"activities": ["line.process"]}))
        .send()
        .await
        .expect("worker binding");
    assert_eq!(binding.status(), StatusCode::OK);
    let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
        .await
        .expect("connect collection worker");
    let registration = worker
        .register_worker(worker_request(
            api,
            RegisterWorkerRequest {
                worker_name: format!("collection-result-worker-{}", Uuid::now_v7()),
                namespace: "payments".into(),
                capabilities: Some(WorkerCapabilities {
                    languages: vec!["rust".into()],
                    activities: vec!["line.process".into()],
                    pools: Vec::new(),
                    max_inflight: 8,
                    protocol_version: "1.5".into(),
                }),
            },
        ))
        .await
        .expect("register collection worker")
        .into_inner();
    let mut status = String::new();
    for _ in 0..rounds {
        scheduler.tick().await;
        let tasks = worker
            .poll_tasks(worker_request(
                api,
                PollTasksRequest {
                    worker_id: registration.worker_id.clone(),
                    session_token: registration.session_token.clone(),
                    max_tasks: 8,
                },
            ))
            .await
            .expect("poll collection tasks")
            .into_inner()
            .tasks;
        for task in &tasks {
            let index = task
                .instance
                .as_ref()
                .and_then(|instance| instance.instance_path.first())
                .map(|segment| segment.iteration_index)
                .expect("collection iteration index");
            worker
                .complete_step(worker_request(
                    api,
                    CompleteStepRequest {
                        task_id: task.task_id.clone(),
                        run_id: task.run_id.clone(),
                        step_id: task.step_id.clone(),
                        attempt: task.attempt,
                        generation: task.generation,
                        lease_token: task.lease_token.clone(),
                        idempotency_key: format!("complete-collection-{index}"),
                        output_json: json!({"position": index}).to_string(),
                        output_blob_ref: String::new(),
                        output_sha256: String::new(),
                        traceparent: String::new(),
                        instance: task.instance.clone(),
                    },
                ))
                .await
                .expect("complete collection member");
        }
        status = sqlx::query_scalar("SELECT status FROM runs WHERE tenant_id=$1 AND id=$2")
            .bind(tenant.as_uuid())
            .bind(run.as_uuid())
            .fetch_one(&api.db.pool)
            .await
            .expect("run status");
        if status == "SUCCEEDED" {
            break;
        }
    }
    status
}

#[tokio::test]
async fn a_result_expression_reads_the_item_and_index_of_every_iteration() {
    // The contract lets a collection's `result` read `bodyResult`, `item` and `index`; the iterations
    // have ended by the time the aggregate is built, so their inputs must come from the snapshot.
    for (result, expected) in [
        ("item.position", json!([0, 1, 2])),
        ("index", json!([0, 1, 2])),
        ("bodyResult.position", json!([0, 1, 2])),
    ] {
        let (api, tenant, namespace, run) = public_run_with(result, 3, 2).await;
        let placement = acquire_scheduler(&api, "result").await;
        let scheduler = scheduler(&api, placement);
        let status = serve_until_finished(&api, tenant, &namespace, run, &scheduler, 60).await;
        let stop: (Option<String>, Option<String>) =
            sqlx::query_as("SELECT stop_reason, stop_code FROM runs WHERE tenant_id=$1 AND id=$2")
                .bind(tenant.as_uuid())
                .bind(run.as_uuid())
                .fetch_one(&api.db.pool)
                .await
                .expect("stop evidence");
        assert_eq!(
            status, "SUCCEEDED",
            "result expression {result}: stopped with {stop:?}"
        );
        let aggregate: Vec<u8> = sqlx::query_scalar(
            "SELECT payload.content FROM iteration_scopes scope JOIN payloads payload \
               ON payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest \
              WHERE scope.tenant_id=$1 AND scope.run_id=$2 AND scope.owner_node='lines'",
        )
        .bind(tenant.as_uuid())
        .bind(run.as_uuid())
        .fetch_one(&api.db.pool)
        .await
        .expect("collection aggregate");
        let aggregate: Value = serde_json::from_slice(&aggregate).expect("aggregate JSON");
        assert_eq!(aggregate, expected, "result expression {result}");
    }
}
