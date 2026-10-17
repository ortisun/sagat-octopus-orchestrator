//! An approval driven the way a deployment drives it: published over HTTP, started by a person, opened
//! and expired by the scheduler, decided over the public route, with declared Query/Update calls and
//! chain commands on the same run. Shared by the integration and the fault suites of feature 014
//! (FR-014-007, FR-014-010, FR-014-012; decision D295).
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::time::Duration as StdDuration;

use reqwest::{Method, StatusCode};
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, PollTasksRequest, RegisterWorkerRequest, TaskLease, WorkerCapabilities,
};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_domain::execution::RunId;
use sagat_domain::identity::{Membership, Role, Subject, TenantId};
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Keeper, KeeperSettings, Placement, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use tonic::transport::Channel;
use uuid::Uuid;

use super::api::{Options, TestApi, json_of};
use super::scope_flow::{Sched, publish};

/// The initiator of every run (an admin).
pub const INITIATOR: &str = "alice";
/// An operator who may decide (the policy of the shared documents admits `operator`).
pub const OPERATOR: &str = "oscar";
/// A second operator, for the loser of a race.
pub const OTHER_OPERATOR: &str = "olivia";
/// A viewer: may list approvals, may not decide.
pub const VIEWER: &str = "vera";

pub struct ApprovalFlow {
    pub api: TestApi,
    pub tenant: TenantId,
    pub namespace: String,
    pub scheduler: Sched,
    worker: WorkerServiceClient<Channel>,
    worker_id: String,
    session_token: String,
}

/// A document whose only declared state is a balance, with one Query and one Update.
#[must_use]
pub fn state_block() -> Value {
    json!({
        "schema": {
            "type": "object",
            "additionalProperties": false,
            "properties": {"balance": {"type": "integer"}},
            "required": ["balance"]
        },
        "initial": "input.account",
        "queries": {
            "balance.read": {
                "argsSchema": {"type": "integer"},
                "resultSchema": {"type": "integer"},
                "expression": "state.balance + args",
                "disclosure": "declared-business-result-v1"
            }
        },
        "updates": {
            "balance.replace": {
                "argsSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "properties": {
                        "allowed": {"type": "boolean"},
                        "nextState": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {"balance": {"type": "integer"}},
                            "required": ["balance"]
                        }
                    },
                    "required": ["allowed", "nextState"]
                },
                "validate": "args.allowed",
                "state": "args.nextState",
                "result": "newState.balance",
                "resultSchema": {"type": "integer"},
                "disclosure": "declared-business-result-v1"
            }
        }
    })
}

/// `review` (approval, operators, initiator excluded) routing to three `end` nodes that tell which
/// route was taken.
#[must_use]
pub fn approval_nodes(timeout_ms: u64) -> Vec<Value> {
    vec![
        json!({
            "id": "review",
            "type": "approval",
            "timeoutMs": timeout_ms,
            "policy": {"roles": ["admin", "operator"], "excludeInitiator": true},
            "onApprove": "approved",
            "onReject": "rejected",
            "onExpire": "expired"
        }),
        json!({"id": "approved", "type": "end", "outputExpr": "'approved'"}),
        json!({"id": "rejected", "type": "end", "outputExpr": "'rejected'"}),
        json!({"id": "expired", "type": "end", "outputExpr": "'expired'"}),
    ]
}

#[must_use]
pub fn approval_with_state(name: &str, timeout_ms: u64) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "review",
        "nodes": approval_nodes(timeout_ms),
        "state": state_block()
    })
}

