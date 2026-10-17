//! Public sequential-loop publication, worker execution and restart evidence.
#![allow(clippy::expect_used)]

use std::time::Duration;

use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, HeartbeatRequest, PollTasksRequest,
    RegisterWorkerRequest, WorkerCapabilities,
};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_application::loop_progress::LoopProgressStore;
use sagat_application::maintenance::Window;
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

fn worker_request<T>(api: &TestApi, message: T) -> Request<T> {
    let token = api.idp.token("loop-worker", "acme").build();
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("worker authorization"),
    );
    request
}

fn loop_document(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "pre",
        "bodies": {
            "iteration": {
                "entrypoint": "increment",
                "nodes": [
                    {
                        "id": "increment",
                        "type": "activity",
                        "activity": "counter.increment",
                        "next": "return-value"
                    },
                    {
                        "id": "return-value",
                        "type": "return",
                        "result": "steps.increment.output"
                    }
                ]
            }
        },
        "nodes": [
            {
                "id": "pre",
                "type": "while",
                "condition": "carry < 1",
                "body": "iteration",
                "initial": "0",
                "carry": "bodyResult",
                "maxIterations": 3,
                "timeoutMs": 60000,
                "next": "post"
            },
            {
                "id": "post",
                "type": "repeatUntil",
                "condition": "carry >= 2",
                "body": "iteration",
                "initial": "1",
                "carry": "bodyResult",
                "maxIterations": 3,
                "timeoutMs": 60000,
                "next": "done"
            },
            {"id": "done", "type": "end", "outputExpr": "steps.post.output"}
        ]
    })
}

fn scheduler(
    api: &TestApi,
    placement: Placement,
) -> Scheduler<PgRunStore, JsonEvaluator, SystemEntropy> {
    Scheduler::new(
        api.executions.clone(),
        Engine::new(JsonEvaluator),
        SystemEntropy,
        SchedulerSettings {
            discovery_rest: Duration::ZERO,
            ..SchedulerSettings::default()
        },
    )
    .with_placement(placement)
}

