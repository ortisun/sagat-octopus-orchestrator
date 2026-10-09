//! A complete execution plane over a real PostgreSQL, for the feature 002 suites.
//!
//! Services are the production ones; only time is manipulated, through the owner connection:
//! `age_*` pull a timestamp into the past so a timer, lease or deadline is due *now*, which keeps
//! the tests deterministic without sleeping.

use std::sync::Arc;

use sagat_application::clock::SystemClock;
use sagat_application::definitions::DefinitionService;
use sagat_application::entropy::SystemEntropy;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::identity::{IdentityStore, RequestContext};
use sagat_application::maintenance::OutboxStore;
use sagat_application::runs::{RunFilter, RunOperation, RunService, RunStore, StartRun};
use sagat_application::worker::{
    Capabilities, ClaimedTask, StepAck, TaskRef, WorkerService, WorkerSettings,
};
use sagat_domain::audit::{AuditAction, AuditRecord, ResourceType};
use sagat_domain::definition::{WorkflowId, WorkflowVersionId};
use sagat_domain::execution::{RunId, RunStatus, StepStatus};
use sagat_domain::identity::{
    Environment, Namespace, NamespaceId, Principal, Role, Subject, TenantId, Workspace, WorkspaceId,
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::PgRunStore;
use sagat_scheduler::{Scheduler, SchedulerSettings};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{TestDb, migrated_db, now, slug, subject, two_tenants};

pub type Runs = RunService<PgRunStore, SystemClock, JsonEvaluator>;
pub type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;
pub type Defs = DefinitionService<PgRunStore, SystemClock, JsonIrCodec>;
pub type Sched = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;

pub struct World {
    pub db: TestDb,
    pub store: PgRunStore,
    pub defs: Defs,
    pub runs: Runs,
    pub workers: Workers,
    pub scheduler: Sched,
    /// Administrator of the first tenant (can do everything a human can).
    pub admin: Principal,
    /// A machine identity of the first tenant.
    pub worker: Principal,
    /// Administrator of the second tenant.
    pub other_admin: Principal,
    pub namespace: Namespace,
    pub other_namespace: Namespace,
}

pub fn ctx() -> RequestContext {
    RequestContext {
        trace_id: Some("0af7651916cd43dd8448eb211c80319c".into()),
    }
}

pub async fn world() -> World {
    world_with(WorkerSettings::default()).await
}

pub async fn world_with(settings: WorkerSettings) -> World {
    // `RUST_LOG=debug cargo test ...` shows the causes the API deliberately hides from clients.
    static LOGGING: std::sync::Once = std::sync::Once::new();
    LOGGING.call_once(|| {
        if std::env::var_os("RUST_LOG").is_some() {
            sagat_telemetry::init_logging(sagat_telemetry::LogFormat::Pretty);
        }
    });
    let db = migrated_db().await;
    let t = two_tenants(&db).await;
    let namespace = namespace_in(&t.store, &t.admin_a, "payments").await;
    let other_namespace = namespace_in(&t.store, &t.admin_b, "payments").await;
    let codec = Arc::new(JsonIrCodec);
    let cache = DefinitionCache::new(codec, 64);
    let store = PgRunStore::new(db.pool.clone(), cache.clone());
    let engine = Engine::new(JsonEvaluator);
    World {
        defs: DefinitionService::new(store.clone(), SystemClock, JsonIrCodec),
        runs: RunService::new(store.clone(), SystemClock, engine.clone(), cache.clone()),
        workers: WorkerService::new(
            store.clone(),
            SystemClock,
            engine.clone(),
            SystemEntropy,
            settings,
        ),
        scheduler: Scheduler::new(
            store.clone(),
            engine,
            SystemEntropy,
            SchedulerSettings::default(),
        )
        .with_children(cache),
        worker: Principal::new(t.admin_a.tenant_id(), subject("worker-1"), Role::Worker),
        admin: t.admin_a,
        other_admin: t.admin_b,
        namespace,
        other_namespace,
        store,
        db,
    }
}

async fn namespace_in(
    identity: &sagat_infra_postgres::PgIdentityStore,
    p: &Principal,
    name: &str,
) -> Namespace {
    let ws = Workspace::create(
        p,
        WorkspaceId::generate(),
        slug(&format!("ws-{name}")),
        now(),
    )
    .expect("workspace");
    identity
        .create_workspace(
            &ws,
            &AuditRecord::by(
                p,
                AuditAction::WorkspaceCreated,
                ResourceType::Workspace,
                ws.id().to_string(),
                None,
                now(),
            ),
        )
        .await
        .expect("persist workspace");
    let ns = Namespace::create(
        p,
        &ws,
        NamespaceId::generate(),
        slug(name),
        Environment::Dev,
        now(),
    )
    .expect("namespace");
    identity
        .create_namespace(
            &ns,
            &AuditRecord::by(
                p,
                AuditAction::NamespaceCreated,
                ResourceType::Namespace,
                ns.id().to_string(),
                None,
                now(),
            ),
        )
        .await
        .expect("persist namespace");
    ns
}

// ------------------------------------------------------------------------------ documents

fn base(name: &str, entrypoint: &str, nodes: Vec<Value>) -> Value {
    json!({
        "apiVersion": "sagat-octopus-orchestrator.io/v1",
        "kind": "Workflow",
        "metadata": { "name": name },
        "entrypoint": entrypoint,
        "nodes": nodes,
    })
}

pub fn activity(id: &str) -> Value {
    json!({ "id": id, "type": "activity", "activity": format!("act.{id}") })
}

/// An activity with `attempts` attempts and a short fixed backoff.
pub fn flaky(id: &str, attempts: u32) -> Value {
    json!({
        "id": id, "type": "activity", "activity": format!("act.{id}"),
        "retry": { "maxAttempts": attempts, "initialDelayMs": 100, "maxDelayMs": 100, "strategy": "fixed", "jitter": false }
    })
}

pub fn with(mut node: Value, key: &str, value: Value) -> Value {
    node[key] = value;
    node
}

pub fn document(name: &str, entrypoint: &str, nodes: Vec<Value>) -> Value {
    base(name, entrypoint, nodes)
}

pub fn timer_node(id: &str, millis: u64) -> Value {
    json!({ "id": id, "type": "timer", "durationMs": millis })
}

pub fn wait_node(
    id: &str,
    event_type: &str,
    key: &str,
    timeout_ms: u64,
    on_timeout: Option<&str>,
) -> Value {
    let mut node = json!({
        "id": id, "type": "waitEvent", "eventType": event_type,
        "correlationExpr": key, "timeoutMs": timeout_ms
    });
    if let Some(target) = on_timeout {
        node["onTimeout"] = json!(target);
    }
    node
}

/// A `child` node starting the workflow `workflow`.
pub fn child_node(id: &str, workflow: &str, next: Option<&str>) -> Value {
    let mut node = json!({ "id": id, "type": "child", "workflowId": workflow });
    if let Some(next) = next {
        node["next"] = json!(next);
    }
    node
}

pub fn condition_node(id: &str, expression: &str, if_true: &str, if_false: &str) -> Value {
    json!({ "id": id, "type": "condition", "expression": expression, "ifTrue": if_true, "ifFalse": if_false })
}

pub fn parallel_node(id: &str, branches: &[&str], join: &str) -> Value {
    json!({ "id": id, "type": "parallel", "branches": branches, "join": join })
}

pub fn end_node(id: &str) -> Value {
    json!({ "id": id, "type": "end" })
}

/// `a -> b`.
pub fn chain(name: &str) -> Value {
    document(
        name,
        "a",
        vec![with(activity("a"), "next", json!("b")), activity("b")],
    )
}

// ------------------------------------------------------------------------------ operations

impl World {
    pub async fn publish(&self, doc: Value) -> (WorkflowId, WorkflowVersionId) {
        let name = doc["metadata"]["name"].as_str().expect("name").to_owned();
        let bytes = serde_json::to_vec(&doc).expect("json");
        let key = format!("create-{}", Uuid::now_v7());
        let workflow = self
            .defs
            .create_workflow(&self.admin, "payments", &name, &bytes, &key, &ctx())
            .await
            .expect("create workflow");
        let id = workflow.workflow.id();
        let published = self
            .defs
            .publish(
                &self.admin,
                id,
                workflow.workflow.draft_revision(),
                None,
                &format!("publish-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("publish");
        (id, published.version.id())
    }

    /// Publishes `doc` as the next version of an existing workflow.
    pub async fn publish_new_version(&self, workflow: WorkflowId, doc: Value) -> WorkflowVersionId {
        let bytes = serde_json::to_vec(&doc).expect("json");
        let record = self
            .defs
            .get_workflow(&self.admin, workflow, &ctx())
            .await
            .expect("get workflow");
        let updated = self
            .defs
            .update_draft(
                &self.admin,
                workflow,
                record.workflow.draft_revision(),
                &bytes,
                &ctx(),
            )
            .await
            .expect("update draft");
        self.defs
            .publish(
                &self.admin,
                workflow,
                updated.workflow.draft_revision(),
                None,
                &format!("publish-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("publish")
            .version
            .id()
    }

    /// Creates a workflow but does not publish it.
    pub async fn create_draft(&self, doc: Value) -> WorkflowId {
        let name = doc["metadata"]["name"].as_str().expect("name").to_owned();
        let bytes = serde_json::to_vec(&doc).expect("json");
        self.defs
            .create_workflow(
                &self.admin,
                "payments",
                &name,
                &bytes,
                &format!("create-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("create workflow")
            .workflow
            .id()
    }

    /// Publishes `doc` in the other tenant's namespace.
    pub async fn publish_as_other_tenant(&self, doc: Value) -> WorkflowId {
        let name = doc["metadata"]["name"].as_str().expect("name").to_owned();
        let bytes = serde_json::to_vec(&doc).expect("json");
        let created = self
            .defs
            .create_workflow(
                &self.other_admin,
                "payments",
                &name,
                &bytes,
                &format!("create-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("create workflow");
        let id = created.workflow.id();
        self.defs
            .publish(
                &self.other_admin,
                id,
                created.workflow.draft_revision(),
                None,
                &format!("publish-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
            .expect("publish");
        id
    }

    /// The stored record of a run (any run of the first tenant).
    pub async fn run_record(&self, run: RunId) -> Option<sagat_application::execution::RunRecord> {
        self.store
            .get(self.admin.tenant_id(), run)
            .await
            .expect("get run")
    }

    pub async fn start(&self, workflow: WorkflowId, input: &Value) -> RunId {
        self.start_with(workflow, input, &format!("start-{}", Uuid::now_v7()), None)
            .await
            .expect("start run")
    }

    pub async fn start_with(
        &self,
        workflow: WorkflowId,
        input: &Value,
        key: &str,
        deadline: Option<time::OffsetDateTime>,
    ) -> Result<RunId, sagat_application::error::AppError> {
        let bytes = serde_json::to_vec(input).expect("json");
        self.runs
            .start(
                &self.admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: &bytes,
                    correlation_id: None,
                    deadline,
                    key,
                },
                &ctx(),
            )
            .await
            .map(|r| r.info.run_id)
    }

    /// Registers a worker for `activities` and returns its session.
    pub async fn session(&self, activities: &[&str]) -> Session {
        self.session_as(&self.worker, activities, 100).await
    }

    pub async fn session_as(
        &self,
        principal: &Principal,
        activities: &[&str],
        max_inflight: u32,
    ) -> Session {
        let caps = Capabilities {
            activities: activities.iter().map(|a| (*a).to_owned()).collect(),
            languages: vec!["rust".into()],
            pools: vec![],
            max_inflight,
            protocol_version: "1".into(),
        };
        let registered = self
            .workers
            // Each session is a worker of its own: registering the same name again replaces the
            // previous session (a worker that reconnects), which is not what a pool of workers is.
            .register(principal, "payments", &unique_worker_name(), &caps)
            .await
            .expect("register");
        Session {
            principal: principal.clone(),
            worker_id: registered.worker_id,
            token: registered.session_token.expose().to_owned(),
        }
    }

    pub async fn poll(&self, session: &Session, max: u32) -> Vec<ClaimedTask> {
        self.workers
            .poll(
                &session.principal,
                session.worker_id,
                &session.token,
                max,
                Some("0af7651916cd43dd8448eb211c80319c"),
            )
            .await
            .expect("poll")
    }

    /// Polls once and returns the single task (panics if there is not exactly one).
    pub async fn one_task(&self, session: &Session) -> ClaimedTask {
        let mut tasks = self.poll(session, 8).await;
        assert_eq!(
            tasks.len(),
            1,
            "expected exactly one task, got {}",
            tasks.len()
        );
        tasks.remove(0)
    }

    pub async fn complete(
        &self,
        session: &Session,
        task: &ClaimedTask,
        output: &Value,
    ) -> Result<StepAck, sagat_application::error::AppError> {
        self.workers
            .complete(
                &session.principal,
                &task_ref(task),
                &serde_json::to_vec(output).expect("json"),
                &format!("done-{}", task.task_id()),
            )
            .await
    }

    pub async fn fail(
        &self,
        session: &Session,
        task: &ClaimedTask,
        retryable: bool,
    ) -> Result<StepAck, sagat_application::error::AppError> {
        self.workers
            .fail(
                &session.principal,
                &task_ref(task),
                "PAYMENT_DECLINED",
                retryable,
                &format!("failed-{}", task.task_id()),
            )
            .await
    }

    pub async fn run_status(&self, run: RunId) -> RunStatus {
        self.store
            .get(self.admin.tenant_id(), run)
            .await
            .expect("get")
            .expect("exists")
            .status
    }

    pub async fn step_status(&self, run: RunId, node: &str) -> Option<StepStatus> {
        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM steps WHERE run_id = $1 AND node = $2")
                .bind(run.as_uuid())
                .bind(node)
                .fetch_optional(&self.db.pool)
                .await
                .expect("query");
        status.map(|s| s.parse().expect("status"))
    }

    pub async fn journal(&self, run: RunId) -> Vec<String> {
        self.store
            .journal(self.admin.tenant_id(), run, None, 500)
            .await
            .expect("journal")
            .expect("run exists")
            .into_iter()
            .map(|e| e.event_type)
            .collect()
    }

    /// The projections must equal a replay of the journal.
    pub async fn assert_consistent(&self, run: RunId) {
        self.store
            .verify_run(self.admin.tenant_id(), run)
            .await
            .unwrap_or_else(|why| panic!("run {run} is inconsistent: {why}"));
    }

    pub async fn operate(
        &self,
        run: RunId,
        op: RunOperation,
    ) -> Result<sagat_application::execution::RunRecord, sagat_application::error::AppError> {
        self.runs
            .operate(
                &self.admin,
                run,
                op,
                "operator decision for the test",
                None,
                &format!("op-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
    }

    /// Cancels, pauses or resumes any run of the first tenant (children included).
    pub async fn operate_run(&self, run: RunId, op: RunOperation) {
        self.operate(run, op).await.expect("operation");
    }

    pub async fn runs_of(&self, filter: &RunFilter) -> Vec<RunId> {
        self.store
            .list(
                self.admin.tenant_id(),
                self.namespace.id(),
                filter,
                None,
                100,
            )
            .await
            .expect("list")
            .into_iter()
            .map(|r| r.info.run_id)
            .collect()
    }

    // ---- time travel through the owner connection

    pub async fn age_lease(&self, run: RunId, node: &str) {
        exec(&self.db.pool, "UPDATE steps SET lease_expires_at = clock_timestamp() - interval '1 second' WHERE run_id = $1 AND node = $2", run, node).await;
    }

    pub async fn age_attempt_deadline(&self, run: RunId, node: &str) {
        exec(&self.db.pool, "UPDATE steps SET attempt_deadline_at = clock_timestamp() - interval '1 second' WHERE run_id = $1 AND node = $2", run, node).await;
    }

    pub async fn age_due(&self, run: RunId, node: &str) {
        exec(&self.db.pool, "UPDATE steps SET due_at = clock_timestamp() - interval '1 second' WHERE run_id = $1 AND node = $2", run, node).await;
    }

    pub async fn age_retry(&self, run: RunId, node: &str) {
        exec(&self.db.pool, "UPDATE steps SET next_attempt_at = clock_timestamp() - interval '1 second' WHERE run_id = $1 AND node = $2", run, node).await;
    }

    pub async fn age_run_deadline(&self, run: RunId) {
        sqlx::query(
            "UPDATE runs SET deadline_at = clock_timestamp() - interval '1 second' WHERE id = $1",
        )
        .bind(run.as_uuid())
        .execute(&self.db.pool)
        .await
        .expect("age deadline");
    }

    pub async fn age_hint(&self, run: RunId, node: &str) {
        exec(&self.db.pool, "UPDATE steps SET last_hinted_at = clock_timestamp() - interval '1 hour' WHERE run_id = $1 AND node = $2", run, node).await;
    }

    pub async fn outbox(&self, event_type: &str) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM event_outbox WHERE event_type = $1")
            .bind(event_type)
            .fetch_one(&self.db.pool)
            .await
            .expect("count")
    }

    pub async fn backlog(&self) -> u64 {
        self.store.outbox_backlog().await.expect("backlog")
    }
}

async fn exec(pool: &PgPool, sql: &'static str, run: RunId, node: &str) {
    let done = sqlx::query(sql)
        .bind(run.as_uuid())
        .bind(node)
        .execute(pool)
        .await
        .expect("age");
    assert_eq!(done.rows_affected(), 1, "no such step to age");
}

pub struct Session {
    pub principal: Principal,
    pub worker_id: Uuid,
    pub token: String,
}

pub fn task_ref(task: &ClaimedTask) -> TaskRef<'_> {
    TaskRef {
        run_id: task.run_id,
        node: &task.node,
        step_id: Some(task.step_id),
        fence: task.fence,
        lease_token: task.lease_token.expose(),
    }
}

pub fn tenant_of(world: &World) -> TenantId {
    world.admin.tenant_id()
}

pub fn subject_of(name: &str) -> Subject {
    subject(name)
}

// ------------------------------------------------------------------------------ the broker

use std::sync::Mutex;

use sagat_application::maintenance::{EventPublisher, PublishError};

/// A broker that records what it was given and can be switched off. Clones share the record.
#[derive(Clone, Default)]
pub struct RecordingBroker {
    published: std::sync::Arc<Mutex<Vec<Published>>>,
    down: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Published {
    pub subject: String,
    pub message_id: String,
    pub payload: Vec<u8>,
}

impl RecordingBroker {
    pub fn set_down(&self, down: bool) {
        self.down.store(down, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn published(&self) -> Vec<Published> {
        self.published.lock().expect("lock").clone()
    }
}

impl EventPublisher for RecordingBroker {
    async fn publish(
        &self,
        subject: &str,
        message_id: &str,
        payload: &[u8],
    ) -> Result<(), PublishError> {
        if self.down.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(PublishError::Unavailable);
        }
        self.published.lock().expect("lock").push(Published {
            subject: subject.to_owned(),
            message_id: message_id.to_owned(),
            payload: payload.to_vec(),
        });
        Ok(())
    }
}

/// A worker name no other session of this test run has used.
fn unique_worker_name() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!("test-worker-{}", NEXT.fetch_add(1, Ordering::Relaxed))
}
