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

    /// An operation on the run (`CANCEL`, `RETRY_COMPENSATION`, ...); the status and the body.
    pub(crate) async fn operate(&self, body: Value) -> (StatusCode, Value) {
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
            .json(&body)
            .send()
            .await
            .expect("operate on the run");
        let status = response.status();
        (status, json_of(response).await)
    }

    /// Makes every backoff that is waiting due now, as time would.
    pub(crate) async fn elapse_backoff(&self) {
        sqlx::query(
            "UPDATE steps SET next_attempt_at = now() - interval '1 second' \
             WHERE run_id = $1 AND status = 'RETRY_WAIT'",
        )
        .bind(self.run.as_uuid())
        .execute(&self.api.db.pool)
        .await
        .expect("elapse the backoff");
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

/// Completes the forward work of one iteration and leaves the next step ready: `reserve` and
/// `charge` of the given iterations.
async fn forward(flow: &mut Flow, steps: &[(&str, u32)]) {
    flow.drive().await;
    let ready = flow.poll().await;
    for task in &ready {
        let key = (node(task), iteration(task));
        if steps.iter().any(|(n, i)| (n.to_string(), *i) == key) {
            flow.complete(task, json!({"done": key.0})).await;
        }
    }
    flow.drive().await;
}

async fn all_compensations(flow: &mut Flow) {
    for _ in 0..12 {
        flow.drive().await;
        let tasks = flow.poll().await;
        if tasks.is_empty() {
            break;
        }
        for task in &tasks {
            assert_eq!(task.task_kind, "COMPENSATION");
            flow.complete(task, json!({"undone": true})).await;
        }
    }
    flow.drive().await;
}

#[tokio::test]
async fn a_cancelled_collection_undoes_what_completed_and_ends_cancelled() {
    let name = format!("scope-cancel-{}", Uuid::now_v7());
    let mut flow = Flow::start(
        &name,
        reserve_and_charge(&name, 2),
        json!([{"sku": "a"}, {"sku": "b"}]),
    )
    .await;
    forward(&mut flow, &[("reserve", 0), ("reserve", 1)]).await;
    // Iteration 0 charges; the charge of iteration 1 is in flight when the run is cancelled.
    let charges = flow.poll().await;
    assert_eq!(charges.len(), 2);
    flow.complete(&charges[0], json!({"charged": 10})).await;
    flow.drive().await;
    let (status, _) = flow
        .operate(json!({"action": "CANCEL", "reason": "the operator changed their mind"}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    flow.drive().await;
    // The attempt in flight is asked to stop; its worker acknowledges, and nothing says it happened
    // or not, so it is not compensated (it failed with a known cause).
    flow.fail(&charges[1], "CANCELLED_BY_WORKER", false).await;
    all_compensations(&mut flow).await;

    assert_eq!(flow.run_status().await, "CANCELLED");
    let scope: (String, String) =
        sqlx::query_as("SELECT lifecycle, drain_kind FROM iteration_scopes WHERE run_id = $1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("scope");
    assert_eq!(scope, ("CANCELLED".into(), "CANCELLED".into()));
    let undone: Vec<(i32, String)> = sqlx::query_as(
        "SELECT instance_path_indices[1], node FROM steps \
         WHERE run_id = $1 AND kind = 'COMPENSATION' AND status = 'SUCCEEDED' \
         ORDER BY instance_path_indices[1], node",
    )
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("compensations");
    assert_eq!(
        undone,
        vec![
            (0, "charge~c".into()),
            (0, "reserve~c".into()),
            (1, "reserve~c".into())
        ]
    );
    flow.verify().await;
}

#[tokio::test]
async fn an_irreversible_effect_ends_the_run_partial_and_is_never_called_compensated() {
    let name = format!("scope-partial-{}", Uuid::now_v7());
    let document = collection_document(
        &name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "notify",
                json!({"compensateWith": "line.release", "compensationRetry": retry()}),
            ),
            activity(
                "notify",
                "line.notify",
                "charge",
                json!({"irreversible": true}),
            ),
            activity("charge", "line.charge", "returned", json!({})),
            json!({"id": "returned", "type": "return", "result": "steps.charge.output"}),
        ],
        1,
    );
    let mut flow = Flow::start(&name, document, json!([{"sku": "a"}])).await;
    forward(&mut flow, &[("reserve", 0)]).await;
    forward(&mut flow, &[("notify", 0)]).await;
    let charge = flow.poll().await;
    assert_eq!(
        summary(&charge),
        vec![row(0, "charge", "FORWARD", "line.charge")]
    );
    flow.fail(&charge[0], "PAYMENT_DECLINED", false).await;
    all_compensations(&mut flow).await;

    assert_eq!(flow.run_status().await, "PARTIAL");
    let saga: (String, Option<String>) =
        sqlx::query_as("SELECT phase, outcome FROM scope_sagas WHERE run_id = $1")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("saga");
    assert_eq!(saga, ("DONE".into(), Some("PARTIAL".into())));
    let irreversible: Vec<(i32, String)> = sqlx::query_as(
        "SELECT iteration_index, node FROM scope_saga_entries \
         WHERE run_id = $1 AND kind = 'IRREVERSIBLE'",
    )
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("irreversible entries");
    assert_eq!(irreversible, vec![(0, "notify".into())]);
    flow.verify().await;
}

/// A collection of two iterations in which iteration 0 completed `reserve` and the declined charge
/// of iteration 1 stopped the scope: the only compensation is the release of iteration 0.
async fn one_release_that_cannot_be_delivered(name: &str) -> (Flow, TaskLease) {
    let document = collection_document(
        name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "charge",
                json!({"compensateWith": "line.release", "compensationRetry": retry()}),
            ),
            activity("charge", "line.charge", "returned", json!({})),
            json!({"id": "returned", "type": "return", "result": "steps.charge.output"}),
        ],
        2,
    );
    let mut flow = Flow::start(name, document, json!([{"sku": "a"}, {"sku": "b"}])).await;
    forward(&mut flow, &[("reserve", 0), ("reserve", 1)]).await;
    let charges = flow.poll().await;
    flow.complete(&charges[0], json!({"charged": 1})).await;
    flow.fail(&charges[1], "PAYMENT_DECLINED", false).await;
    flow.drive().await;
    // Both reservations are released; the first is delivered, the second keeps failing.
    let releases = flow.poll().await;
    assert_eq!(releases.len(), 2);
    assert!(releases.iter().all(|task| task.task_kind == "COMPENSATION"));
    flow.complete(&releases[0], json!({"undone": true})).await;
    let mut stuck = releases[1].clone();
    for attempt in 1..=2 {
        flow.fail(&stuck, "PROVIDER_DOWN", true).await;
        if attempt == 1 {
            flow.elapse_backoff().await;
            flow.drive().await;
            let again = flow.poll().await;
            assert_eq!(again.len(), 1, "the retry of the compensation is leased");
            assert_eq!(
                again[0].idempotency_key, stuck.idempotency_key,
                "a retry asks the provider with the same key"
            );
            stuck = again[0].clone();
        }
    }
    flow.drive().await;
    (flow, stuck)
}