async fn acquire_scheduler(api: &TestApi, label: &str) -> Placement {
    let placement = Placement::new(format!("public-loop-{label}-{}", Uuid::now_v7()));
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

async fn poll_and_complete(
    api: &TestApi,
    worker: &mut WorkerServiceClient<tonic::transport::Channel>,
    worker_id: &str,
    session_token: &str,
    expected_index: u32,
    output: i32,
) {
    let tasks = worker
        .poll_tasks(worker_request(
            api,
            PollTasksRequest {
                worker_id: worker_id.to_owned(),
                session_token: session_token.to_owned(),
                max_tasks: 1,
            },
        ))
        .await
        .expect("poll loop task")
        .into_inner()
        .tasks;
    assert_eq!(tasks.len(), 1);
    let task = &tasks[0];
    assert_eq!(task.activity_name, "counter.increment");
    let instance = task.instance.as_ref().expect("scoped loop task");
    assert_eq!(instance.instance_path.len(), 1);
    assert_eq!(instance.instance_path[0].iteration_index, expected_index);
    let completed = worker
        .complete_step(worker_request(
            api,
            CompleteStepRequest {
                task_id: task.task_id.clone(),
                run_id: task.run_id.clone(),
                step_id: task.step_id.clone(),
                attempt: task.attempt,
                generation: task.generation,
                lease_token: task.lease_token.clone(),
                idempotency_key: format!("complete-public-loop-{expected_index}-{output}"),
                output_json: output.to_string(),
                output_blob_ref: String::new(),
                output_sha256: String::new(),
                traceparent: String::new(),
                instance: task.instance.clone(),
            },
        ))
        .await
        .expect("complete loop task")
        .into_inner();
    assert_eq!(completed.step_status, "SUCCEEDED");
}

#[tokio::test]
async fn public_pre_and_post_test_loops_finish_after_restart_and_terminal_rollback() {
    let api = support::api::start_with(support::api::Options {
        runtime_role: true,
        ..support::api::Options::default()
    })
    .await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "loops").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let name = format!("public-loop-{}", Uuid::now_v7());
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": loop_document(&name)}))
        .send()
        .await
        .expect("create loop workflow");
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
        .expect("publish loop workflow");
    let published_status = published.status();
    let published_body = json_of(published).await;
    assert_eq!(
        published_status,
        StatusCode::CREATED,
        "publish loop: {published_body}"
    );
    let capacity = api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": 1}))
        .send()
        .await
        .expect("configure iteration capacity");
    assert_eq!(capacity.status(), StatusCode::OK);
    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": {}}))
        .send()
        .await
        .expect("start loop run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let run: RunId = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("valid run id");

    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            Subject::parse("loop-worker").expect("worker subject"),
            Role::Worker,
            support::now(),
        ))
        .await
        .expect("worker membership");
    let binding = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/loop-worker"),
            "alice",
            "acme",
        )
        .json(&json!({"activities": ["counter.increment"]}))
        .send()
        .await
        .expect("worker binding");
    assert_eq!(binding.status(), StatusCode::OK);
    let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
        .await
        .expect("connect loop worker");
    let registered = worker
        .register_worker(worker_request(
            &api,
            RegisterWorkerRequest {
                worker_name: format!("public-loop-worker-{name}"),
                namespace: "payments".into(),
                capabilities: Some(WorkerCapabilities {
                    languages: vec!["rust".into()],
                    activities: vec!["counter.increment".into()],
                    pools: Vec::new(),
                    max_inflight: 1,
                    protocol_version: "1.5".into(),
                }),
            },
        ))
        .await
        .expect("register loop worker")
        .into_inner();

    let first_placement = acquire_scheduler(&api, "first").await;
    let first_owner = first_placement.owner().to_owned();
    let first = scheduler(&api, first_placement);
    let entered = first.tick().await;
    assert_eq!(entered.loops_advanced, 1, "{entered:?}");
    assert_eq!(entered.failed, 0, "{entered:?}");
    poll_and_complete(
        &api,
        &mut worker,
        &registered.worker_id,
        &registered.session_token,
        0,
        1,
    )
    .await;
    let returned: (String, Option<String>, String) = sqlx::query_as(
        "SELECT iteration.lifecycle,iteration.return_node,returned.status \
         FROM loop_iterations iteration \
         JOIN loop_scopes scope ON scope.tenant_id=iteration.tenant_id \
           AND scope.run_id=iteration.run_id \
           AND scope.owner_instance_id=iteration.owner_instance_id \
         JOIN steps returned ON returned.tenant_id=iteration.tenant_id \
           AND returned.run_id=iteration.run_id \
           AND returned.instance_path_nodes=iteration.entry_path_nodes \
           AND returned.instance_path_indices=iteration.entry_path_indices \
           AND returned.node='return-value' \
         WHERE iteration.run_id=$1 AND scope.owner_node='pre'",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("returned loop member");
    assert_eq!(returned, ("ADMITTED".into(), None, "PENDING".into()));
    let candidates = api
        .executions
        .loop_candidates(&Window::everything(8))
        .await
        .expect("discover returned public loop");
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.step.run_id == run && candidate.step.node.as_str() == "pre"),
        "returned loop must remain discoverable before its checkpoint: {candidates:?}"
    );
    let pre_finished = first.tick().await;
    assert_eq!(pre_finished.loops_advanced, 1, "{pre_finished:?}");
    assert_eq!(pre_finished.failed, 0, "{pre_finished:?}");

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
    let post_entered = restarted.tick().await;
    assert_eq!(post_entered.loops_advanced, 1, "{post_entered:?}");
    assert_eq!(post_entered.failed, 0, "{post_entered:?}");
    let post_projection: (String, String, String) = sqlx::query_as(
        "SELECT iteration.lifecycle,entry.status,run.status \
         FROM loop_iterations iteration \
         JOIN loop_scopes scope ON scope.tenant_id=iteration.tenant_id \
           AND scope.run_id=iteration.run_id \
           AND scope.owner_instance_id=iteration.owner_instance_id \
         JOIN steps entry ON entry.tenant_id=iteration.tenant_id \
           AND entry.run_id=iteration.run_id \
           AND entry.instance_path_nodes=iteration.entry_path_nodes \
           AND entry.instance_path_indices=iteration.entry_path_indices \
           AND entry.node=iteration.entry_node \
         JOIN runs run ON run.tenant_id=iteration.tenant_id AND run.id=iteration.run_id \
         WHERE iteration.run_id=$1 AND scope.owner_node='post'",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("post loop projection");
    assert_eq!(
        post_projection,
        ("ADMITTED".into(), "READY".into(), "RUNNING".into()),
    );
    poll_and_complete(
        &api,
        &mut worker,
        &registered.worker_id,
        &registered.session_token,
        0,
        2,
    )
    .await;

    sqlx::raw_sql(
        "CREATE FUNCTION reject_public_loop_finish() RETURNS trigger LANGUAGE plpgsql AS $$ \
         BEGIN RAISE EXCEPTION 'injected terminal outbox failure'; END $$; \
         CREATE TRIGGER reject_public_loop_finish BEFORE INSERT ON event_outbox \
         FOR EACH ROW WHEN (NEW.event_type = 'run.finished') \
         EXECUTE FUNCTION reject_public_loop_finish()",
    )
    .execute(&api.db.pool)
    .await
    .expect("install loop terminal fault");
    let failed_finish = restarted.tick().await;
    assert_eq!(failed_finish.loops_advanced, 0, "{failed_finish:?}");
    assert_eq!(failed_finish.failed, 1, "{failed_finish:?}");
    let rollback: (String, String, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle, \
           (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
             AND event.run_id=run.id AND event.event_type='run.finished'), \
           (SELECT count(*) FROM steps terminal WHERE terminal.tenant_id=run.tenant_id \
             AND terminal.run_id=run.id AND terminal.node='done') \
         FROM runs run JOIN loop_scopes scope ON scope.tenant_id=run.tenant_id \
           AND scope.run_id=run.id AND scope.owner_node='post' WHERE run.id=$1",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("terminal rollback evidence");
    assert_eq!(rollback, ("RUNNING".into(), "RUNNING".into(), 0, 0));
    sqlx::raw_sql(
        "DROP TRIGGER reject_public_loop_finish ON event_outbox; \
         DROP FUNCTION reject_public_loop_finish()",
    )
    .execute(&api.db.pool)
    .await
    .expect("remove loop terminal fault");
    let finished = restarted.tick().await;
    assert_eq!(finished.loops_advanced, 1, "{finished:?}");
    assert_eq!(finished.failed, 0, "{finished:?}");
    let terminal: (String, String, String, bool, i64, i64) = sqlx::query_as(
        "SELECT run.status,scope.lifecycle,terminal.status,terminal.routed, \
           (SELECT count(*) FROM run_events event WHERE event.tenant_id=run.tenant_id \
             AND event.run_id=run.id AND event.event_type='run.finished'), \
           (SELECT count(*) FROM event_outbox message WHERE message.tenant_id=run.tenant_id \
             AND message.aggregate_id=run.id AND message.event_type='run.finished') \
         FROM runs run JOIN loop_scopes scope ON scope.tenant_id=run.tenant_id \
           AND scope.run_id=run.id AND scope.owner_node='post' \
         JOIN steps terminal ON terminal.tenant_id=run.tenant_id AND terminal.run_id=run.id \
           AND terminal.node='done' AND terminal.instance_path_nodes='{}' WHERE run.id=$1",
    )
    .bind(run.as_uuid())
    .fetch_one(&api.db.pool)
    .await
    .expect("terminal loop projection");
    assert_eq!(
        terminal,
        (
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            "SUCCEEDED".into(),
            true,
            1,
            1,
        )
    );
    api.executions
        .verify_run(TenantId::new(tenant.as_uuid()), run)
        .await
        .expect("terminal loop journal and projection");
}

#[tokio::test]
async fn publication_keeps_unproven_loop_bodies_closed() {
    let api = support::api::start().await;
    api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "loops").await;
    api.create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let name = format!("closed-loop-{}", Uuid::now_v7());
    let mut document = loop_document(&name);
    document["bodies"]["iteration"]["nodes"] = json!([
        {
            "id": "increment",
            "type": "activity",
            "activity": "counter.increment",
            "next": "return-value",
            "onError": "fallback"
        },
        {
            "id": "fallback",
            "type": "activity",
            "activity": "counter.fallback",
            "next": "return-value"
        },
        {"id": "return-value", "type": "return", "result": "steps.increment.output"}
    ]);
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": document}))
        .send()
        .await
        .expect("create unproven loop");
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
        .expect("reject unproven loop");
    assert_eq!(published.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        json_of(published).await["details"]["reason"],
        "UNSUPPORTED_NODE"
    );
}

