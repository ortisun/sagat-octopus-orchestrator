//! The platform as a worker sees it: definitions and runs are managed over REST by a human, the
//! worker protocol is spoken over gRPC by a machine identity. Both servers share one database and
//! one identity provider, exactly as the deployed processes do.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use sagat_api_grpc::pb::{
    ReadBlobRequest, WriteBlobHeader, WriteBlobRequest, WriteBlobResponse, read_blob_response,
    write_blob_request,
};
use std::time::Duration;

use reqwest::Method;
use sagat_api_grpc::pb::worker_service_client::WorkerServiceClient;
use sagat_api_grpc::pb::{
    CompleteStepRequest, FailStepRequest, HeartbeatRequest, PollTasksRequest,
    RegisterWorkerRequest, TaskLease, WorkerCapabilities,
};
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_domain::identity::{Membership, Role, TenantId};
use sagat_infra_ir::JsonEvaluator;
use sagat_scheduler::{Report, Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use std::sync::Arc;
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

    /// A REST call as `sub` of acme (any role), in the `payments` namespace, with a fresh
    /// idempotency key.
    pub async fn rest_as(
        &self,
        sub: &str,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> (u16, Value) {
        let mut request = self
            .api
            .as_user(method, path, sub, TENANT)
            .header("x-octopus-namespace", NAMESPACE)
            .header("idempotency-key", format!("key-{}", Uuid::now_v7()));
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await.expect("send");
        let status = response.status().as_u16();
        let text = response.text().await.expect("body");
        (status, serde_json::from_str(&text).unwrap_or(Value::Null))
    }

    /// The id of acme's `payments` namespace.
    pub async fn namespace_id(&self) -> String {
        let (status, listed) = self.rest(Method::GET, "/namespaces", None).await;
        assert_eq!(status, 200, "{listed}");
        listed["items"]
            .as_array()
            .expect("items")
            .iter()
            .find(|n| n["slug"] == NAMESPACE)
            .and_then(|n| n["id"].as_str())
            .expect("the namespace")
            .to_owned()
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

    /// Outbox messages not yet delivered.
    pub async fn outbox_backlog(&self) -> u64 {
        use sagat_application::maintenance::OutboxStore;
        self.api.executions.outbox_backlog().await.expect("backlog")
    }

    /// One pass of the scheduler over this platform's database (what the engine's loop does).
    pub async fn tick(&self) -> Report {
        Scheduler::new(
            self.api.executions.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            SchedulerSettings::default(),
        )
        .with_children(DefinitionCache::new(
            Arc::new(sagat_infra_ir::JsonIrCodec),
            64,
        ))
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

    /// Binds `sub` to every activity of acme's namespace, as an administrator would before starting
    /// a fleet (a worker without a binding is refused at registration).
    pub async fn bind_everything(&self, sub: &str) {
        sqlx::query(
            "INSERT INTO worker_bindings (tenant_id, namespace_id, subject, pattern, updated_by) \
             SELECT n.tenant_id, n.id, $2, '*', 'test' FROM namespaces n \
             WHERE n.tenant_id = $1 AND n.slug = $3 ON CONFLICT DO NOTHING",
        )
        .bind(self.tenant.as_uuid())
        .bind(sub)
        .bind(NAMESPACE)
        .execute(&self.api.db.pool)
        .await
        .expect("bind the worker");
    }

    /// A worker identity of acme that has connected but not registered yet.
    pub async fn worker(&self, sub: &str) -> Worker<'_> {
        self.grant(self.tenant, sub, Role::Worker).await;
        self.bind_everything(sub).await;
        self.connect(sub, TENANT).await
    }

    /// A client authenticating as `sub` of `tenant`, whatever its role (or lack of one).
    pub async fn connect(&self, sub: &str, tenant: &str) -> Worker<'_> {
        self.connect_at(self.api.grpc_url.clone(), sub, tenant)
            .await
    }

    /// A worker identity of acme speaking to the worker service at `url` (an engine process).
    pub async fn worker_at(&self, url: &str, sub: &str) -> Worker<'_> {
        self.grant(self.tenant, sub, Role::Worker).await;
        self.bind_everything(sub).await;
        self.connect_at(url.to_owned(), sub, TENANT).await
    }

    pub async fn connect_at(&self, url: String, sub: &str, tenant: &str) -> Worker<'_> {
        let client = WorkerServiceClient::connect(url)
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
        self.register_version(activities, "1").await;
    }

    /// Registers at an explicit protocol version and remembers the negotiated session.
    pub async fn register_version(&mut self, activities: &[&str], protocol_version: &str) {
        let mut message = Self::register_message(activities, 100);
        message
            .capabilities
            .as_mut()
            .expect("capabilities")
            .protocol_version = protocol_version.to_owned();
        let request = self.req(message);
        let response = self
            .client
            .register_worker(request)
            .await
            .expect("register")
            .into_inner();
        assert_eq!(
            response.accepted_protocol_version,
            sagat_application::worker::negotiate_protocol(protocol_version)
                .expect("a supported protocol version")
        );
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

    pub fn resolve_secrets_message(lease: &TaskLease) -> sagat_api_grpc::pb::ResolveSecretsRequest {
        sagat_api_grpc::pb::ResolveSecretsRequest {
            task_id: lease.task_id.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            traceparent: String::new(),
        }
    }

    /// Asks for the secrets the leased step declares.
    pub async fn resolve_secrets(
        &mut self,
        lease: &TaskLease,
    ) -> Result<Vec<sagat_api_grpc::pb::ResolvedSecret>, Status> {
        let request = self.req(Self::resolve_secrets_message(lease));
        self.client
            .resolve_secrets(request)
            .await
            .map(|r| r.into_inner().secrets)
    }

    /// Uploads `data` as a blob, in chunks of `chunk` bytes, for the attempt `lease` names.
    pub async fn write_blob(
        &mut self,
        lease: &TaskLease,
        header: WriteBlobHeader,
        data: &[u8],
        chunk: usize,
    ) -> Result<WriteBlobResponse, Status> {
        let header = WriteBlobHeader {
            task_id: lease.task_id.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            ..header
        };
        let mut messages = vec![WriteBlobRequest {
            part: Some(write_blob_request::Part::Header(header)),
        }];
        messages.extend(data.chunks(chunk.max(1)).map(|c| WriteBlobRequest {
            part: Some(write_blob_request::Part::Chunk(
                bytes::Bytes::copy_from_slice(c),
            )),
        }));
        self.write_blob_messages(messages).await
    }

    /// Uploads exactly these messages, for the tests about a malformed stream.
    pub async fn write_blob_messages(
        &mut self,
        messages: Vec<WriteBlobRequest>,
    ) -> Result<WriteBlobResponse, Status> {
        let request = self.req(tokio_stream::iter(messages));
        self.client
            .write_blob(request)
            .await
            .map(tonic::Response::into_inner)
    }

    pub fn read_blob_message(lease: &TaskLease, blob_id: &str) -> ReadBlobRequest {
        ReadBlobRequest {
            task_id: lease.task_id.clone(),
            run_id: lease.run_id.clone(),
            step_id: lease.step_id.clone(),
            attempt: lease.attempt,
            generation: lease.generation,
            lease_token: lease.lease_token.clone(),
            blob_id: blob_id.to_owned(),
        }
    }

    /// Downloads a blob whole: its header and its content. A stream that ends with an error (for
    /// content that failed verification) is that error.
    pub async fn read_blob(
        &mut self,
        lease: &TaskLease,
        blob_id: &str,
    ) -> Result<(sagat_api_grpc::pb::ReadBlobHeader, Vec<u8>), Status> {
        let request = self.req(Self::read_blob_message(lease, blob_id));
        let mut stream = self.client.read_blob(request).await?.into_inner();
        let mut header = None;
        let mut content = Vec::new();
        while let Some(message) = stream.message().await? {
            match message.part {
                Some(read_blob_response::Part::Header(h)) => {
                    assert!(header.is_none(), "one header, first");
                    header = Some(h);
                }
                Some(read_blob_response::Part::Chunk(c)) => {
                    assert!(header.is_some(), "the header comes first");
                    assert!(
                        c.len() <= sagat_api_grpc::BLOB_CHUNK_BYTES,
                        "chunks are bounded"
                    );
                    content.extend_from_slice(&c);
                }
                None => panic!("an empty message"),
            }
        }
        Ok((header.expect("a header"), content))
    }

    /// Completes the step with a blob as its output.
    pub async fn complete_with_blob(
        &mut self,
        lease: &TaskLease,
        blob_id: &str,
    ) -> Result<sagat_api_grpc::pb::StepCommandResponse, Status> {
        let message = CompleteStepRequest {
            output_json: String::new(),
            output_blob_ref: blob_id.to_owned(),
            ..Self::complete_message(lease, "")
        };
        let request = self.req(message);
        self.client
            .complete_step(request)
            .await
            .map(tonic::Response::into_inner)
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
        self.fail_with(lease, "PAYMENT_DECLINED", retryable).await
    }

    /// A failure with a code of the worker's choosing (`OUTCOME_UNKNOWN` is the reserved one).
    pub async fn fail_with(
        &mut self,
        lease: &TaskLease,
        code: &str,
        retryable: bool,
    ) -> Result<sagat_api_grpc::pb::StepCommandResponse, Status> {
        let request = self.req(Self::fail_message(lease, code, retryable));
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

// ------------------------------------------------------------------------------ the engine

use std::collections::HashMap;
use std::net::SocketAddr;

use sagat_control_plane::RunError;
use sagat_control_plane::config::Config;
use sagat_execution_engine::EngineConfig;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use super::TestDb;
use super::oidc::{AUDIENCE, ISSUER, TestIdp};

/// A real `execution-engine` (worker API, scheduler loop, outbox dispatcher, probes) serving a
/// test database, started the way the binary starts it: from environment variables.
pub struct RunningEngine {
    pub grpc_url: String,
    pub admin: SocketAddr,
    stop: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), RunError>>>,
}

/// Environment of an engine for `db`, with fast loops. `extra` overrides anything.
pub fn engine_env(db: &TestDb, idp: &TestIdp, extra: &[(&str, &str)]) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = [
        ("DATABASE_URL", db.url.as_str()),
        ("SAGAT_ENV", "dev"),
        ("SAGAT_OIDC_ISSUER", ISSUER),
        ("SAGAT_OIDC_AUDIENCE", AUDIENCE),
        ("SAGAT_OIDC_JWKS_URI", idp.jwks_url().as_str()),
        ("SAGAT_ALLOW_INSECURE_JWKS_HTTP", "true"),
        ("SAGAT_SCHEDULER_INTERVAL_MS", "50"),
        ("SAGAT_LIVENESS_GRACE_SECS", "1"),
        ("SAGAT_OUTBOX_POLL_MS", "50"),
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "1000"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "2000"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v.to_owned()))
    .collect();
    for (key, value) in extra {
        env.insert((*key).to_owned(), (*value).to_owned());
    }
    env
}

