//! Compensation of the iterations a collection completed, through the public path: published over
//! HTTP, run by the scheduler, undone by workers over the worker protocol 1.5 (FR-014-005,
//! FR-003-002, D380..D389).
#![allow(clippy::expect_used)]

use std::time::Duration as StdDuration;

use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, PollTasksRequest, RegisterWorkerRequest, TaskLease,
    WorkerCapabilities,
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
use tonic::transport::Channel;
use uuid::Uuid;

use crate::support;
use crate::support::api::{TestApi, json_of};

type Sched = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

const ACTIVITIES: [&str; 6] = [
    "line.reserve",
    "line.charge",
    "line.notify",
    "line.release",
    "line.refund",
    "line.lookup",
];

fn retry() -> Value {
    json!({
        "maxAttempts": 2, "initialDelayMs": 100, "maxDelayMs": 100,
        "strategy": "fixed", "jitter": false
    })
}

fn activity(id: &str, name: &str, next: &str, extra: Value) -> Value {
    let mut node = json!({"id": id, "type": "activity", "activity": name, "next": next});
    for (key, value) in extra.as_object().expect("extra fields") {
        node[key] = value.clone();
    }
    node
}

pub(crate) fn collection_document(name: &str, nodes: Vec<Value>, concurrency: u32) -> Value {
    let entry = nodes[0]["id"].clone();
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "lines",
        "bodies": {"line": {"entrypoint": entry, "nodes": nodes}},
        "nodes": [
            {
                "id": "lines", "type": "forEach", "items": "input.lines", "body": "line",
                "result": "bodyResult", "maxItems": 8, "maxConcurrency": concurrency,
                "next": "done"
            },
            {"id": "done", "type": "end"}
        ]
    })
}

/// `reserve -> charge`, each undone by its own activity.
fn reserve_and_charge(name: &str, concurrency: u32) -> Value {
    collection_document(
        name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "charge",
                json!({"compensateWith": "line.release", "compensationRetry": retry()}),
            ),
            activity(
                "charge",
                "line.charge",
                "returned",
                json!({"compensateWith": "line.refund", "compensationRetry": retry()}),
            ),
            json!({"id": "returned", "type": "return", "result": "steps.charge.output"}),
        ],
        concurrency,
    )
}

pub(crate) struct Flow {
    pub(crate) api: TestApi,
    pub(crate) tenant: TenantId,
    pub(crate) run: RunId,
    pub(crate) scheduler: Sched,
    worker: WorkerServiceClient<Channel>,
    worker_id: String,
    session_token: String,
    logs: (tracing::subscriber::DefaultGuard, support::logs::Captured),
}

fn worker_request<T>(api: &TestApi, message: T) -> Request<T> {
    let token = api.idp.token("scope-worker", "acme").build();
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("worker authorization"),
    );
    request
}

pub(crate) async fn publish(api: &TestApi, name: &str, document: Value) -> Result<String, Value> {
    let created = api
        .as_user(Method::POST, "/workflows", "alice", "acme")
        .header("x-octopus-namespace", "payments")
        .header("idempotency-key", Uuid::now_v7().to_string())
        .json(&json!({"name": name, "ir": document}))
        .send()
        .await
        .expect("create workflow");
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
        .expect("publish workflow");
    if published.status() == StatusCode::CREATED {
        Ok(workflow)
    } else {
        Err(json_of(published).await)
    }
}