/// A `while` loop whose body activity can fail. With `on_error` the loop routes every failure to an
/// `end` (D294); without it the failure fails the run. The public loop subset is a loop chain
/// ending at `end`, so an `end` is the error target publication accepts.
fn failing_loop_document(name: &str, on_error: bool, timeout_ms: u64) -> Value {
    let mut nodes = vec![
        json!({
            "id": "poll",
            "type": "while",
            "condition": "carry < 1",
            "body": "iteration",
            "initial": "0",
            "carry": "bodyResult",
            "maxIterations": 3,
            "timeoutMs": timeout_ms,
            "next": "done"
        }),
        json!({"id": "done", "type": "end", "outputExpr": "steps.poll.output"}),
    ];
    if on_error {
        nodes[0]["onError"] = json!("failed");
        nodes.push(json!({"id": "failed", "type": "end"}));
    }
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "poll",
        "bodies": {
            "iteration": {
                "entrypoint": "increment",
                "nodes": [
                    {
                        "id": "increment",
                        "type": "activity",
                        "activity": "counter.increment",
                        "next": "return-value"
                    },
                    {"id": "return-value", "type": "return", "result": "steps.increment.output"}
                ]
            }
        },
        "nodes": nodes
    })
}

/// Connects a worker that serves both activities the public loop documents use.
async fn register_loop_worker(
    api: &TestApi,
    name: &str,
) -> (
    WorkerServiceClient<tonic::transport::Channel>,
    String,
    String,
) {
    let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
        .await
        .expect("connect loop worker");
    let registered = worker
        .register_worker(worker_request(
            api,
            RegisterWorkerRequest {
                worker_name: format!("public-loop-worker-{name}-{}", Uuid::now_v7()),
                namespace: "payments".into(),
                capabilities: Some(WorkerCapabilities {
                    languages: vec!["rust".into()],
                    activities: vec!["counter.increment".into(), "recover.cleanup".into()],
                    pools: Vec::new(),
                    max_inflight: 1,
                    protocol_version: "1.5".into(),
                }),
            },
        ))
        .await
        .expect("register loop worker")
        .into_inner();
    (worker, registered.worker_id, registered.session_token)
}

struct PublicLoop {
    api: TestApi,
    run: RunId,
    tenant: TenantId,
    worker: WorkerServiceClient<tonic::transport::Channel>,
    worker_id: String,
    session_token: String,
}

/// Publishes `document`, starts a run and registers a worker for the counter activity.
async fn start_public_loop(document: impl Fn(&str) -> Value) -> PublicLoop {
    static LOGGING: std::sync::Once = std::sync::Once::new();
    LOGGING.call_once(|| {
        if std::env::var_os("RUST_LOG").is_some() {
            sagat_telemetry::init_logging(sagat_telemetry::LogFormat::Pretty);
        }
    });
    let api = support::api::start_with(support::api::Options {
        runtime_role: true,
        ..support::api::Options::default()
    })
    .await;
    let tenant = api.tenant("acme", "alice").await;
    let workspace = api.create_workspace("alice", "acme", "loops").await;
    let namespace = api
        .create_namespace("alice", "acme", &workspace, "payments", "dev")
        .await;
    let name = format!("public-loop-error-{}", Uuid::now_v7());
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": document(&name)}))
        .send()
        .await
        .expect("create loop workflow");
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
        .expect("publish loop workflow");
    let published_status = published.status();
    let published_body = json_of(published).await;
    assert_eq!(
        published_status,
        StatusCode::CREATED,
        "publish loop: {published_body}"
    );
    let capacity = api
        .as_user(
            Method::PUT,
            "/quotas/iteration_concurrency",
            "alice",
            "acme",
        )
        .json(&json!({"limit": 1}))
        .send()
        .await
        .expect("configure iteration capacity");
    assert_eq!(capacity.status(), StatusCode::OK);
    let started = api
        .as_user(Method::POST, "/runs", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"workflowId": workflow, "input": {}}))
        .send()
        .await
        .expect("start loop run");
    assert_eq!(started.status(), StatusCode::ACCEPTED);
    let run: RunId = json_of(started).await["runId"]
        .as_str()
        .expect("run id")
        .parse()
        .expect("valid run id");
    api.owner_store
        .insert_membership(&Membership::restore(
            tenant,
            Subject::parse("loop-worker").expect("worker subject"),
            Role::Worker,
            support::now(),
        ))
        .await
        .expect("worker membership");
    let binding = api
        .as_user(
            Method::PUT,
            &format!("/namespaces/{namespace}/worker-bindings/loop-worker"),
            "alice",
            "acme",
        )
        .json(&json!({"activities": ["counter.increment", "recover.cleanup"]}))
        .send()
        .await
        .expect("worker binding");
    assert_eq!(binding.status(), StatusCode::OK);
    let (worker, worker_id, session_token) = register_loop_worker(&api, &name).await;
    PublicLoop {
        api,
        run,
        tenant: TenantId::new(tenant.as_uuid()),
        worker,
        worker_id,
        session_token,
    }
}