#[tokio::test]
async fn a_compensation_that_keeps_failing_waits_for_a_person_who_can_retry_it() {
    let name = format!("scope-review-retry-{}", Uuid::now_v7());
    let (mut flow, stuck) = one_release_that_cannot_be_delivered(&name).await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let waiting: (String, Option<String>, Option<String>) = sqlx::query_as(
        "SELECT status, waiting_kind, failure_code FROM steps \
         WHERE run_id = $1 AND node = 'reserve~c' AND instance_path_indices = ARRAY[1]",
    )
    .bind(flow.run.as_uuid())
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("waiting compensation");
    assert_eq!(
        waiting,
        (
            "WAITING".into(),
            Some("OPERATOR".into()),
            Some("PROVIDER_DOWN".into())
        )
    );
    // A run that is not decided is not finished: the scheduler leaves it alone however often it runs.
    assert_eq!(flow.drive().await, 0);
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let key = stuck.idempotency_key.clone();

    let (status, body) = flow
        .operate(json!({
            "action": "RETRY_COMPENSATION", "stepId": "reserve~c",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 1}],
            "reason": "the provider is back, release the reservation"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    let retried = flow.poll().await;
    assert_eq!(retried.len(), 1);
    assert_eq!(
        retried[0].idempotency_key, key,
        "the person's retry asks with the same key"
    );
    flow.complete(&retried[0], json!({"undone": true})).await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_records WHERE resource_id LIKE $1 AND action = 'saga.step_retried'",
    )
    .bind(format!("{}/lines[1]/reserve~c", flow.run))
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("audit");
    assert_eq!(audits, 1);
    flow.verify().await;
}

