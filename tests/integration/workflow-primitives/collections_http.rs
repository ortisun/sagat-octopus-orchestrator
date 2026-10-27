//! Public publication and start boundaries for the proven collection shape.
#![allow(clippy::expect_used)]

use std::time::Duration as StdDuration;

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
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use tonic::Request;
use tonic::metadata::MetadataValue;
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};

fn worker_request<T>(api: &TestApi, message: T) -> Request<T> {
    let token = api.idp.token("collection-worker", "acme").build();
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("worker authorization"),
    );
    request
}

fn collection_document(name: &str, body_nodes: Value) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "lines",
        "bodies": {
            "line": {
                "entrypoint": "price",
                "nodes": body_nodes
            }
        },
        "nodes": [
            {
                "id": "lines",
                "type": "forEach",
                "items": "input.lines",
                "body": "line",
                "result": "bodyResult",
                "maxItems": 17,
                "maxConcurrency": 3,
                "next": "done"
            },
            {"id": "done", "type": "end", "outputExpr": "steps.lines.output"}
        ]
    })
}

fn proven_collection(name: &str) -> Value {
    let mut document = collection_document(
        name,
        json!([
            {
                "id": "price",
                "type": "activity",
                "activity": "line.price",
                "next": "return-price"
            },
            {"id": "return-price", "type": "return", "result": "steps.price.output"}
        ]),
    );
    document["bodies"]["tax"] = json!({
        "entrypoint": "price",
        "nodes": [
            {
                "id": "price",
                "type": "activity",
                "activity": "tax.price",
                "next": "return-price"
            },
            {"id": "return-price", "type": "return", "result": "steps.price.output"}
        ]
    });
    document["nodes"][0]["next"] = json!("taxes");
    document["nodes"]
        .as_array_mut()
        .expect("collection nodes")
        .insert(
            1,
            json!({
                "id": "taxes",
                "type": "forEach",
                "items": "input.lines",
                "body": "tax",
                "result": "bodyResult.tax",
                "maxItems": 17,
                "maxConcurrency": 3,
                "next": "done"
            }),
        );
    document
}

async fn create(api: &TestApi, name: &str, document: Value) -> String {
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": document}))
        .send()
        .await
        .expect("create workflow");
    assert_eq!(created.status(), StatusCode::CREATED);
    json_of(created).await["id"]
        .as_str()
        .expect("workflow id")
        .to_owned()
}

async fn fixture() -> (TestApi, TenantId, String) {
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
    (api, tenant, namespace)
}