impl PublicLoop {
    async fn poll_one(&mut self, activity: &str) -> sagat_api_grpc::pb::TaskLease {
        let tasks = self
            .worker
            .poll_tasks(worker_request(
                &self.api,
                PollTasksRequest {
                    worker_id: self.worker_id.clone(),
                    session_token: self.session_token.clone(),
                    max_tasks: 1,
                },
            ))
            .await
            .expect("poll loop task")
            .into_inner()
            .tasks;
        assert_eq!(tasks.len(), 1, "exactly one {activity} task");
        let task = tasks.into_iter().next().expect("one task");
        assert_eq!(task.activity_name, activity);
        task
    }

    async fn fail(&mut self, task: &sagat_api_grpc::pb::TaskLease) {
        let failed = self
            .worker
            .fail_step(worker_request(
                &self.api,
                FailStepRequest {
                    task_id: task.task_id.clone(),
                    run_id: task.run_id.clone(),
                    step_id: task.step_id.clone(),
                    attempt: task.attempt,
                    generation: task.generation,
                    lease_token: task.lease_token.clone(),
                    idempotency_key: format!("fail-public-loop-{}", Uuid::now_v7()),
                    error_code: "WORKER_ERROR".into(),
                    safe_message: String::new(),
                    retryable: false,
                    traceparent: String::new(),
                    instance: task.instance.clone(),
                },
            ))
            .await
            .expect("fail loop task")
            .into_inner();
        assert_eq!(failed.step_status, "FAILED");
    }

    async fn run_state(&self) -> (String, Option<String>, Option<String>) {
        sqlx::query_as("SELECT status,stop_reason,stop_code FROM runs WHERE id=$1")
            .bind(self.run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("run state")
    }

    async fn scope(&self) -> String {
        sqlx::query_scalar("SELECT lifecycle FROM loop_scopes WHERE run_id=$1")
            .bind(self.run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("loop scope")
    }

    async fn verify(&self, what: &str) {
        self.api
            .executions
            .verify_run(self.tenant, self.run)
            .await
            .unwrap_or_else(|error| panic!("{what}: {error}"));
    }
}

/// Ticks until the loop scope is `FAILED`, returning how many loop transitions that took. The
/// drain is three fenced transitions (start, iteration, finish); the run keeps running throughout
/// when the loop declares `onError`.
async fn drain_failed_loop(
    public: &PublicLoop,
    scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
    run_keeps_running: bool,
) -> u32 {
    let mut advanced = 0;
    for _ in 0..12 {
        if public.scope().await == "FAILED" {
            return advanced;
        }
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
        advanced += report.loops_advanced;
        if run_keeps_running {
            let (status, stop_reason, _) = public.run_state().await;
            assert_ne!(status, "CANCELLING", "the failure must not stop the run");
            assert_eq!(stop_reason, None);
        }
    }
    panic!("the loop never finished its drain");
}

#[tokio::test]
async fn public_loop_body_failure_routes_the_error_edge_and_the_run_completes() {
    let mut public = start_public_loop(|name| failing_loop_document(name, true, 60_000)).await;
    let placement = acquire_scheduler(&public.api, "error-edge").await;
    let scheduler = scheduler(&public.api, placement);
    let entered = scheduler.tick().await;
    assert_eq!(entered.loops_advanced, 1, "{entered:?}");
    let body = public.poll_one("counter.increment").await;
    public.fail(&body).await;

    // start, iteration and finish are three fenced transitions; the last one ends the run.
    assert_eq!(drain_failed_loop(&public, &scheduler, true).await, 3);
    let (status, stop_reason, stop_code) = public.run_state().await;
    assert_eq!(
        (status.as_str(), stop_reason, stop_code),
        ("SUCCEEDED", None, None)
    );
    let owner: (String, bool, Option<String>) = sqlx::query_as(
        "SELECT status,routed,failure_code FROM steps WHERE run_id=$1 AND node='poll'",
    )
    .bind(public.run.as_uuid())
    .fetch_one(&public.api.db.pool)
    .await
    .expect("loop owner");
    assert_eq!(owner, ("FAILED".into(), true, Some("WORKER_ERROR".into())));
    let finished: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type='run.finished'",
    )
    .bind(public.run.as_uuid())
    .fetch_one(&public.api.db.pool)
    .await
    .expect("finish events");
    assert_eq!(finished, 1);
    public.verify("run finished over the error edge").await;
}

#[tokio::test]
async fn public_loop_body_failure_without_an_error_edge_fails_the_run() {
    let mut public = start_public_loop(|name| failing_loop_document(name, false, 60_000)).await;
    let placement = acquire_scheduler(&public.api, "no-edge").await;
    let scheduler = scheduler(&public.api, placement);
    let entered = scheduler.tick().await;
    assert_eq!(entered.loops_advanced, 1, "{entered:?}");
    let body = public.poll_one("counter.increment").await;
    public.fail(&body).await;

    assert_eq!(drain_failed_loop(&public, &scheduler, false).await, 3);
    let (status, stop_reason, stop_code) = public.run_state().await;
    assert_eq!(
        (
            status.as_str(),
            stop_reason.as_deref(),
            stop_code.as_deref()
        ),
        ("FAILED", Some("FAILED"), Some("WORKER_ERROR"))
    );
    public.verify("run failed without an error edge").await;
}

#[tokio::test]
async fn public_loop_timeout_routes_the_error_edge_and_the_run_completes() {
    let public = start_public_loop(|name| failing_loop_document(name, true, 500)).await;
    let placement = acquire_scheduler(&public.api, "timeout").await;
    let scheduler = scheduler(&public.api, placement);
    let entered = scheduler.tick().await;
    assert_eq!(entered.loops_advanced, 1, "{entered:?}");
    // The loop was entered with a half-second budget: the next tick finds it expired.
    tokio::time::sleep(Duration::from_millis(700)).await;

    assert_eq!(drain_failed_loop(&public, &scheduler, true).await, 3);
    let owner: (String, Option<String>) =
        sqlx::query_as("SELECT status,failure_code FROM steps WHERE run_id=$1 AND node='poll'")
            .bind(public.run.as_uuid())
            .fetch_one(&public.api.db.pool)
            .await
            .expect("loop owner");
    assert_eq!(owner, ("FAILED".into(), Some("RUN_TIMED_OUT".into())));
    assert_eq!(public.run_state().await.0, "SUCCEEDED");
    public.verify("loop timed out over the error edge").await;
}

/// Where the loop's `onError` edge leads in a shaped document.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ErrorTarget {
    None,
    End,
    Activity,
}