impl ApprovalFlow {
    /// A platform with one namespace, the roles above, a scheduler holding every shard and a worker
    /// bound to `activities`.
    pub async fn boot(activities: &[&str]) -> Self {
        // `RUST_LOG=debug cargo test ...` shows the causes the API deliberately hides from clients.
        static LOGGING: std::sync::Once = std::sync::Once::new();
        LOGGING.call_once(|| {
            if std::env::var_os("RUST_LOG").is_some() {
                sagat_telemetry::init_logging(sagat_telemetry::LogFormat::Pretty);
            }
        });
        let api = super::api::start_with(Options::default()).await;
        let tenant = api.tenant("acme", INITIATOR).await;
        let workspace = api.create_workspace(INITIATOR, "acme", "approvals").await;
        let namespace = api
            .create_namespace(INITIATOR, "acme", &workspace, "payments", "dev")
            .await;
        for (subject, role) in [
            (OPERATOR, Role::Operator),
            (OTHER_OPERATOR, Role::Operator),
            (VIEWER, Role::Viewer),
            ("approval-worker", Role::Worker),
        ] {
            api.owner_store
                .insert_membership(&Membership::restore(
                    tenant,
                    Subject::parse(subject).expect("subject"),
                    role,
                    super::now(),
                ))
                .await
                .expect("membership");
        }
        let mut activities: Vec<&str> = activities.to_vec();
        if activities.is_empty() {
            activities.push("approval.unused");
        }
        let activities = activities.as_slice();
        let binding = api
            .as_user(
                Method::PUT,
                &format!("/namespaces/{namespace}/worker-bindings/approval-worker"),
                INITIATOR,
                "acme",
            )
            .json(&json!({"activities": activities}))
            .send()
            .await
            .expect("worker binding");
        assert_eq!(binding.status(), StatusCode::OK);

        let placement = Placement::new(format!("approval-flow-{}", Uuid::now_v7()));
        Keeper::new(
            api.executions.clone(),
            placement.clone(),
            KeeperSettings {
                ttl: StdDuration::from_secs(600),
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

        let mut worker = WorkerServiceClient::connect(api.grpc_url.clone())
            .await
            .expect("connect worker");
        let registered = worker
            .register_worker(approval_worker_request(
                &api,
                RegisterWorkerRequest {
                    worker_name: "approval-flow-worker".into(),
                    namespace: "payments".into(),
                    capabilities: Some(WorkerCapabilities {
                        languages: vec!["rust".into()],
                        activities: activities.iter().map(|a| (*a).to_owned()).collect(),
                        pools: Vec::new(),
                        max_inflight: 16,
                        protocol_version: "1.5".into(),
                    }),
                },
            ))
            .await
            .expect("register worker")
            .into_inner();
        Self {
            api,
            tenant,
            namespace,
            scheduler,
            worker,
            worker_id: registered.worker_id,
            session_token: registered.session_token,
        }
    }

    /// Publishes `document`; the workflow id, or the error body of the refusal.
    pub async fn publish(&self, name: &str, document: Value) -> Result<String, Value> {
        publish(&self.api, name, document).await
    }

    pub async fn start(&self, workflow: &str, input: Value) -> RunId {
        let (status, body) = self.try_start(workflow, input).await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        body["runId"]
            .as_str()
            .expect("run id")
            .parse()
            .expect("run id")
    }

    pub async fn try_start(&self, workflow: &str, input: Value) -> (StatusCode, Value) {
        self.try_start_traced(workflow, input, None).await
    }

    /// Starts a run under the W3C trace context `traceparent`, if one is given.
    pub async fn start_traced(&self, workflow: &str, input: Value, traceparent: &str) -> RunId {
        let (status, body) = self
            .try_start_traced(workflow, input, Some(traceparent))
            .await;
        assert_eq!(status, StatusCode::ACCEPTED, "{body}");
        body["runId"]
            .as_str()
            .expect("run id")
            .parse()
            .expect("run id")
    }

    async fn try_start_traced(
        &self,
        workflow: &str,
        input: Value,
        traceparent: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = self
            .api
            .as_user(Method::POST, "/runs", INITIATOR, "acme")
            .header("x-octopus-namespace", "payments");
        if let Some(traceparent) = traceparent {
            request = request.header("traceparent", traceparent);
        }
        let started = request
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"workflowId": workflow, "input": input}))
            .send()
            .await
            .expect("start run");
        let status = started.status();
        (status, json_of(started).await)
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

    /// The approvals of `run`, listed by the operator.
    pub async fn approvals(&self, run: RunId) -> Vec<Value> {
        let response = self
            .api
            .get_as(&format!("/runs/{run}/approvals"), OPERATOR, "acme")
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        json_of(response).await["items"]
            .as_array()
            .expect("approval items")
            .clone()
    }

    /// The only approval of `run`, after the scheduler has opened it.
    pub async fn the_approval(&self, run: RunId) -> String {
        self.drive().await;
        let items = self.approvals(run).await;
        assert_eq!(items.len(), 1, "{items:?}");
        items[0]["approvalId"]
            .as_str()
            .expect("approval id")
            .to_owned()
    }

    pub async fn decide(
        &self,
        approval: &str,
        subject: &str,
        key: &str,
        decision: &str,
    ) -> (StatusCode, Value) {
        let response = self
            .api
            .post_as(
                &format!("/approvals/{approval}/decisions"),
                subject,
                "acme",
                &json!({"decisionKey": key, "decision": decision, "reasonCode": "POLICY_OK"}),
            )
            .await;
        let status = response.status();
        (status, json_of(response).await)
    }

    pub async fn update(&self, run: RunId, key: &str, balance: i64) -> (StatusCode, Value) {
        self.update_expecting(run, key, balance, None).await
    }