#[tokio::test]
async fn proven_collection_can_be_published_and_started_over_http() {
    let (api, tenant, namespace) = fixture().await;
    let name = format!("public-collection-{}", Uuid::now_v7());
    let workflow = create(&api, &name, proven_collection(&name)).await;
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
        .expect("publish collection");
    let published_status = published.status();
    let published_body = json_of(published).await;
    assert_eq!(
        published_status,
        StatusCode::CREATED,
        "publish collection: {published_body}"
    );

    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({
            "workflowId": workflow,
            "input": {"lines": [{"sku": "a"}, {"sku": "b"}]}
        }))
        .send()
        .await
        .expect("start collection");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let run: RunId = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("run id");
    let projection: (String, String, i64) = sqlx::query_as(
        "SELECT run.status,step.status,run.run_version FROM runs run JOIN steps step \
         ON step.tenant_id=run.tenant_id AND step.run_id=run.id \
         WHERE run.id=$1 AND step.node='lines' AND step.instance_path_nodes='{}'",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("collection root projection");
    assert_eq!(projection, ("RUNNING".into(), "PENDING".into(), 1));

    let placement = Placement::new(format!("public-collection-scheduler-{name}"));
    Keeper::new(
        api.executions.clone(),
        placement.clone(),
        KeeperSettings {
            ttl: StdDuration::from_secs(60),
        },
    )
    .round()
    .await
    .expect("acquire scheduler shards");
    let scheduler = Scheduler::new(
        api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings::default(),
    )
    .with_placement(placement);
    let closed = scheduler.tick().await;
    assert_eq!(closed.collections_advanced, 0, "{closed:?}");
    assert_eq!(closed.failed, 1, "{closed:?}");
    let scopes_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM iteration_scopes WHERE run_id = $1")
            .bind(run.as_uuid())
            .fetch_one(&api.db.pool)
            .await
            .expect("collection scopes before policy");
    assert_eq!(scopes_before, 0);

    let capacity = api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": 2}))
        .send()
        .await
        .expect("configure iteration capacity");
    assert_eq!(capacity.status(), StatusCode::OK);
    let opened = scheduler.tick().await;
    assert_eq!(opened.collections_advanced, 1, "{opened:?}");
    assert_eq!(opened.failed, 0, "{opened:?}");
    let opened: (String, Option<String>, String, i32) = sqlx::query_as(
        "SELECT step.status,step.waiting_kind,scope.lifecycle,scope.item_count \
           FROM steps step JOIN iteration_scopes scope \
             ON scope.tenant_id=step.tenant_id AND scope.run_id=step.run_id \
            AND scope.owner_node=step.node \
          WHERE step.run_id=$1 AND step.node='lines' AND step.instance_path_nodes='{}'",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("opened collection projection");
    assert_eq!(
        opened,
        (
            "WAITING".into(),
            Some("COLLECTION".into()),
            "OPEN".into(),
            2
        )
    );

    for admission in 0..2 {
        let report = scheduler.tick().await;
        assert_eq!(
            report.collections_advanced, 1,
            "admission {admission}: {report:?}"
        );
        assert_eq!(report.failed, 0, "admission {admission}: {report:?}");
    }
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
        .json(&json!({"activities": ["line.price", "tax.price"]}))
        .send()
        .await
        .expect("worker binding");
    assert_eq!(binding.status(), StatusCode::OK);
    let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
        .await
        .expect("connect scoped worker");
    let registered = worker
        .register_worker(worker_request(
            &api,
            RegisterWorkerRequest {
                worker_name: format!("public-collection-worker-{name}"),
                namespace: "payments".into(),
                capabilities: Some(WorkerCapabilities {
                    languages: vec!["rust".into()],
                    activities: vec!["line.price".into(), "tax.price".into()],
                    pools: Vec::new(),
                    max_inflight: 8,
                    protocol_version: "1.5".into(),
                }),
            },
        ))
        .await
        .expect("register scoped worker")
        .into_inner();
    assert_eq!(registered.accepted_protocol_version, "1.5");
    let mut tasks = worker
        .poll_tasks(worker_request(
            &api,
            PollTasksRequest {
                worker_id: registered.worker_id.clone(),
                session_token: registered.session_token.clone(),
                max_tasks: 8,
            },
        ))
        .await
        .expect("poll scoped collection tasks")
        .into_inner()
        .tasks;
    tasks.sort_by_key(|task| {
        task.instance
            .as_ref()
            .and_then(|instance| instance.instance_path.first())
            .map_or(u32::MAX, |segment| segment.iteration_index)
    });
    assert_eq!(tasks.len(), 2);
    for (index, task) in tasks.iter().enumerate() {
        assert_eq!(task.activity_name, "line.price");
        let input: Value = serde_json::from_str(&task.input_json).expect("scoped task input");
        assert_eq!(input["item"]["sku"], if index == 0 { "a" } else { "b" });
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
                    idempotency_key: format!("complete-public-collection-{index}"),
                    output_json: json!({"amount": index + 1}).to_string(),
                    output_blob_ref: String::new(),
                    output_sha256: String::new(),
                    traceparent: String::new(),
                    instance: task.instance.clone(),
                },
            ))
            .await
            .expect("complete scoped collection task")
            .into_inner();
        assert_eq!(response.step_status, "SUCCEEDED");
    }

    for transition in [
        "first collection checkpoint",
        "second collection checkpoint",
    ] {
        let report = scheduler.tick().await;
        assert_eq!(report.collections_advanced, 1, "{transition}: {report:?}");
        assert_eq!(report.failed, 0, "{transition}: {report:?}");
    }
    let first_finish = scheduler.tick().await;
    assert_eq!(first_finish.collections_advanced, 1, "{first_finish:?}");
    assert_eq!(first_finish.failed, 0, "{first_finish:?}");
    for transition in [
        "second collection open",
        "first tax admission",
        "second tax admission",
    ] {
        let report = scheduler.tick().await;
        assert_eq!(report.collections_advanced, 1, "{transition}: {report:?}");
        assert_eq!(report.failed, 0, "{transition}: {report:?}");
    }
    let mut tax_tasks = worker
        .poll_tasks(worker_request(
            &api,
            PollTasksRequest {
                worker_id: registered.worker_id.clone(),
                session_token: registered.session_token.clone(),
                max_tasks: 8,
            },
        ))
        .await
        .expect("poll second collection tasks")
        .into_inner()
        .tasks;
    tax_tasks.sort_by_key(|task| {
        task.instance
            .as_ref()
            .and_then(|instance| instance.instance_path.first())
            .map_or(u32::MAX, |segment| segment.iteration_index)
    });
    assert_eq!(tax_tasks.len(), 2);
    for (index, task) in tax_tasks.iter().enumerate() {
        assert_eq!(task.activity_name, "tax.price");
        let input: Value = serde_json::from_str(&task.input_json).expect("tax task input");
        assert_eq!(input["item"]["sku"], if index == 0 { "a" } else { "b" });
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
                    idempotency_key: format!("complete-public-tax-{index}"),
                    output_json: json!({"tax": index + 1}).to_string(),
                    output_blob_ref: String::new(),
                    output_sha256: String::new(),
                    traceparent: String::new(),
                    instance: task.instance.clone(),
                },
            ))
            .await
            .expect("complete second collection task")
            .into_inner();
        assert_eq!(response.step_status, "SUCCEEDED");
    }
    for transition in ["first tax checkpoint", "second tax checkpoint"] {
        let report = scheduler.tick().await;
        assert_eq!(report.collections_advanced, 1, "{transition}: {report:?}");
        assert_eq!(report.failed, 0, "{transition}: {report:?}");
    }
    sqlx::raw_sql(
        "CREATE FUNCTION reject_public_collection_finish() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected terminal outbox failure'; END $$; \
         CREATE TRIGGER reject_public_collection_finish BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type = 'run.finished') \
         EXECUTE FUNCTION reject_public_collection_finish()",
    )
    .execute(&api.db.pool)
    .await
    .expect("install terminal fault");
    let failed_finish = scheduler.tick().await;
    assert_eq!(failed_finish.collections_advanced, 0, "{failed_finish:?}");
    assert_eq!(failed_finish.failed, 1, "{failed_finish:?}");
    let rollback: (String, String, i64, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle, \
           (SELECT count(*) FROM steps terminal WHERE terminal.tenant_id=run.tenant_id \
             AND terminal.run_id=run.id AND terminal.node='done'), \
           (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
             AND event.run_id=run.id AND event.event_type='collection.finished' \
             AND event.node='taxes'), \
           (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
             AND event.run_id=run.id AND event.event_type='run.finished') \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id AND scope.owner_node='taxes' \
         WHERE run.id=$1",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("terminal rollback evidence");
    assert_eq!(rollback, ("WAITING".into(), "OPEN".into(), 0, 0, 0));
    sqlx::raw_sql(
        "DROP TRIGGER reject_public_collection_finish ON event_outbox; \
         DROP FUNCTION reject_public_collection_finish()",
    )
    .execute(&api.db.pool)
    .await
    .expect("remove terminal fault");
    let finish = scheduler.tick().await;
    assert_eq!(finish.collections_advanced, 1, "finish: {finish:?}");
    assert_eq!(finish.failed, 0, "finish: {finish:?}");
    let aggregate: Value = sqlx::query_scalar(
        "SELECT convert_from(payload.content, 'UTF8')::jsonb \
         FROM iteration_scopes scope JOIN payloads payload \
           ON payload.tenant_id=scope.tenant_id AND payload.digest=scope.result_digest \
         WHERE scope.run_id=$1 AND scope.owner_node='taxes' \
           AND cardinality(scope.owner_path_nodes)=0",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("mapped collection result");
    assert_eq!(aggregate, json!([1, 2]));
    let terminal: (String, String, String, bool, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,terminal.status,terminal.routed, \
           (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
             AND event.run_id=run.id AND event.event_type='run.finished'), \
           (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=run.tenant_id \
             AND message.aggregate_id=run.id AND message.event_type='run.finished') \
         FROM runs run JOIN iteration_scopes scope \
           ON scope.tenant_id=run.tenant_id AND scope.run_id=run.id AND scope.owner_node='taxes' \
         JOIN steps terminal ON terminal.tenant_id=run.tenant_id AND terminal.run_id=run.id \
          AND terminal.node='done' AND terminal.instance_path_nodes='{}' \
         WHERE run.id=$1",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("terminal collection projection");
    assert_eq!(
        terminal,
        (
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            true,
            1,
            1
        )
    );
    api.executions
        .verify_run(tenant, run)
        .await
        .expect("terminal collection journal and projection");
}

#[tokio::test]
async fn publication_keeps_unproven_collection_bodies_closed() {
    let (api, _, _) = fixture().await;
    let name = format!("closed-collection-{}", Uuid::now_v7());
    let document = collection_document(
        &name,
        json!([
            // Return-only entry retains its separate admission/budget gate.
            {"id":"price","type":"return","result":"item"}
        ]),
    );
    let workflow = create(&api, &name, document).await;
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
        .expect("reject unproven collection");
    assert_eq!(published.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_of(published).await["details"]["reason"],
        "UNSUPPORTED_NODE"
    );
}

#[tokio::test]
async fn iteration_capacity_is_tenant_scoped_authorized_audited_and_reducible() {
    let (api, tenant, namespace) = fixture().await;
    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            Subject::parse("dev").expect("developer subject"),
            Role::Developer,
            support::now(),
        ))
        .await
        .expect("developer membership");

    let denied = api
        .as_user(Method::PUT, "/quotas/iteration_concurrency", "dev", "acme")
        .json(&json!({"limit": 3}))
        .send()
        .await
        .expect("denied quota change");
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    let wrong_scope = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/quotas/iteration_concurrency"),
            "alice",
            "acme",
        )
        .json(&json!({"limit": 3}))
        .send()
        .await
        .expect("reject namespace iteration capacity");
    assert_eq!(wrong_scope.status(), StatusCode::UNPROCESSABLE_ENTITY);

    for value in [1_025_u64, 1_000_000_001] {
        let rejected = api
            .as_user(
                Method::PUT,
                "/quotas/iteration_concurrency",
                "alice",
                "acme",
            )
            .json(&json!({"limit": value}))
            .send()
            .await
            .expect("reject capacity above the operator ceiling or the hard maximum");
        assert_eq!(
            rejected.status(),
            StatusCode::UNPROCESSABLE_ENTITY,
            "value={value}"
        );
    }

    let initial = json_of(api.get_as("/quotas", "alice", "acme").await).await;
    let initial = initial["items"]
        .as_array()
        .expect("quota items")
        .iter()
        .find(|quota| quota["resource"] == "iteration_concurrency")
        .expect("iteration capacity");
    assert_eq!(initial["limit"], 0);
    assert_eq!(initial["source"], "default");
    assert_eq!(initial["ceiling"], 1_024);
    assert_eq!(initial["used"], 0);

    let configured = api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": 3}))
        .send()
        .await
        .expect("configure iteration capacity");
    assert_eq!(configured.status(), StatusCode::OK);
    let configured = json_of(configured).await;
    assert_eq!(configured["limit"], 3);
    assert_eq!(configured["source"], "tenant");

    sqlx::query("UPDATE tenant_iteration_usage SET active_iterations = 2 WHERE tenant_id = $1")
        .bind(tenant.as_uuid())
        .execute(&api.db.pool)
        .await
        .expect("represent active iterations");
    let reduced = api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": 1}))
        .send()
        .await
        .expect("reduce iteration capacity");
    assert_eq!(reduced.status(), StatusCode::OK);
    let reduced = json_of(reduced).await;
    assert_eq!(reduced["limit"], 1);
    assert_eq!(reduced["used"], 2);

    let stored: (i32, String) = sqlx::query_as(
        "SELECT max_active_iterations, updated_by FROM tenant_collection_policies \
         WHERE tenant_id = $1",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("stored iteration capacity");
    assert_eq!(stored, (1, "alice".into()));
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE tenant_id = $1 \
         AND action = 'quota.changed' AND reason LIKE 'iteration_concurrency = %'",
    )
    .bind(tenant.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("capacity audits");
    assert_eq!(audits, 2);
}