/// A public loop with an optional delay between iterations and an optional error edge. The `while`
/// form keeps going while `carry < 1`, the `repeatUntil` form until `carry >= 1`, so an iteration
/// that returns `0` always asks for another one.
fn shaped_loop_document(
    name: &str,
    kind: &str,
    delay_ms: Option<u64>,
    timeout_ms: u64,
    error: ErrorTarget,
) -> Value {
    let mut owner = json!({
        "id": "poll",
        "type": kind,
        "condition": if kind == "while" { "carry < 1" } else { "carry >= 1" },
        "body": "iteration",
        "initial": "0",
        "carry": "bodyResult",
        "maxIterations": 3,
        "timeoutMs": timeout_ms,
        "next": "done"
    });
    if let Some(delay_ms) = delay_ms {
        owner["delayMs"] = json!(delay_ms);
    }
    let mut nodes = vec![json!({"id": "done", "type": "end", "outputExpr": "steps.poll.output"})];
    match error {
        ErrorTarget::None => {}
        ErrorTarget::End => {
            owner["onError"] = json!("failed");
            nodes.push(json!({"id": "failed", "type": "end"}));
        }
        ErrorTarget::Activity => {
            owner["onError"] = json!("recover");
            nodes.push(json!({
                "id": "recover",
                "type": "activity",
                "activity": "recover.cleanup",
                "next": "recovered"
            }));
            nodes.push(json!({"id": "recovered", "type": "end"}));
        }
    }
    nodes.insert(0, owner);
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "poll",
        "bodies": {
            "iteration": {
                "entrypoint": "increment",
                "nodes": [
                    {
                        "id": "increment",
                        "type": "activity",
                        "activity": "counter.increment",
                        "next": "return-value"
                    },
                    {"id": "return-value", "type": "return", "result": "steps.increment.output"}
                ]
            }
        },
        "nodes": nodes
    })
}

impl PublicLoop {
    /// `POST /runs/{id}/operations` with a plain whole-run action.
    async fn operate(&self, action: &str) -> (StatusCode, Value) {
        let response = self
            .api
            .as_user(
                Method::POST,
                &format!("/runs/{}/operations", self.run),
                "alice",
                "acme",
            )
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"action": action, "reason": "operator asked for it over HTTP"}))
            .send()
            .await
            .expect("operate the run");
        let status = response.status();
        (status, json_of(response).await)
    }

    async fn heartbeat(&mut self, task: &sagat_api_grpc::pb::TaskLease) -> bool {
        self.worker
            .heartbeat(worker_request(
                &self.api,
                HeartbeatRequest {
                    task_id: task.task_id.clone(),
                    worker_id: self.worker_id.clone(),
                    generation: task.generation,
                    lease_token: task.lease_token.clone(),
                    run_id: task.run_id.clone(),
                    step_id: task.step_id.clone(),
                    attempt: task.attempt,
                    instance: task.instance.clone(),
                },
            ))
            .await
            .expect("heartbeat the loop task")
            .into_inner()
            .cancel_requested
    }

    async fn complete(&mut self, task: &sagat_api_grpc::pb::TaskLease, output: &str) {
        let completed = self
            .worker
            .complete_step(worker_request(
                &self.api,
                CompleteStepRequest {
                    task_id: task.task_id.clone(),
                    run_id: task.run_id.clone(),
                    step_id: task.step_id.clone(),
                    attempt: task.attempt,
                    generation: task.generation,
                    lease_token: task.lease_token.clone(),
                    idempotency_key: format!("complete-public-loop-{}", Uuid::now_v7()),
                    output_json: output.to_owned(),
                    output_blob_ref: String::new(),
                    output_sha256: String::new(),
                    traceparent: String::new(),
                    instance: task.instance.clone(),
                },
            ))
            .await
            .expect("complete loop task")
            .into_inner();
        assert_eq!(completed.step_status, "SUCCEEDED");
    }

    /// How many tasks a poll finds right now.
    async fn poll_count(&mut self) -> usize {
        self.worker
            .poll_tasks(worker_request(
                &self.api,
                PollTasksRequest {
                    worker_id: self.worker_id.clone(),
                    session_token: self.session_token.clone(),
                    max_tasks: 1,
                },
            ))
            .await
            .expect("poll loop task")
            .into_inner()
            .tasks
            .len()
    }

    /// `(reserved instances of the run, iterations the tenant has active)`.
    async fn usage(&self) -> (i32, i32) {
        let reserved: i32 =
            sqlx::query_scalar("SELECT reserved_instances FROM run_instance_usage WHERE run_id=$1")
                .bind(self.run.as_uuid())
                .fetch_one(&self.api.db.pool)
                .await
                .expect("run instance usage");
        let active: i32 =
            sqlx::query_scalar("SELECT active_iterations FROM tenant_iteration_usage")
                .fetch_one(&self.api.db.pool)
                .await
                .expect("tenant iteration usage");
        (reserved, active)
    }

    async fn owner_status(&self) -> String {
        sqlx::query_scalar("SELECT status FROM steps WHERE run_id=$1 AND node='poll'")
            .bind(self.run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("loop owner")
    }

    /// Ticks the scheduler until the run has this status.
    async fn tick_until_run(
        &self,
        scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
        status: &str,
    ) {
        for _ in 0..12 {
            if self.run_state().await.0 == status {
                return;
            }
            let report = scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
        }
        panic!(
            "the run never reached {status}: {:?}",
            self.run_state().await
        );
    }
}