#[tokio::test]
async fn a_person_can_skip_a_compensation_and_the_run_ends_partial() {
    let name = format!("scope-review-skip-{}", Uuid::now_v7());
    let (mut flow, _) = one_release_that_cannot_be_delivered(&name).await;
    let (status, body) = flow
        .operate(json!({
            "action": "SKIP_COMPENSATION", "stepId": "reserve~c",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 1}],
            "reason": "released by hand at the provider"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    assert_eq!(flow.run_status().await, "PARTIAL");
    flow.verify().await;
}

#[tokio::test]
async fn a_person_can_declare_a_compensation_failed_and_the_run_ends_compensation_failed() {
    let name = format!("scope-review-fail-{}", Uuid::now_v7());
    let (mut flow, _) = one_release_that_cannot_be_delivered(&name).await;
    let (status, body) = flow
        .operate(json!({
            "action": "RESOLVE_STEP", "stepId": "reserve~c", "outcome": "FAILED",
            "errorCode": "PROVIDER_REFUSED",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 1}],
            "reason": "the provider confirmed it cannot release"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    assert_eq!(flow.run_status().await, "COMPENSATION_FAILED");
    flow.verify().await;
}

#[tokio::test]
async fn a_cancellation_never_gives_up_on_a_compensation_that_waits_for_a_person() {
    let name = format!("scope-review-cancel-{}", Uuid::now_v7());
    let (mut flow, _) = one_release_that_cannot_be_delivered(&name).await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    // The run is already ending for the declined charge: a second stop is not accepted, and the
    // person still has to decide.
    let (status, _) = flow
        .operate(json!({"action": "CANCEL", "reason": "cancel while it waits for a person"}))
        .await;
    assert!(status.is_client_error(), "{status}");
    flow.drive().await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    flow.verify().await;
}

#[tokio::test]
async fn a_cancelled_run_whose_compensation_keeps_failing_ends_partial_not_cancelled() {
    let name = format!("scope-cancel-review-{}", Uuid::now_v7());
    let document = collection_document(
        &name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "returned",
                json!({"compensateWith": "line.release", "compensationRetry": retry()}),
            ),
            json!({"id": "returned", "type": "return", "result": "steps.reserve.output"}),
        ],
        2,
    );
    let mut flow = Flow::start(&name, document, json!([{"sku": "a"}, {"sku": "b"}])).await;
    flow.drive().await;
    let reserves = flow.poll().await;
    flow.complete(&reserves[0], json!({"hold": 1})).await;
    flow.drive().await;
    let (status, _) = flow
        .operate(json!({"action": "CANCEL", "reason": "stop everything now please"}))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED);
    flow.drive().await;
    flow.fail(&reserves[1], "CANCELLED_BY_WORKER", false).await;
    flow.drive().await;
    let mut release = flow.poll().await;
    assert_eq!(release.len(), 1);
    for attempt in 1..=2 {
        flow.fail(&release[0], "PROVIDER_DOWN", true).await;
        if attempt == 1 {
            flow.elapse_backoff().await;
            flow.drive().await;
            release = flow.poll().await;
        }
    }
    flow.drive().await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let (status, body) = flow
        .operate(json!({
            "action": "SKIP_COMPENSATION", "stepId": "reserve~c",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 0}],
            "reason": "released by hand at the provider"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    assert_eq!(
        flow.run_status().await,
        "PARTIAL",
        "a cancellation whose compensation was not completed is not a clean cancellation"
    );
    flow.verify().await;
}

/// `reserve` can be looked up; its first attempt is leased and then lost.
async fn reserve_with_a_lost_attempt(name: &str) -> (Flow, TaskLease) {
    let document = collection_document(
        name,
        vec![
            activity(
                "reserve",
                "line.reserve",
                "charge",
                json!({
                    "compensateWith": "line.release", "reconcileWith": "line.lookup",
                    "compensationRetry": retry(), "retry": retry()
                }),
            ),
            activity("charge", "line.charge", "returned", json!({})),
            json!({"id": "returned", "type": "return", "result": "steps.charge.output"}),
        ],
        1,
    );
    let mut flow = Flow::start(name, document, json!([{"sku": "a"}])).await;
    flow.drive().await;
    let leased = flow.poll().await;
    assert_eq!(
        summary(&leased),
        vec![row(0, "reserve", "FORWARD", "line.reserve")]
    );
    sqlx::query(
        "UPDATE steps SET lease_expires_at = now() - interval '1 second' \
         WHERE run_id = $1 AND node = 'reserve' AND status IN ('LEASED', 'RUNNING')",
    )
    .bind(flow.run.as_uuid())
    .execute(&flow.api.db.pool)
    .await
    .expect("lose the lease");
    flow.drive().await;
    (flow, leased[0].clone())
}

