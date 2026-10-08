//! The platform as a worker sees it: definitions and runs are managed over REST by a human, the
//! worker protocol is spoken over gRPC by a machine identity. Both servers share one database and
//! one identity provider, exactly as the deployed processes do.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use reqwest::Method;
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, HeartbeatRequest, PollTasksRequest,
    RegisterWorkerRequest, TaskLease, WorkerCapabilities,
};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::Engine;
use sagat_domain::identity::{Membership, Role, TenantId};
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Report, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use tonic::metadata::MetadataValue;
use tonic::transport::Channel;
use tonic::{Request, Status};
use uuid::Uuid;

use super::api::{Options, TestApi, json_of, start_with};
use super::{now, subject};

pub const TENANT: &str = "acme";
pub const NAMESPACE: &str = "payments";
pub const HUMAN: &str = "alice";

pub struct Platform {
    pub api: TestApi,
    pub tenant: TenantId,
    pub other_tenant: TenantId,
}

pub async fn platform() -> Platform {
    platform_with(Options::default()).await
}

/// A tenant `acme` (admin `alice`, namespace `payments`) and a second tenant `globex` (admin `bob`).
pub async fn platform_with(options: Options) -> Platform {
    let api = start_with(options).await;
    let tenant = api.tenant(TENANT, HUMAN).await;
    let other_tenant = api.tenant("globex", "bob").await;
    let ws = api.create_workspace(HUMAN, TENANT, "platform").await;
    api.create_namespace(HUMAN, TENANT, &ws, NAMESPACE, "dev")
        .await;
    Platform {
        api,
        tenant,
        other_tenant,
    }
}

pub fn chain_ir(name: &str) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a",
        "nodes": [
            {"id": "a", "type": "activity", "activity": "act.a", "next": "b"},
            {"id": "b", "type": "activity", "activity": "act.b"}
        ]
    })
}

pub fn single_ir(name: &str, retry: Option<Value>) -> Value {
    let mut node = json!({"id": "a", "type": "activity", "activity": "act.a"});
    if let Some(retry) = retry {
        node["retry"] = retry;
    }
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1", "kind": "Workflow",
        "metadata": {"name": name}, "entrypoint": "a", "nodes": [node]
    })
}

impl Platform {
    /// A REST call as `alice` of acme, in the `payments` namespace, with a fresh idempotency key.
    pub async fn rest(&self, method: Method, path: &str, body: Option<&Value>) -> (u16, Value) {
        let mut request = self
            .api
            .as_user(method, path, HUMAN, TENANT)
            .header("x-octopus-namespace", NAMESPACE)
            .header("idempotency-key", format!("key-{}", Uuid::now_v7()));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("send");
        (response.status().as_u16(), json_of(response).await)
    }

    /// Creates and publishes a workflow; returns its id.
    pub async fn publish(&self, name: &str, ir: &Value) -> String {
        let (status, created) = self
            .rest(
                Method::POST,
                "/workflows",
                Some(&json!({"name": name, "ir": ir})),
            )
            .await;
        assert_eq!(status, 201, "{created}");
        let id = created["id"].as_str().expect("id").to_owned();
        let (status, published) = self
            .rest(
                Method::POST,
                &format!("/workflows/{id}/publish"),
                Some(&json!({"draftRevision": 1})),
            )
            .await;
        assert_eq!(status, 201, "{published}");
        id
    }

    pub async fn start(&self, workflow: &str, input: &Value) -> String {
        let (status, run) = self
            .rest(
                Method::POST,
                "/runs",
                Some(&json!({"workflowId": workflow, "input": input})),
            )
            .await;
        assert_eq!(status, 202, "{run}");
        run["runId"].as_str().expect("run id").to_owned()
    }

    pub async fn run(&self, run: &str) -> Value {
        let (status, body) = self.rest(Method::GET, &format!("/runs/{run}"), None).await;
        assert_eq!(status, 200, "{body}");
        body
    }

    pub async fn status(&self, run: &str) -> String {
        self.run(run).await["status"]
            .as_str()
            .expect("status")
            .to_owned()
    }

    pub async fn grant(&self, tenant: TenantId, sub: &str, role: Role) {
        self.api
            .owner_store
            .insert_membership(&Membership::restore(tenant, subject(sub), role, now()))
            .await
            .expect("grant");
    }

    /// One pass of the scheduler over this platform's database (what the engine's loop does).
    pub async fn tick(&self) -> Report {
        Scheduler::new(
            self.api.executions.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings::default(),
        )
        .tick()
        .await
    }

    /// Pretends the worker holding the lease went silent: the lease expired a second ago.
    pub async fn expire_lease(&self, run: &str, node: &str) {
        let done = sqlx::query(
            "UPDATE steps SET lease_expires_at = clock_timestamp() - interval '1 second' \
             WHERE run_id = $1 AND node = $2",
        )
        .bind(Uuid::parse_str(run).expect("run id"))
        .bind(node)
        .execute(&self.api.db.pool)
        .await
        .expect("age lease");
        assert_eq!(done.rows_affected(), 1, "no such step");
    }

    /// Makes a retry back-off elapse.
    pub async fn elapse_retry_delay(&self, run: &str, node: &str) {
        let done = sqlx::query(
            "UPDATE steps SET next_attempt_at = clock_timestamp() - interval '1 second' \
             WHERE run_id = $1 AND node = $2",
        )
        .bind(Uuid::parse_str(run).expect("run id"))
        .bind(node)
        .execute(&self.api.db.pool)
        .await
        .expect("age retry");
        assert_eq!(done.rows_affected(), 1, "no such step");
    }