    pub async fn update_expecting(
        &self,
        run: RunId,
        key: &str,
        balance: i64,
        expected_revision: Option<u64>,
    ) -> (StatusCode, Value) {
        let mut body = json!({
            "updateKey": key,
            "args": {"allowed": true, "nextState": {"balance": balance}}
        });
        if let Some(revision) = expected_revision {
            body["expectedStateRevision"] = json!(revision);
        }
        let response = self
            .api
            .post_as(
                &format!("/runs/{run}/updates/balance.replace"),
                INITIATOR,
                "acme",
                &body,
            )
            .await;
        let status = response.status();
        (status, json_of(response).await)
    }

    pub async fn query(&self, run: RunId, args: i64) -> (StatusCode, Value) {
        let response = self
            .api
            .post_as(
                &format!("/runs/{run}/queries/balance.read"),
                INITIATOR,
                "acme",
                &json!({"args": args}),
            )
            .await;
        let status = response.status();
        (status, json_of(response).await)
    }

    /// A Query addressed to the chain (its current head), the way a client reads a chain's state.
    pub async fn chain_query(&self, chain: RunId, args: i64) -> (StatusCode, Value) {
        let response = self
            .api
            .post_as(
                &format!(
                    "/namespaces/{}/chains/{chain}/queries/balance.read",
                    self.namespace
                ),
                INITIATOR,
                "acme",
                &json!({"args": args}),
            )
            .await;
        let status = response.status();
        (status, json_of(response).await)
    }

    /// An Update addressed to the chain; `expected_head` fences it to one segment.
    pub async fn chain_update(
        &self,
        chain: RunId,
        key: &str,
        balance: i64,
        expected_head: Option<RunId>,
    ) -> (StatusCode, Value) {
        let mut body = json!({
            "updateKey": key,
            "args": {"allowed": true, "nextState": {"balance": balance}}
        });
        if let Some(head) = expected_head {
            body["expectedRunId"] = json!(head.to_string());
        }
        let response = self
            .api
            .post_as(
                &format!(
                    "/namespaces/{}/chains/{chain}/updates/balance.replace",
                    self.namespace
                ),
                INITIATOR,
                "acme",
                &body,
            )
            .await;
        let status = response.status();
        (status, json_of(response).await)
    }

    pub async fn cancel(&self, run: RunId) -> (StatusCode, Value) {
        let response = self
            .api
            .as_user(
                Method::POST,
                &format!("/runs/{run}/operations"),
                INITIATOR,
                "acme",
            )
            .header("x-octopus-namespace", "payments")
            .header("idempotency-key", Uuid::now_v7().to_string())
            .json(&json!({"action": "CANCEL", "reason": "operator stopped the request"}))
            .send()
            .await
            .expect("cancel");
        let status = response.status();
        (status, json_of(response).await)
    }

    /// `GET /namespaces/{ns}/chains/{chain}`.
    pub async fn chain(&self, chain: RunId) -> Value {
        let response = self
            .api
            .get_as(
                &format!("/namespaces/{}/chains/{chain}", self.namespace),
                INITIATOR,
                "acme",
            )
            .await;
        assert_eq!(response.status(), StatusCode::OK);
        json_of(response).await
    }

    /// The head run of `chain` now.
    pub async fn head(&self, chain: RunId) -> RunId {
        self.chain(chain).await["currentRunId"]
            .as_str()
            .expect("head")
            .parse()
            .expect("head run id")
    }