#[tokio::test]
async fn an_effect_of_unknown_outcome_is_looked_up_and_found_applied_before_anything_repeats() {
    let name = format!("scope-lookup-applied-{}", Uuid::now_v7());
    let (mut flow, lost) = reserve_with_a_lost_attempt(&name).await;
    let lookups = flow.poll().await;
    assert_eq!(
        summary(&lookups),
        vec![row(0, "reserve~r", "RECONCILIATION", "line.lookup")]
    );
    let lookup = &lookups[0];
    assert_eq!(lookup.target_node, "reserve");
    assert_eq!(
        lookup.target_idempotency_key, lost.idempotency_key,
        "the lookup names the effect by the key its attempts carried"
    );
    assert!(lookup.idempotency_key.starts_with("reconciliation:"));
    flow.complete(lookup, json!({"effect": "applied", "output": {"hold": 7}}))
        .await;
    flow.drive().await;
    // The step happened: the iteration goes on with the output the lookup found, and the effect was
    // not repeated (the forward step has had one attempt).
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempt FROM steps WHERE run_id = $1 AND node = 'reserve'")
            .bind(flow.run.as_uuid())
            .fetch_one(&flow.api.db.pool)
            .await
            .expect("attempts");
    assert_eq!(attempts, 1);
    let next = flow.poll().await;
    assert_eq!(
        summary(&next),
        vec![row(0, "charge", "FORWARD", "line.charge")]
    );
    let input: Value = serde_json::from_str(&next[0].input_json).expect("input");
    assert_eq!(input["steps"]["reserve"]["output"], json!({"hold": 7}));
    // Now the charge fails: what the lookup found is undone like any other effect.
    flow.fail(&next[0], "PAYMENT_DECLINED", false).await;
    all_compensations(&mut flow).await;
    assert_eq!(flow.run_status().await, "COMPENSATED");
    flow.verify().await;
}

#[tokio::test]
async fn an_effect_found_not_applied_is_tried_again_with_the_same_key() {
    let name = format!("scope-lookup-not-applied-{}", Uuid::now_v7());
    let (mut flow, lost) = reserve_with_a_lost_attempt(&name).await;
    let lookups = flow.poll().await;
    flow.complete(&lookups[0], json!({"effect": "not_applied"}))
        .await;
    flow.elapse_backoff().await;
    flow.drive().await;
    let again = flow.poll().await;
    assert_eq!(
        summary(&again),
        vec![row(0, "reserve", "FORWARD", "line.reserve")]
    );
    assert_eq!(again[0].attempt, 2);
    assert_eq!(
        again[0].idempotency_key, lost.idempotency_key,
        "the second attempt carries the key of the first"
    );
    flow.verify().await;
}

#[tokio::test]
async fn a_lookup_that_cannot_say_waits_for_a_person_who_declares_what_happened() {
    let name = format!("scope-lookup-unknown-{}", Uuid::now_v7());
    let (mut flow, _) = reserve_with_a_lost_attempt(&name).await;
    let lookups = flow.poll().await;
    flow.complete(&lookups[0], json!({"effect": "unknown"}))
        .await;
    flow.drive().await;
    assert_eq!(flow.run_status().await, "MANUAL_REVIEW");
    let (status, body) = flow
        .operate(json!({
            "action": "RESOLVE_STEP", "stepId": "reserve", "outcome": "SUCCEEDED",
            "instancePath": [{"scopeNodeId": "lines", "iterationIndex": 0}],
            "reason": "the provider's ledger shows the reservation"
        }))
        .await;
    assert_eq!(status, StatusCode::ACCEPTED, "{body}");
    flow.drive().await;
    let next = flow.poll().await;
    assert_eq!(
        summary(&next),
        vec![row(0, "charge", "FORWARD", "line.charge")]
    );
    flow.verify().await;
}
