//! Public sequential-loop publication, worker execution and restart evidence.
#![allow(clippy::expect_used)]

use std::time::Duration;

use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, PollTasksRequest, RegisterWorkerRequest,
    WorkerCapabilities,
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
    PublicLoop {
        api,
        run,
        tenant: TenantId::new(tenant.as_uuid()),
        worker,
        worker_id: registered.worker_id,
        session_token: registered.session_token,
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