impl Flow {
    pub(crate) async fn start(name: &str, document: Value, lines: Value) -> Self {
        let api = support::api::start_with(support::api::Options {
            runtime_role: true,
            ..support::api::Options::default()
        })
        .await;
        let tenant = api.tenant("acme", "alice").await;
        let workspace = api.create_workspace("alice", "acme", "compensation").await;
        let namespace = api
            .create_namespace("alice", "acme", &workspace, "payments", "dev")
            .await;
        let workflow = publish(&api, name, document)
            .await
            .expect("the compensating collection publishes");
        let started = api
            .as_user(Method::POST, "/runs", "alice", "acme")
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"workflowId": workflow, "input": {"lines": lines}}))
            .send()
            .await
            .expect("start run");
        assert_eq!(started.status(), StatusCode::ACCEPTED);
        let run: RunId = json_of(started).await["runId"]
            .as_str()
            .expect("run id")
            .parse()
            .expect("run id");
        let capacity = api
            .as_user(
                Method::PUT,
                "/quotas/iteration_concurrency",
                "alice",
                "acme",
            )
            .json(&json!({"limit": 4}))
            .send()
            .await
            .expect("configure iteration capacity");
        assert_eq!(capacity.status(), StatusCode::OK);

        let placement = Placement::new(format!("scope-compensation-{name}"));
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

        api.owner_store
            .insert_membership(&Membership::restore(
                tenant,
                Subject::parse("scope-worker").expect("worker subject"),
                Role::Worker,
                support::now(),
            ))
            .await
            .expect("worker membership");
        let binding = api
            .as_user(
                Method::PUT,
                &format!("/namespaces/{namespace}/worker-bindings/scope-worker"),
                "alice",
                "acme",
            )
            .json(&json!({"activities": ACTIVITIES}))
            .send()
            .await
            .expect("worker binding");
        assert_eq!(binding.status(), StatusCode::OK);
        let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
            .await
            .expect("connect worker");
        let registered = worker
            .register_worker(worker_request(
                &api,
                RegisterWorkerRequest {
                    worker_name: format!("scope-worker-{name}"),
                    namespace: "payments".into(),
                    capabilities: Some(WorkerCapabilities {
                        languages: vec!["rust".into()],
                        activities: ACTIVITIES.iter().map(|a| (*a).to_owned()).collect(),
                        pools: Vec::new(),
                        max_inflight: 16,
                        protocol_version: "1.5".into(),
                    }),
                },
            ))
            .await
            .expect("register worker")
            .into_inner();
        assert_eq!(registered.accepted_protocol_version, "1.5");
        Self {
            api,
            tenant,
            run,
            scheduler,
            worker,
            worker_id: registered.worker_id,
            session_token: registered.session_token,
            logs: support::logs::capture_at(tracing::Level::DEBUG),
        }
    }

    /// Ticks the scheduler until it has nothing left to do (bounded).
    pub(crate) async fn drive(&self) -> u32 {
        let mut ticks = 0;
        for _ in 0..80 {
            let report = self.scheduler.tick().await;
            assert_eq!(report.failed, 0, "a scheduler action failed: {report:?}");
            if !report.did_work() {
                break;
            }
            ticks += 1;
        }
        ticks
    }

    /// Everything a worker can lease now, in a stable order: iteration, then node.
    pub(crate) async fn poll(&mut self) -> Vec<TaskLease> {
        let polled = self
            .worker
            .poll_tasks(worker_request(
                &self.api,
                PollTasksRequest {
                    worker_id: self.worker_id.clone(),
                    session_token: self.session_token.clone(),
                    max_tasks: 16,
                },
            ))
            .await;
        let mut tasks = match polled {
            Ok(response) => response.into_inner().tasks,
            Err(status) => panic!("poll tasks: {status:?}\n{}", self.logs.1.text()),
        };
        tasks.sort_by_key(|task| (iteration(task), node(task)));
        tasks
    }

    pub(crate) async fn complete(&mut self, task: &TaskLease, output: Value) {
        let response = self
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
                    idempotency_key: format!("complete-{}", task.step_id),
                    output_json: output.to_string(),
                    output_blob_ref: String::new(),
                    output_sha256: String::new(),
                    traceparent: String::new(),
                    instance: task.instance.clone(),
                },
            ))
            .await
            .expect("complete step")
            .into_inner();
        assert_eq!(response.step_status, "SUCCEEDED");
    }

    pub(crate) async fn fail(&mut self, task: &TaskLease, code: &str, retryable: bool) {
        self.worker
            .fail_step(worker_request(
                &self.api,
                FailStepRequest {
                    task_id: task.task_id.clone(),
                    run_id: task.run_id.clone(),
                    step_id: task.step_id.clone(),
                    attempt: task.attempt,
                    generation: task.generation,
                    lease_token: task.lease_token.clone(),
                    idempotency_key: format!("fail-{}-{}", task.step_id, task.attempt),
                    error_code: code.into(),
                    safe_message: String::new(),
                    retryable,
                    traceparent: String::new(),
                    instance: task.instance.clone(),
                },
            ))
            .await
            .expect("fail step");
    }

    pub(crate) async fn run_status(&self) -> String {
        sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
            .bind(self.run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("run status")
    }

    pub(crate) async fn verify(&self) {
        self.api
            .executions
            .verify_run(self.tenant, self.run)
            .await
            .expect("the journal rebuilds the projection");
    }
}