    /// Makes a timer step's due time elapse.
    pub async fn elapse_timer(&self, run: &str, node: &str) {
        let done = sqlx::query(
            "UPDATE steps SET due_at = clock_timestamp() - interval '1 second' \
             WHERE run_id = $1 AND node = $2",
        )
        .bind(Uuid::parse_str(run).expect("run id"))
        .bind(node)
        .execute(&self.api.db.pool)
        .await
        .expect("age timer");
        assert_eq!(done.rows_affected(), 1, "no such step");
    }

    /// A worker identity of acme that has connected but not registered yet.
    pub async fn worker(&self, sub: &str) -> Worker<'_> {
        self.grant(self.tenant, sub, Role::Worker).await;
        self.connect(sub, TENANT).await
    }

    /// A client authenticating as `sub` of `tenant`, whatever its role (or lack of one).
    pub async fn connect(&self, sub: &str, tenant: &str) -> Worker<'_> {
        let client = WorkerServiceClient::connect(self.api.grpc_url.clone())
            .await
            .expect("connect to the worker service");
        Worker {
            platform: self,
            subject: sub.to_owned(),
            tenant: tenant.to_owned(),
            client,
            worker_id: String::new(),
            session: String::new(),
        }
    }
}

pub struct Worker<'a> {
    platform: &'a Platform,
    pub subject: String,
    pub tenant: String,
    pub client: WorkerServiceClient<Channel>,
    pub worker_id: String,
    pub session: String,
}

impl Worker<'_> {
    /// The message wrapped with a freshly signed bearer token.
    pub fn req<T>(&self, message: T) -> Request<T> {
        let token = self
            .platform
            .api
            .idp
            .token(&self.subject, &self.tenant)
            .build();
        self.req_with_token(message, &token)
    }

    pub fn req_with_token<T>(&self, message: T, token: &str) -> Request<T> {
        let mut request = Request::new(message);
        request.metadata_mut().insert(
            "authorization",
            MetadataValue::try_from(format!("Bearer {token}")).expect("ascii"),
        );
        request
    }

    pub fn capabilities(activities: &[&str], max_inflight: i32) -> WorkerCapabilities {
        WorkerCapabilities {
            languages: vec!["rust".into()],
            activities: activities.iter().map(|a| (*a).to_owned()).collect(),
            pools: vec![],
            max_inflight,
            protocol_version: "1".into(),
        }
    }

    pub fn register_message(activities: &[&str], max_inflight: i32) -> RegisterWorkerRequest {
        RegisterWorkerRequest {
            worker_name: "wire-test-worker".into(),
            namespace: NAMESPACE.into(),
            capabilities: Some(Self::capabilities(activities, max_inflight)),
        }
    }

    /// Registers and remembers the session.
    pub async fn register(&mut self, activities: &[&str]) {
        let request = self.req(Self::register_message(activities, 100));
        let response = self
            .client
            .register_worker(request)
            .await
            .expect("register")
            .into_inner();
        assert_eq!(response.accepted_protocol_version, "1");
        self.worker_id = response.worker_id;
        self.session = response.session_token;
    }

    pub fn poll_message(&self, max: i32) -> PollTasksRequest {
        PollTasksRequest {
            worker_id: self.worker_id.clone(),
            session_token: self.session.clone(),
            max_tasks: max,
        }
    }

    pub async fn poll(&mut self, max: i32) -> Vec<TaskLease> {
        let request = self.req(self.poll_message(max));
        self.client
            .poll_tasks(request)
            .await
            .expect("poll")
            .into_inner()
            .tasks
    }

    pub async fn one_task(&mut self) -> TaskLease {
        let mut tasks = self.poll(8).await;
        assert_eq!(tasks.len(), 1, "expected exactly one task");
        tasks.remove(0)
    }

    pub fn heartbeat_message(&self, lease: &TaskLease) -> HeartbeatRequest {
        HeartbeatRequest {
            task_id: lease.task_id.clone(),
            worker_id: self.worker_id.clone(),
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
        }
    }

    pub fn complete_message(lease: &TaskLease, output: &str) -> CompleteStepRequest {
        CompleteStepRequest {
            task_id: lease.task_id.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            idempotency_key: format!("done-{}", lease.task_id),
            output_json: output.to_owned(),
            output_blob_ref: String::new(),
            output_sha256: String::new(),
            traceparent: String::new(),
        }
    }

    pub fn fail_message(lease: &TaskLease, code: &str, retryable: bool) -> FailStepRequest {
        FailStepRequest {
            task_id: lease.task_id.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            idempotency_key: format!("failed-{}", lease.task_id),
            error_code: code.to_owned(),
            safe_message: "declined by the issuer".into(),
            retryable,
            traceparent: String::new(),
        }
    }

    pub async fn complete(
        &mut self,
        lease: &TaskLease,
        output: &Value,
    ) -> Result<sagat_api_grpc::pb::StepCommandResponse, Status> {
        let request = self.req(Self::complete_message(lease, &output.to_string()));
        self.client
            .complete_step(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub async fn fail(
        &mut self,
        lease: &TaskLease,
        retryable: bool,
    ) -> Result<sagat_api_grpc::pb::StepCommandResponse, Status> {
        let request = self.req(Self::fail_message(lease, "PAYMENT_DECLINED", retryable));
        self.client
            .fail_step(request)
            .await
            .map(tonic::Response::into_inner)
    }
}

/// The stable machine code the server put in the trailing metadata.
pub fn machine_code(status: &Status) -> String {
    status
        .metadata()
        .get("x-sagat-error")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}