    /// Cancels the chain through its head; the status and the body.
    pub async fn cancel_chain(&self, chain: RunId, expected_head: RunId) -> (StatusCode, Value) {
        let response = self
            .api
            .as_user(
                Method::POST,
                &format!("/namespaces/{}/chains/{chain}/cancel", self.namespace),
                INITIATOR,
                "acme",
            )
            .header(
                "idempotency-key",
                format!("chain-cancel-{}", Uuid::now_v7()),
            )
            .json(&json!({
                "expectedRunId": expected_head.to_string(),
                "reason": "operator stopped the chain"
            }))
            .send()
            .await
            .expect("cancel chain");
        let status = response.status();
        let text = response.text().await.expect("cancel body");
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    pub async fn run_status(&self, run: RunId) -> String {
        sqlx::query_scalar("SELECT status FROM runs WHERE id = $1")
            .bind(run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("run status")
    }

    /// What the telemetry projector derives from the run's committed journal now.
    pub async fn derived(&self, run: RunId) -> sagat_application::telemetry::Derived {
        use sagat_application::telemetry::{TelemetryStore, derive};
        let pending = self
            .api
            .executions
            .pending_runs(1000)
            .await
            .expect("pending telemetry")
            .into_iter()
            .find(|pending| pending.run_id == run)
            .expect("run pending telemetry");
        let projected = self
            .api
            .executions
            .load_for_projection(&pending)
            .await
            .expect("load telemetry projection")
            .expect("traceable run");
        derive(&projected.facts, &projected.entries, projected.applied_seq)
    }

    /// The trace id the run was started under.
    pub async fn trace_of(&self, run: RunId) -> String {
        sqlx::query_scalar("SELECT trace_id FROM runs WHERE id = $1")
            .bind(run.as_uuid())
            .fetch_one(&self.api.db.pool)
            .await
            .expect("trace id")
    }

    /// The status of a root step of the run, if it exists.
    pub async fn step_status(&self, run: RunId, node: &str) -> Option<String> {
        sqlx::query_scalar(
            "SELECT status FROM steps WHERE run_id = $1 AND node = $2 \
             AND instance_path_nodes = '{}'",
        )
        .bind(run.as_uuid())
        .bind(node)
        .fetch_optional(&self.api.db.pool)
        .await
        .expect("step status")
    }

    /// The types of the run's journal entries, in order.
    pub async fn journal(&self, run: RunId) -> Vec<String> {
        sqlx::query_scalar("SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY seq")
            .bind(run.as_uuid())
            .fetch_all(&self.api.db.pool)
            .await
            .expect("journal")
    }

    pub async fn verify(&self, run: RunId) {
        self.api
            .executions
            .verify_run(self.tenant, run)
            .await
            .expect("the journal rebuilds the projection");
    }

    /// Everything the worker can lease now.
    pub async fn poll(&self) -> Vec<TaskLease> {
        let mut worker = self.worker.clone();
        worker
            .poll_tasks(approval_worker_request(
                &self.api,
                PollTasksRequest {
                    worker_id: self.worker_id.clone(),
                    session_token: self.session_token.clone(),
                    max_tasks: 16,
                },
            ))
            .await
            .expect("poll tasks")
            .into_inner()
            .tasks
    }

    pub async fn complete(&self, task: &TaskLease, output: Value) {
        let mut worker = self.worker.clone();
        let response = worker
            .complete_step(approval_worker_request(
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

    /// Leases the task of `run` that is ready, completes it with `output` and lets the scheduler
    /// follow. Tasks of other runs stay leased until their lease lapses.
    pub async fn work_on(&self, run: RunId, output: Value) -> TaskLease {
        self.drive().await;
        let mut tasks: Vec<TaskLease> = self
            .poll()
            .await
            .into_iter()
            .filter(|task| task.run_id == run.to_string())
            .collect();
        assert_eq!(tasks.len(), 1, "{tasks:?}");
        let task = tasks.remove(0);
        self.complete(&task, output).await;
        self.drive().await;
        task
    }

    /// Leases the one task that is ready, completes it with `output` and lets the scheduler follow.
    pub async fn work(&self, output: Value) -> TaskLease {
        let mut tasks = self.poll().await;
        assert_eq!(tasks.len(), 1, "{tasks:?}");
        let task = tasks.remove(0);
        self.complete(&task, output).await;
        self.drive().await;
        task
    }
}

fn approval_worker_request<T>(api: &TestApi, message: T) -> tonic::Request<T> {
    let token = api.idp.token("approval-worker", "acme").build();
    let mut request = tonic::Request::new(message);
    request.metadata_mut().insert(
        "authorization",
        tonic::metadata::MetadataValue::try_from(format!("Bearer {token}"))
            .expect("worker authorization"),
    );
    request
}

/// Whether the refusal names `reason` in `details.reason`.
#[must_use]
pub fn reason_of(body: &Value) -> &str {
    body["details"]["reason"].as_str().unwrap_or("")
}

/// `work` (an activity) then `review` (an approval): an approval rolls the chain over to the next
/// segment, a rejection or an expiry ends it. The segment limit is `max_segments`.
#[must_use]
pub fn approval_chain(name: &str, timeout_ms: u64, max_segments: u32) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": {"name": name},
        "entrypoint": "work",
        "nodes": [
            {"id": "work", "type": "activity", "activity": "segment.work", "next": "review"},
            {
                "id": "review",
                "type": "approval",
                "timeoutMs": timeout_ms,
                "policy": {"roles": ["admin", "operator"], "excludeInitiator": true},
                "onApprove": "rollover",
                "onReject": "rejected",
                "onExpire": "expired"
            },
            {"id": "rollover", "type": "continueAsNew", "input": "input.next"},
            {"id": "rejected", "type": "end"},
            {"id": "expired", "type": "end"}
        ],
        "chainPolicy": {"maxSegments": max_segments, "timeoutMs": 3_600_000}
    })
}

/// `approval_chain` that also declares the balance state and transfers it exactly to the successor.
#[must_use]
pub fn approval_chain_with_state(name: &str, timeout_ms: u64, max_segments: u32) -> Value {
    let mut document = approval_chain(name, timeout_ms, max_segments);
    document["state"] = state_block();
    document["nodes"][2]["state"] = json!("state");
    document
}