impl RunningEngine {
    pub async fn start(env: HashMap<String, String>) -> Self {
        Self::try_start(env).await.expect("the engine starts")
    }

    /// Starts the engine; an engine that refuses to start returns its error.
    pub async fn try_start(env: HashMap<String, String>) -> Result<Self, RunError> {
        let lookup = move |name: &str| env.get(name).cloned();
        let config = Config::from_lookup(&lookup)?;
        let engine = EngineConfig::from_lookup(&lookup, config.environment)?;
        Self::try_start_with(config, engine).await
    }

    /// As [`Self::try_start`] for settings the environment parser would not produce (production
    /// settings pointing at a test database without TLS).
    pub async fn try_start_with(config: Config, engine: EngineConfig) -> Result<Self, RunError> {
        let grpc = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind grpc");
        let admin = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind admin");
        let grpc_url = format!("http://{}", grpc.local_addr().expect("grpc addr"));
        let admin_addr = admin.local_addr().expect("admin addr");
        let (stop, stopped) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            sagat_execution_engine::serve(&config, &engine, grpc, Some(admin), async move {
                let _ = stopped.await;
            })
            .await
        });
        let mut running = Self {
            grpc_url,
            admin: admin_addr,
            stop: Some(stop),
            handle: Some(handle),
        };
        // Wait for the listeners to be served, or for the start-up to fail.
        for _ in 0..100 {
            if running.handle.as_ref().is_some_and(JoinHandle::is_finished) {
                return Err(running.finish().await.expect_err("it ended early"));
            }
            if running.status("/livez").await == Some(200) {
                return Ok(running);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        panic!("the engine never became alive");
    }

    /// Status code of `GET path` on the admin listener (`None` if it cannot be reached).
    pub async fn status(&self, path: &str) -> Option<u16> {
        reqwest::get(format!("http://{}{path}", self.admin))
            .await
            .ok()
            .map(|response| response.status().as_u16())
    }

    pub async fn body(&self, path: &str) -> String {
        reqwest::get(format!("http://{}{path}", self.admin))
            .await
            .expect("admin request")
            .text()
            .await
            .expect("text")
    }

    /// Asks the engine to stop and waits for it; returns what `serve` returned.
    pub async fn finish(&mut self) -> Result<(), RunError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let handle = self.handle.take().expect("already finished");
        tokio::time::timeout(Duration::from_secs(20), handle)
            .await
            .expect("the engine stops within its grace period")
            .expect("the engine task did not panic")
    }
}

impl Drop for RunningEngine {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// What the server tells the caller to do next (SC-002-004), from the trailing metadata.
pub fn remediation_of(status: &Status) -> String {
    status
        .metadata()
        .get("x-sagat-remediation")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}