#[tokio::test]
async fn a_public_cancel_while_an_iteration_is_leased_ends_the_run_cancelled() {
    let mut public = start_public_loop(|name| {
        shaped_loop_document(name, "while", None, 60_000, ErrorTarget::None)
    })
    .await;
    let placement = acquire_scheduler(&public.api, "cancel-leased").await;
    let scheduler = scheduler(&public.api, placement);
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    let body = public.poll_one("counter.increment").await;
    let (reserved, active) = public.usage().await;
    assert!(
        reserved > 0 && active == 1,
        "the leased iteration holds its charge"
    );

    let (status, answer) = public.operate("CANCEL").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    assert_eq!(answer["runStatus"], "CANCELLING");
    let (run, reason, _) = public.run_state().await;
    assert_eq!(
        (run.as_str(), reason.as_deref()),
        ("CANCELLING", Some("CANCELLED"))
    );

    // The scheduler drains the loop under its own fence; the leased attempt is asked to stop and
    // the run cannot end while it still holds the lease.
    scheduler.tick().await;
    assert!(
        public.heartbeat(&body).await,
        "the leased attempt must be told to stop"
    );
    assert_eq!(public.run_state().await.0, "CANCELLING");
    let acknowledged = public
        .worker
        .fail_step(worker_request(
            &public.api,
            FailStepRequest {
                task_id: body.task_id.clone(),
                run_id: body.run_id.clone(),
                step_id: body.step_id.clone(),
                attempt: body.attempt,
                generation: body.generation,
                lease_token: body.lease_token.clone(),
                idempotency_key: format!("ack-cancel-{}", Uuid::now_v7()),
                error_code: "CANCELLED".into(),
                safe_message: String::new(),
                retryable: false,
                traceparent: String::new(),
                instance: body.instance.clone(),
            },
        ))
        .await
        .expect("the worker acknowledges the stop")
        .into_inner();
    assert_eq!(acknowledged.step_status, "CANCELLED");

    public.tick_until_run(&scheduler, "CANCELLED").await;
    assert_eq!(public.scope().await, "CANCELLED");
    assert_eq!(public.owner_status().await, "CANCELLED");
    assert_eq!(
        public.usage().await,
        (0, 0),
        "nothing leaks after the cancel"
    );
    public
        .verify("a leased iteration cancelled over HTTP")
        .await;
}

impl PublicLoop {
    /// Runs the first iteration to its checkpoint, which leaves the loop waiting on its delay timer.
    async fn reach_the_delay(
        &mut self,
        scheduler: &Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>,
    ) {
        assert_eq!(
            scheduler.tick().await.loops_advanced,
            1,
            "the loop is entered"
        );
        let body = self.poll_one("counter.increment").await;
        self.complete(&body, "0").await;
        for _ in 0..6 {
            if self.timer().await.as_deref() == Some("WAITING") {
                return;
            }
            let report = scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
        }
        panic!(
            "the loop never reached its delay: scope {}",
            self.scope().await
        );
    }