pub(crate) fn iteration(task: &TaskLease) -> u32 {
    task.instance
        .as_ref()
        .and_then(|instance| instance.instance_path.last())
        .map_or(u32::MAX, |segment| segment.iteration_index)
}

pub(crate) fn node(task: &TaskLease) -> String {
    task.instance
        .as_ref()
        .map(|instance| instance.definition_node_id.clone())
        .unwrap_or_default()
}

/// `(iteration, node, kind, activity)` of a batch of leases.
fn summary(tasks: &[TaskLease]) -> Vec<(u32, String, String, String)> {
    tasks
        .iter()
        .map(|task| {
            (
                iteration(task),
                node(task),
                task.task_kind.clone(),
                task.activity_name.clone(),
            )
        })
        .collect()
}

fn row(iteration: u32, node: &str, kind: &str, activity: &str) -> (u32, String, String, String) {
    (iteration, node.into(), kind.into(), activity.into())
}

#[tokio::test]
async fn a_failed_collection_undoes_what_its_iterations_completed_in_reverse_order() {
    let name = format!("scope-compensation-{}", Uuid::now_v7());
    let mut flow = Flow::start(
        &name,
        reserve_and_charge(&name, 2),
        json!([{"sku": "a"}, {"sku": "b"}]),
    )
    .await;

    // Forward work: both iterations reserve, then the first charges and the second is declined.
    flow.drive().await;
    let reserves = flow.poll().await;
    assert_eq!(
        summary(&reserves),
        vec![
            row(0, "reserve", "FORWARD", "line.reserve"),
            row(1, "reserve", "FORWARD", "line.reserve")
        ]
    );
    for (index, task) in reserves.iter().enumerate() {
        flow.complete(task, json!({"hold": index})).await;
    }
    flow.drive().await;
    let charges = flow.poll().await;
    assert_eq!(
        summary(&charges),
        vec![
            row(0, "charge", "FORWARD", "line.charge"),
            row(1, "charge", "FORWARD", "line.charge")
        ]
    );
    flow.complete(&charges[0], json!({"charged": 10})).await;
    flow.fail(&charges[1], "PAYMENT_DECLINED", false).await;
    flow.drive().await;

    // The scope stopped for the failure and compensates: what nothing waits for is ready first, in
    // both iterations at once, and the earlier effect of iteration 0 waits for the later one.
    assert_eq!(flow.run_status().await, "COMPENSATING");
    flow.verify().await;
    let first = flow.poll().await;
    assert_eq!(
        summary(&first),
        vec![
            row(0, "charge~c", "COMPENSATION", "line.refund"),
            row(1, "reserve~c", "COMPENSATION", "line.release")
        ]
    );
    for task in &first {
        // The task names the forward instance it undoes, at the same iteration, and carries the
        // key of the compensation, which is not the key of the effect.
        let target = task.target_instance.as_ref().expect("target instance");
        assert_eq!(
            target.instance_path,
            task.instance.as_ref().expect("instance").instance_path
        );
        assert!(task.idempotency_key.starts_with("compensation:"));
        assert!(!task.target_idempotency_key.is_empty());
        assert_ne!(task.idempotency_key, task.target_idempotency_key);
    }
    assert_ne!(
        first[0].idempotency_key, first[1].idempotency_key,
        "two iterations never share the key of their compensation"
    );
    for task in &first {
        flow.complete(task, json!({"undone": true})).await;
    }
    flow.drive().await;
    let second = flow.poll().await;
    assert_eq!(
        summary(&second),
        vec![row(0, "reserve~c", "COMPENSATION", "line.release")],
        "the reservation of iteration 0 is released only after its charge was refunded"
    );
    flow.complete(&second[0], json!({"undone": true})).await;
    flow.drive().await;

    assert_eq!(flow.run_status().await, "COMPENSATED");
    let scope: (String, String, String) = sqlx::query_as(
        "SELECT lifecycle, drain_kind, drain_failure_code FROM iteration_scopes WHERE run_id = $1",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("scope projection");
    assert_eq!(
        scope,
        ("FAILED".into(), "FAILED".into(), "PAYMENT_DECLINED".into())
    );
    let saga: (String, String, Option<String>, bool) = sqlx::query_as(
        "SELECT phase, cause_code, outcome, during_stop FROM scope_sagas WHERE run_id = $1",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("scope compensation projection");
    assert_eq!(
        saga,
        (
            "DONE".into(),
            "PAYMENT_DECLINED".into(),
            Some("COMPENSATED".into()),
            true
        )
    );
    flow.verify().await;
}
