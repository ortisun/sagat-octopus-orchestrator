//! A compensating collection or loop driven the way a deployment drives it: published over HTTP,
//! advanced by the scheduler, and undone by a worker over the worker protocol 1.5. Shared by the
//! integration and the fault suites of feature 014 (FR-014-005, FR-003-002).
#![allow(dead_code, clippy::expect_used, clippy::panic)]

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

use super::api::{TestApi, json_of};

pub type Sched = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

pub const ACTIVITIES: [&str; 6] = [
    "line.reserve",
    "line.charge",
    "line.notify",
    "line.release",
    "line.refund",
    "line.lookup",
];

pub fn retry() -> Value {
    json!({
        "maxAttempts": 2, "initialDelayMs": 100, "maxDelayMs": 100,
        "strategy": "fixed", "jitter": false
    })
}

pub fn activity(id: &str, name: &str, next: &str, extra: Value) -> Value {
    let mut node = json!({"id": id, "type": "activity", "activity": name, "next": next});
    for (key, value) in extra.as_object().expect("extra fields") {
        node[key] = value.clone();
    }
    node
}

pub fn collection_document(name: &str, nodes: Vec<Value>, concurrency: u32) -> Value {
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
pub fn reserve_and_charge(name: &str, concurrency: u32) -> Value {
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

pub struct Flow {
    pub api: TestApi,
    pub tenant: TenantId,
    pub run: RunId,
    pub scheduler: Sched,
    worker: WorkerServiceClient<Channel>,
    worker_id: String,
    session_token: String,
    logs: (tracing::subscriber::DefaultGuard, super::logs::Captured),
}

pub fn worker_request<T>(api: &TestApi, message: T) -> Request<T> {
    let token = api.idp.token("scope-worker", "acme").build();
    let mut request = Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        MetadataValue::try_from(format!("Bearer {token}")).expect("worker authorization"),
    );
    request
}

pub async fn publish(api: &TestApi, name: &str, document: Value) -> Result<String, Value> {
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
    pub async fn start(name: &str, document: Value, lines: Value) -> Self {
        let api = super::api::start_with(super::api::Options {
            runtime_role: true,
            ..super::api::Options::default()
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
                super::now(),
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
            logs: super::logs::capture_at(tracing::Level::DEBUG),
        }
    }

    /// Ticks the scheduler until it has nothing left to do (bounded), accepting that an action
    /// fails: returns how many ticks did work and how many actions failed.
    pub async fn drive_through_failures(&self) -> (u32, u32) {
        let (mut worked, mut failed) = (0, 0);
        for _ in 0..40 {
            let report = self.scheduler.tick().await;
            failed += report.failed;
            if !report.did_work() {
                break;
            }
            worked += 1;
        }
        (worked, failed)
    }

    /// Ticks the scheduler until it has nothing left to do (bounded).
    pub async fn drive(&self) -> u32 {
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
    pub async fn poll(&self) -> Vec<TaskLease> {
        match self.try_poll().await {
            Ok(tasks) => tasks,
            Err(status) => panic!("poll tasks: {status:?}\n{}", self.logs.1.text()),
        }
    }

    pub async fn try_poll(&self) -> Result<Vec<TaskLease>, tonic::Status> {
        let mut worker = self.worker.clone();
        let mut tasks = worker
            .poll_tasks(worker_request(
                &self.api,
                PollTasksRequest {
                    worker_id: self.worker_id.clone(),
                    session_token: self.session_token.clone(),
                    max_tasks: 16,
                },
            ))
            .await?
            .into_inner()
            .tasks;
        tasks.sort_by_key(|task| (iteration(task), node(task)));
        Ok(tasks)
    }

    pub async fn complete(&self, task: &TaskLease, output: Value) {
        let response = self
            .try_complete(task, output, &format!("complete-{}", task.step_id))
            .await
            .expect("complete step");
        assert_eq!(response.step_status, "SUCCEEDED");
    }

    /// A completion under an explicit idempotency key.
    pub async fn try_complete(
        &self,
        task: &TaskLease,
        output: Value,
        key: &str,
    ) -> Result<sagat_api_grpc::pb::StepCommandResponse, tonic::Status> {
        let mut worker = self.worker.clone();
        Ok(worker
            .complete_step(worker_request(
                &self.api,
                CompleteStepRequest {
                    task_id: task.task_id.clone(),
                    run_id: task.run_id.clone(),
                    step_id: task.step_id.clone(),
                    attempt: task.attempt,
                    generation: task.generation,
                    lease_token: task.lease_token.clone(),
                    idempotency_key: key.to_owned(),
                    output_json: output.to_string(),
                    output_blob_ref: String::new(),
                    output_sha256: String::new(),
                    traceparent: String::new(),
                    instance: task.instance.clone(),
                },
            ))
            .await?
            .into_inner())
    }

    pub async fn fail(&self, task: &TaskLease, code: &str, retryable: bool) {
        self.try_fail(task, code, retryable)
            .await
            .expect("fail step");
    }

    pub async fn try_fail(
        &self,
        task: &TaskLease,
        code: &str,
        retryable: bool,
    ) -> Result<(), tonic::Status> {
        let mut worker = self.worker.clone();
        worker
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
            .await?;
        Ok(())
    }

    /// An operation on the run (`CANCEL`, `RETRY_COMPENSATION`, ...); the status and the body.
    pub async fn operate(&self, body: Value) -> (StatusCode, Value) {
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
    pub async fn elapse_backoff(&self) {
        sqlx::query(
            "UPDATE steps SET next_attempt_at = now() - interval '1 second' \
             WHERE run_id = $1 AND status = 'RETRY_WAIT'",
        )
        .bind(self.run.as_uuid())
        .execute(&self.api.db.pool)
        .await
        .expect("elapse the backoff");
    }

    pub async fn run_status(&self) -> String {
        sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
            .bind(self.run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("run status")
    }

    pub async fn verify(&self) {
        self.api
            .executions
            .verify_run(self.tenant, self.run)
            .await
            .expect("the journal rebuilds the projection");
    }
}

pub fn iteration(task: &TaskLease) -> u32 {
    task.instance
        .as_ref()
        .and_then(|instance| instance.instance_path.last())
        .map_or(u32::MAX, |segment| segment.iteration_index)
}

pub fn node(task: &TaskLease) -> String {
    task.instance
        .as_ref()
        .map(|instance| instance.definition_node_id.clone())
        .unwrap_or_default()
}

/// `(iteration, node, kind, activity)` of a batch of leases.
pub fn summary(tasks: &[TaskLease]) -> Vec<(u32, String, String, String)> {
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

pub fn row(
    iteration: u32,
    node: &str,
    kind: &str,
    activity: &str,
) -> (u32, String, String, String) {
    (iteration, node.into(), kind.into(), activity.into())
}

/// Completes the forward work of one iteration and leaves the next step ready: `reserve` and
/// `charge` of the given iterations.
pub async fn forward(flow: &Flow, steps: &[(&str, u32)]) {
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

pub async fn all_compensations(flow: &Flow) {
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

/// A collection of two iterations in which iteration 0 completed `reserve` and the declined charge
/// of iteration 1 stopped the scope: the only compensation is the release of iteration 0.
pub async fn one_release_that_cannot_be_delivered(name: &str) -> (Flow, TaskLease) {
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

/// `reserve` can be looked up; its first attempt is leased and then lost.
pub async fn reserve_with_a_lost_attempt(name: &str) -> (Flow, TaskLease) {
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
    let flow = Flow::start(name, document, json!([{"sku": "a"}])).await;
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

// ---------------------------------------------------------------------------------------- loops

/// A `while` loop of up to three passes, each `reserve -> charge` with its compensations; the
/// pass returns its own number plus one, so the loop runs while the carry is below three. With
/// `on_error`, a failure that was undone takes the edge to `recovered`.
pub fn compensating_loop(name: &str, on_error: bool, irreversible_notify: bool) -> Value {
    let mut nodes = vec![activity(
        "reserve",
        "line.reserve",
        if irreversible_notify {
            "notify"
        } else {
            "charge"
        },
        json!({"compensateWith": "line.release", "compensationRetry": retry()}),
    )];
    if irreversible_notify {
        nodes.push(activity(
            "notify",
            "line.notify",
            "charge",
            json!({"irreversible": true}),
        ));
    }
    nodes.push(activity(
        "charge",
        "line.charge",
        "returned",
        json!({"compensateWith": "line.refund", "compensationRetry": retry()}),
    ));
    if irreversible_notify {
        // The charge of the irreversible variant has nothing to refund.
        let last = nodes.len() - 1;
        nodes[last] = activity("charge", "line.charge", "returned", json!({}));
    }
    nodes.push(json!({"id": "returned", "type": "return", "result": "iteration + 1"}));
    let mut poll = json!({
        "id": "poll", "type": "while", "condition": "carry < 3", "body": "pass",
        "initial": "0", "carry": "bodyResult", "maxIterations": 8, "timeoutMs": 600000,
        "next": "done"
    });
    let mut graph = vec![json!({"id": "done", "type": "end"})];
    if on_error {
        poll["onError"] = json!("recovered");
        graph.push(json!({"id": "recovered", "type": "end"}));
    }
    graph.insert(0, poll);
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "poll",
        "bodies": {"pass": {"entrypoint": "reserve", "nodes": nodes}},
        "nodes": graph
    })
}

/// Runs the passes of a loop: passes `0..completed` finish both effects, the next one reserves and
/// then its charge is declined.
pub async fn loop_until_a_charge_is_declined(flow: &Flow, completed: u32) {
    for pass in 0..=completed {
        flow.drive().await;
        let reserve = flow.poll().await;
        assert_eq!(
            summary(&reserve),
            vec![row(pass, "reserve", "FORWARD", "line.reserve")]
        );
        flow.complete(&reserve[0], json!({"hold": pass})).await;
        flow.drive().await;
        let charge = flow.poll().await;
        assert_eq!(
            summary(&charge),
            vec![row(pass, "charge", "FORWARD", "line.charge")]
        );
        if pass < completed {
            flow.complete(&charge[0], json!({"charged": pass})).await;
        } else {
            flow.fail(&charge[0], "PAYMENT_DECLINED", false).await;
        }
    }
    flow.drive().await;
}

/// The compensations the loop runs, one at a time, in the order they are leased.
pub async fn compensation_order(flow: &Flow) -> Vec<(u32, String)> {
    compensation_order_with_carry(flow).await.0
}

/// As [`compensation_order`], with the carry each compensation task was given.
pub async fn compensation_order_with_carry(flow: &Flow) -> (Vec<(u32, String)>, Vec<i64>) {
    let mut order = Vec::new();
    let mut carries = Vec::new();
    for _ in 0..16 {
        flow.drive().await;
        let tasks = flow.poll().await;
        if tasks.is_empty() {
            break;
        }
        // A loop undoes its iterations one after the other: never two compensations at once.
        assert_eq!(tasks.len(), 1, "{:?}", summary(&tasks));
        assert_eq!(tasks[0].task_kind, "COMPENSATION");
        order.push((iteration(&tasks[0]), node(&tasks[0])));
        let input: Value = serde_json::from_str(&tasks[0].input_json).expect("task input");
        carries.push(input["carry"].as_i64().expect("the iteration's carry"));
        flow.complete(&tasks[0], json!({"undone": true})).await;
    }
    flow.drive().await;
    (order, carries)
}

// ------------------------------------------------------------------------------ lock queue

/// A transaction that holds the row lock of one run until it is released.
pub struct RunHold {
    tx: sqlx::Transaction<'static, sqlx::Postgres>,
}

impl RunHold {
    pub async fn take(flow: &Flow) -> Self {
        let mut tx = flow.api.db.pool.begin().await.expect("holding transaction");
        sqlx::query("SELECT 1 FROM runs WHERE tenant_id=$1 AND id=$2 FOR UPDATE")
            .bind(flow.tenant.as_uuid())
            .bind(flow.run.as_uuid())
            .execute(&mut *tx)
            .await
            .expect("hold the run row");
        Self { tx }
    }

    /// Lets go without changing anything.
    pub async fn release(self) {
        self.tx.rollback().await.expect("release the run row");
    }
}

/// Completes when at least `count` backends of this database wait on a lock. This watches an
/// observable condition of the database; it is not a pause.
pub async fn blocked(pool: &sqlx::PgPool, count: i64) {
    loop {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM pg_stat_activity \
             WHERE datname = current_database() AND wait_event_type = 'Lock'",
        )
        .fetch_one(pool)
        .await
        .expect("lock waiters");
        if waiting >= count {
            return;
        }
        tokio::task::yield_now().await;
    }
}

/// Ends the connections that wait on a lock: the way the database sees a client die.
pub async fn kill_waiters(pool: &sqlx::PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT count(pg_terminate_backend(pid)) FROM pg_stat_activity \
         WHERE datname = current_database() AND wait_event_type = 'Lock'",
    )
    .fetch_one(pool)
    .await
    .expect("terminate the waiting backends")
}

/// Polls its future at most `polls` times and gives it up, dropped mid-flight, if it is still
/// pending after the last poll. Every poll that returns pending is one await point of the future,
/// so counting up from one cuts it at each of them in turn.
pub struct CutAfter<F> {
    pub inner: std::pin::Pin<Box<F>>,
    pub polls: usize,
}

impl<F: std::future::Future> std::future::Future for CutAfter<F> {
    type Output = Option<F::Output>;

    fn poll(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        if self.polls == 0 {
            return std::task::Poll::Ready(None);
        }
        self.polls -= 1;
        match self.inner.as_mut().poll(cx) {
            std::task::Poll::Ready(output) => std::task::Poll::Ready(Some(output)),
            std::task::Poll::Pending if self.polls == 0 => std::task::Poll::Ready(None),
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

/// The committed journal of the run in order: `(seq, event type)`.
pub async fn journal(flow: &Flow) -> Vec<(i64, String)> {
    sqlx::query_as(
        "SELECT seq, event_type FROM run_events WHERE tenant_id=$1 AND run_id=$2 ORDER BY seq",
    )
    .bind(flow.tenant.as_uuid())
    .bind(flow.run.as_uuid())
    .fetch_all(&flow.api.db.pool)
    .await
    .expect("journal")
}

/// Nothing is left open: no session of this database idles inside a transaction.
pub async fn assert_no_open_transaction(flow: &Flow, what: &str) {
    let open: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND state LIKE 'idle in transaction%'",
    )
    .fetch_one(&flow.api.db.pool)
    .await
    .expect("sessions");
    assert_eq!(open, 0, "{what} left a session idle in a transaction");
}

/// The task of `tasks` for the given iteration and node.
pub fn pick<'a>(tasks: &'a [TaskLease], iteration_index: u32, node_id: &str) -> &'a TaskLease {
    tasks
        .iter()
        .find(|task| iteration(task) == iteration_index && node(task) == node_id)
        .unwrap_or_else(|| panic!("no task for {node_id} of iteration {iteration_index}"))
}

impl Flow {
    /// Ticks until the scope's drain has settled every iteration and the compensation is not yet
    /// planned: the next action of the scheduler is the planning transaction.
    pub async fn until_planning(&self) {
        for _ in 0..40 {
            let ready: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM iteration_scopes scope WHERE scope.run_id = $1 \
                   AND scope.lifecycle = 'DRAINING') \
                 AND NOT EXISTS (SELECT 1 FROM collection_iterations WHERE run_id = $1 \
                   AND lifecycle = 'ADMITTED') \
                 AND NOT EXISTS (SELECT 1 FROM scope_sagas WHERE run_id = $1)",
            )
            .bind(self.run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("planning state");
            if ready {
                return;
            }
            let report = self.scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
        }
        panic!("the scope never reached the point where it plans");
    }

    /// Ticks until the compensation is over and the scope has not finished: the next action of
    /// the scheduler is the transaction that ends the scope and the run.
    pub async fn until_finishing(&self) {
        for _ in 0..40 {
            let ready: bool = sqlx::query_scalar(
                "SELECT EXISTS (SELECT 1 FROM scope_sagas WHERE run_id = $1 AND phase = 'DONE') \
                 AND EXISTS (SELECT 1 FROM iteration_scopes WHERE run_id = $1 \
                   AND lifecycle = 'DRAINING')",
            )
            .bind(self.run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("finishing state");
            if ready {
                return;
            }
            let report = self.scheduler.tick().await;
            assert_eq!(report.failed, 0, "{report:?}");
            for task in self.poll().await {
                self.complete(&task, serde_json::json!({"undone": true}))
                    .await;
            }
        }
        panic!("the scope never reached the point where it finishes");
    }

    /// Gives the connections of the API pool back to a clean state: a dropped transaction rolls
    /// back when its connection is next used, so every connection is taken once.
    pub async fn flush_connections(&self) {
        let size = self.api.api_pool.options().get_max_connections();
        let mut held = Vec::new();
        for _ in 0..size {
            held.push(
                self.api
                    .api_pool
                    .acquire()
                    .await
                    .expect("a pooled connection"),
            );
        }
        drop(held);
    }
}