    async fn timer(&self) -> Option<String> {
        sqlx::query_scalar(
            "SELECT lifecycle FROM loop_timers WHERE run_id=$1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(self.run.as_uuid())
        .fetch_optional(&self.api.db.pool)
        .await
        .expect("loop timer")
    }
}

#[tokio::test]
async fn a_public_cancel_while_the_loop_waits_on_its_delay_ends_the_run_cancelled() {
    let mut public = start_public_loop(|name| {
        shaped_loop_document(name, "while", Some(60_000), 120_000, ErrorTarget::None)
    })
    .await;
    let placement = acquire_scheduler(&public.api, "cancel-delay").await;
    let scheduler = scheduler(&public.api, placement);
    public.reach_the_delay(&scheduler).await;
    // Only a durable timer holds the loop: no task is out for a worker to take.
    assert_eq!(public.poll_count().await, 0);

    let (status, answer) = public.operate("CANCEL").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    public.tick_until_run(&scheduler, "CANCELLED").await;
    assert_eq!(public.scope().await, "CANCELLED");
    assert_eq!(public.owner_status().await, "CANCELLED");
    assert_eq!(public.timer().await.as_deref(), Some("CANCELLED"));
    assert_eq!(
        public.usage().await,
        (0, 0),
        "nothing leaks after the cancel"
    );
    assert_eq!(
        public.poll_count().await,
        0,
        "no iteration was admitted after the cancel"
    );
    public.verify("a delaying loop cancelled over HTTP").await;
}

#[tokio::test]
async fn a_paused_run_holds_the_delay_and_resumes_the_loop() {
    let mut public = start_public_loop(|name| {
        shaped_loop_document(name, "while", Some(1_500), 120_000, ErrorTarget::None)
    })
    .await;
    let placement = acquire_scheduler(&public.api, "pause-delay").await;
    let scheduler = scheduler(&public.api, placement);
    public.reach_the_delay(&scheduler).await;

    let (status, answer) = public.operate("PAUSE").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    assert_eq!(answer["runStatus"], "PAUSED");
    // The delay passes while the run is paused: nothing may be admitted until it resumes.
    tokio::time::sleep(Duration::from_millis(1_800)).await;
    for _ in 0..3 {
        scheduler.tick().await;
    }
    assert_eq!(
        public.poll_count().await,
        0,
        "a paused run admits no iteration"
    );
    assert_eq!(public.timer().await.as_deref(), Some("WAITING"));

    let (status, answer) = public.operate("RESUME").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    for _ in 0..6 {
        scheduler.tick().await;
        if public.poll_count().await == 1 {
            public
                .verify("a resumed loop admits its next iteration")
                .await;
            return;
        }
    }
    panic!("the resumed loop never admitted its second iteration");
}

#[tokio::test]
async fn a_public_cancel_before_the_loop_is_entered_cancels_the_run_directly() {
    let public = start_public_loop(|name| {
        shaped_loop_document(name, "repeatUntil", None, 60_000, ErrorTarget::None)
    })
    .await;
    let (status, answer) = public.operate("CANCEL").await;
    assert_eq!(status, StatusCode::ACCEPTED, "{answer}");
    let placement = acquire_scheduler(&public.api, "cancel-pending").await;
    let scheduler = scheduler(&public.api, placement);
    public.tick_until_run(&scheduler, "CANCELLED").await;
    assert_eq!(public.owner_status().await, "CANCELLED");
    let scopes: i64 = sqlx::query_scalar("SELECT count(*) FROM loop_scopes WHERE run_id=$1")
        .bind(public.run.as_uuid())
        .fetch_one(&public.api.db.pool)
        .await
        .expect("loop scopes");
    assert_eq!(scopes, 0, "a cancelled loop is never entered");
    public
        .verify("a loop cancelled before it was entered")
        .await;
}

#[tokio::test]
async fn public_loop_body_failure_routes_the_error_edge_to_an_activity_that_completes_the_run() {
    let mut public = start_public_loop(|name| {
        shaped_loop_document(name, "while", None, 60_000, ErrorTarget::Activity)
    })
    .await;
    let placement = acquire_scheduler(&public.api, "error-activity").await;
    let scheduler = scheduler(&public.api, placement);
    assert_eq!(scheduler.tick().await.loops_advanced, 1);
    let body = public.poll_one("counter.increment").await;
    public.fail(&body).await;

    // The failure drains the active iteration first (D294); the run is not stopped by it.
    assert_eq!(drain_failed_loop(&public, &scheduler, true).await, 3);
    let (status, stop_reason, _) = public.run_state().await;
    assert_ne!(status, "SUCCEEDED", "the recovery activity has not run yet");
    assert_eq!(stop_reason, None);
    let owner: (String, bool, Option<String>) = sqlx::query_as(
        "SELECT status,routed,failure_code FROM steps WHERE run_id=$1 AND node='poll'",
    )
    .bind(public.run.as_uuid())
    .fetch_one(&public.api.db.pool)
    .await
    .expect("loop owner");
    assert_eq!(owner, ("FAILED".into(), true, Some("WORKER_ERROR".into())));

    // A worker takes the recovery activity like any root activity and completes it.
    let recovery = public.poll_one("recover.cleanup").await;
    assert!(
        recovery
            .instance
            .as_ref()
            .is_none_or(|i| i.instance_path.is_empty())
    );
    public.complete(&recovery, "{\"cleaned\":true}").await;
    public.tick_until_run(&scheduler, "SUCCEEDED").await;
    let recovered: (String, String) = sqlx::query_as(
        "SELECT (SELECT status FROM steps WHERE run_id=$1 AND node='recover'), \
                (SELECT status FROM steps WHERE run_id=$1 AND node='recovered')",
    )
    .bind(public.run.as_uuid())
    .fetch_one(&public.api.db.pool)
    .await
    .expect("recovery steps");
    assert_eq!(recovered, ("SUCCEEDED".into(), "SUCCEEDED".into()));
    let skipped: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM steps WHERE run_id=$1 AND node='done' AND status='SUCCEEDED'",
    )
    .bind(public.run.as_uuid())
    .fetch_one(&public.api.db.pool)
    .await
    .expect("success edge");
    assert_eq!(skipped, 0, "the success edge was not taken");
    assert_eq!(public.usage().await, (0, 0));
    public.verify("a loop recovered by an activity").await;
}

impl PublicLoop {
    /// The worker session that took the first iteration is gone: nothing polls, heartbeats or holds
    /// a lease from here on, until a new session is registered.
    fn worker_leaves(&mut self) {
        self.session_token.clear();
    }

    async fn worker_arrives(&mut self) {
        let (worker, worker_id, session_token) = register_loop_worker(&self.api, "late").await;
        self.worker = worker;
        self.worker_id = worker_id;
        self.session_token = session_token;
    }

    /// Steps of the run that hold or wait for a worker: leased, running or ready.
    async fn steps_needing_a_worker(&self) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM steps WHERE run_id=$1 AND status IN ('READY','LEASED','RUNNING')",
        )
        .bind(self.run.as_uuid())
        .fetch_one(&self.api.db.pool)
        .await
        .expect("steps needing a worker")
    }

    async fn event_count(&self, kind: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM run_events WHERE run_id=$1 AND event_type=$2")
            .bind(self.run.as_uuid())
            .bind(kind)
            .fetch_one(&self.api.db.pool)
            .await
            .expect("event count")
    }
}

/// The happy demo of the zero-worker delay: the loop waits between iterations on a durable timer
/// alone, the scheduler wakes it, and a worker that arrives only afterwards takes the next iteration.
async fn a_delay_between_iterations_needs_no_worker(kind: &str) {
    let mut public = start_public_loop(|name| {
        shaped_loop_document(name, kind, Some(3_000), 120_000, ErrorTarget::None)
    })
    .await;
    let placement = acquire_scheduler(&public.api, "zero-worker").await;
    let scheduler = scheduler(&public.api, placement);
    public.reach_the_delay(&scheduler).await;
    public.worker_leaves();

    // During the wait the engine holds a timer and nothing else: no lease, no ready task.
    assert_eq!(public.steps_needing_a_worker().await, 0);
    assert_eq!(public.timer().await.as_deref(), Some("WAITING"));
    assert_eq!(public.run_state().await.0, "WAITING");
    // Ticks before the delay elapsed change nothing.
    let before = scheduler.tick().await;
    assert_eq!(before.loops_advanced, 0, "{before:?}");
    assert_eq!(public.event_count("loop.delay_elapsed").await, 0);

    tokio::time::sleep(Duration::from_millis(3_200)).await;
    let woke = scheduler.tick().await;
    assert_eq!(woke.loops_advanced, 1, "{woke:?}");
    assert_eq!(woke.failed, 0, "{woke:?}");
    assert_eq!(public.event_count("loop.delay_elapsed").await, 1);
    assert_eq!(public.timer().await.as_deref(), Some("FIRED"));
    // The next iteration is ready for whoever comes: still no worker has been involved.
    assert_eq!(public.steps_needing_a_worker().await, 1);

    public.worker_arrives().await;
    let second = public.poll_one("counter.increment").await;
    assert_eq!(
        second.instance.as_ref().expect("scoped task").instance_path[0].iteration_index,
        1
    );
    public.complete(&second, "1").await;
    public.tick_until_run(&scheduler, "SUCCEEDED").await;
    assert_eq!(public.scope().await, "SUCCEEDED");
    assert_eq!(public.event_count("loop.delay_elapsed").await, 1);
    assert_eq!(public.usage().await, (0, 0));
    public.verify("a delayed loop finished").await;
}

#[tokio::test]
async fn a_while_loop_waits_out_its_delay_with_no_worker_connected() {
    a_delay_between_iterations_needs_no_worker("while").await;
}

#[tokio::test]
async fn a_repeat_until_loop_waits_out_its_delay_with_no_worker_connected() {
    a_delay_between_iterations_needs_no_worker("repeatUntil").await;
}

/// The failure demo: the loop's time budget runs out while its delay timer is waiting. No worker is
/// connected, so there is no iteration to drain; the timer is cancelled, the scope ends `FAILED`
/// with the timeout code and the run follows the `onError` edge. The delay fits the budget when it
/// is scheduled (5 s of a 9 s budget), and nothing ticks between the due time and the deadline.
#[tokio::test]
async fn a_loop_that_times_out_during_its_delay_follows_its_error_edge_with_no_worker() {
    let mut public = start_public_loop(|name| {
        shaped_loop_document(name, "while", Some(5_000), 9_000, ErrorTarget::End)
    })
    .await;
    let placement = acquire_scheduler(&public.api, "delay-timeout").await;
    let scheduler = scheduler(&public.api, placement);
    public.reach_the_delay(&scheduler).await;
    public.worker_leaves();
    assert_eq!(public.steps_needing_a_worker().await, 0);

    tokio::time::sleep(Duration::from_millis(9_300)).await;
    public.tick_until_run(&scheduler, "SUCCEEDED").await;
    assert_eq!(public.scope().await, "FAILED");
    assert_eq!(public.timer().await.as_deref(), Some("CANCELLED"));
    assert_eq!(public.event_count("loop.delay_elapsed").await, 0);
    let owner: (String, Option<String>) =
        sqlx::query_as("SELECT status,failure_code FROM steps WHERE run_id=$1 AND node='poll'")
            .bind(public.run.as_uuid())
            .fetch_one(&public.api.db.pool)
            .await
            .expect("loop owner");
    assert_eq!(owner, ("FAILED".into(), Some("RUN_TIMED_OUT".into())));
    assert_eq!(public.usage().await, (0, 0));
    public.verify("a loop timed out during its delay").await;
}

/// A delay that cannot end before the loop's budget does is never scheduled: the returned iteration
/// waits and the loop times out at its deadline (the early timeout the scheduler used to request
/// was refused by the store on every tick, which counted as a failed action).
#[tokio::test]
async fn a_delay_longer_than_the_loop_budget_waits_for_the_deadline_without_a_timer() {
    let mut public = start_public_loop(|name| {
        shaped_loop_document(name, "while", Some(30_000), 6_000, ErrorTarget::End)
    })
    .await;
    let placement = acquire_scheduler(&public.api, "delay-overrun").await;
    let scheduler = scheduler(&public.api, placement);
    assert_eq!(
        scheduler.tick().await.loops_advanced,
        1,
        "the loop is entered"
    );
    let body = public.poll_one("counter.increment").await;
    public.complete(&body, "0").await;
    public.worker_leaves();
    // The first tick returns the body; after that the loop only waits for its deadline.
    for _ in 0..3 {
        let report = scheduler.tick().await;
        assert_eq!(report.failed, 0, "{report:?}");
    }
    let waiting = scheduler.tick().await;
    assert_eq!(
        (waiting.failed, waiting.loops_advanced),
        (0, 0),
        "{waiting:?}"
    );
    assert_eq!(
        public.timer().await,
        None,
        "no delay is scheduled past the deadline"
    );
    assert_eq!(public.run_state().await.0, "RUNNING");

    tokio::time::sleep(Duration::from_millis(6_200)).await;
    public.tick_until_run(&scheduler, "SUCCEEDED").await;
    assert_eq!(public.scope().await, "FAILED");
    assert_eq!(public.timer().await, None);
    let owner: (String, Option<String>) =
        sqlx::query_as("SELECT status,failure_code FROM steps WHERE run_id=$1 AND node='poll'")
            .bind(public.run.as_uuid())
            .fetch_one(&public.api.db.pool)
            .await
            .expect("loop owner");
    assert_eq!(owner, ("FAILED".into(), Some("RUN_TIMED_OUT".into())));
    assert_eq!(public.usage().await, (0, 0));
    public.verify("a loop timed out before its delay").await;
}
