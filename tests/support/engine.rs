//! A complete execution plane over a real PostgreSQL, for the feature 002 suites.
//!
//! Services are the production ones; only time is manipulated, through the owner connection:
//! `age_*` pull a timestamp into the past so a timer, lease or deadline is due *now*, which keeps
//! the tests deterministic without sleeping.

use std::sync::Arc;

use sagat_application::bindings::BindingService;
use sagat_application::clock::SystemClock;
use sagat_application::connectors::ConnectorService;
use sagat_application::definitions::DefinitionService;
use sagat_application::egress::EgressService;
use sagat_application::entropy::SystemEntropy;
use sagat_application::events::EventService;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::identity::{IdentityStore, RequestContext};
use sagat_application::maintenance::OutboxStore;
use sagat_application::outbound::OutboundService;
use sagat_application::quotas::QuotaService;
use sagat_application::runs::{RunFilter, RunOperation, RunService, RunStore, StartRun};
use sagat_application::script_signing::Signer;
use sagat_application::scripts::{RunnerService, ScriptService};
use sagat_application::secrets::SecretService;
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
use sagat_infra_postgres::{PgRunStore, PoolConfig, connect};
use sagat_infra_schema::Schemas;
use sagat_scheduler::{
    DeliverySettings, DeliveryWorker, EventDispatchSettings, EventDispatcher, Scheduler,
    SchedulerSettings,
};
use serde_json::{Value, json};
use sqlx::PgPool;
use uuid::Uuid;

use super::{TestDb, migrated_db, now, slug, subject, two_tenants};

pub type Runs = RunService<PgRunStore, SystemClock, JsonEvaluator>;
pub type Workers = WorkerService<PgRunStore, SystemClock, JsonEvaluator, SystemEntropy>;
pub type Defs = DefinitionService<PgRunStore, SystemClock, JsonIrCodec>;
pub type Publishing =
    sagat_application::publishing::PublishingService<PgRunStore, SystemClock, JsonIrCodec>;
pub type Sched = Scheduler<PgRunStore, JsonEvaluator, SystemEntropy>;
pub type EventLoop = EventDispatcher<PgRunStore, JsonEvaluator, SystemEntropy>;
pub type Quotas = QuotaService<PgRunStore, SystemClock>;
pub type Bindings = BindingService<PgRunStore, SystemClock>;
pub type Secrets = SecretService<PgRunStore, SystemClock>;
pub type Egress = EgressService<PgRunStore, SystemClock>;
pub type Events = EventService<PgRunStore, SystemClock>;
pub type Outbound = OutboundService<PgRunStore, SystemClock>;
pub type Connectors = ConnectorService<PgRunStore, SystemClock>;
pub type Scripts = ScriptService<PgRunStore, SystemClock>;
pub type Runner = RunnerService<PgRunStore, SystemClock>;
pub type DeliveryLoop = DeliveryWorker<PgRunStore, super::receiver::LoopbackSender, SystemEntropy>;

pub struct World {
    pub db: TestDb,
    pub store: PgRunStore,
    pub defs: Defs,
    pub publishing: Publishing,
    pub runs: Runs,
    pub quotas: Quotas,
    pub bindings: Bindings,
    pub secrets: Secrets,
    pub egress: Egress,
    pub events: Events,
    pub outbound: Outbound,
    pub connectors: Connectors,
    pub scripts: Scripts,
    /// The runner of the sandbox, which signs what it is handed with a key made for this world.
    pub runner: Runner,
    /// The public half of the key behind [`Self::runner`].
    pub signing_key: Vec<u8>,
    /// The identity of the runner of the first tenant.
    pub runner_identity: Principal,
    /// The worker that sends outbound webhooks, to a local receiver (tests tick it by hand).
    pub deliveries: DeliveryLoop,
    /// The key ring behind [`Self::secrets`] and the workers (a fresh random key per world).
    pub keys: sagat_infra_secrets::SecretsConfig,
    /// The base64 material of key `k1`, for tests that build a rotated ring around it.
    pub key_k1: String,
    pub workers: Workers,
    pub scheduler: Sched,
    /// The dispatcher of admitted events (tests tick it by hand).
    pub dispatcher: EventLoop,
    /// The parsed definitions every service of this world shares.
    pub cache: DefinitionCache,
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
        span_id: None,
    }
}

pub async fn world() -> World {
    world_with(WorkerSettings::default()).await
}

pub async fn world_with(settings: WorkerSettings) -> World {
    world_with_breaker(
        settings,
        sagat_application::execution::BreakerPolicy::default(),
    )
    .await
}

/// A world whose compensation circuit breaker follows `breaker`.
pub async fn world_with_breaker(
    settings: WorkerSettings,
    breaker: sagat_application::execution::BreakerPolicy,
) -> World {
    // `RUST_LOG=debug cargo test ...` shows the causes the API deliberately hides from clients.
    static LOGGING: std::sync::Once = std::sync::Once::new();
    LOGGING.call_once(|| {
        if std::env::var_os("RUST_LOG").is_some() {
            sagat_telemetry::init_logging(sagat_telemetry::LogFormat::Pretty);
        }
    });
    let db = migrated_db().await;
    let t = two_tenants(&db).await;
    let namespace = namespace_in(&t.store, &t.admin_a, "payments", Environment::Dev).await;
    let other_namespace = namespace_in(&t.store, &t.admin_b, "payments", Environment::Dev).await;
    let codec = Arc::new(JsonIrCodec);
    let cache = DefinitionCache::new(codec, 64);
    // `SAGAT_TEST_AS_RUNTIME_ROLE=1` runs the services of the world (engine, workers, schedulers,
    // dispatchers) as the least-privilege runtime role, the role the engine is required to run as.
    let store_pool = if super::as_runtime_role() {
        db.runtime_pool_with(16).await
    } else {
        db.pool.clone()
    };
    let store = PgRunStore::new(store_pool, cache.clone()).with_breaker(breaker);
    let engine = Engine::new(JsonEvaluator);
    let key_k1 = sagat_infra_secrets::generate_key().expect("a key");
    let keys =
        sagat_infra_secrets::SecretsConfig::parse(&format!("k1:{key_k1}")).expect("a key ring");
    let signer =
        Signer::from_pkcs8(&Signer::generate_pkcs8().expect("a signing key")).expect("a signer");
    let signing_key = signer.public_key();
    let world = World {
        defs: DefinitionService::new(store.clone(), SystemClock, JsonIrCodec),
        publishing: Publishing::new(store.clone(), SystemClock, JsonIrCodec),
        runs: RunService::new(store.clone(), SystemClock, engine.clone(), cache.clone()),
        quotas: QuotaService::new(store.clone(), SystemClock),
        bindings: BindingService::new(store.clone(), SystemClock),
        secrets: SecretService::new(store.clone(), SystemClock, keys.cipher()),
        egress: EgressService::new(store.clone(), SystemClock),
        events: EventService::new(
            store.clone(),
            SystemClock,
            keys.cipher(),
            Arc::new(JsonEvaluator),
        ),
        outbound: OutboundService::new(
            store.clone(),
            SystemClock,
            EgressService::new(store.clone(), SystemClock),
        ),
        connectors: ConnectorService::new(
            store.clone(),
            SystemClock,
            EgressService::new(store.clone(), SystemClock),
            Arc::new(JsonEvaluator),
        ),
        scripts: ScriptService::new(
            store.clone(),
            SystemClock,
            Schemas,
            sagat_domain::scripts::Policy::default(),
        ),
        runner: RunnerService::new(
            store.clone(),
            SystemClock,
            SystemEntropy,
            signer,
            sagat_infra_schema::bundle::Bundles,
            sagat_domain::scripts::Policy::default(),
        ),
        signing_key,
        runner_identity: Principal::new(t.admin_a.tenant_id(), subject("runner-1"), Role::Runner),
        deliveries: DeliveryWorker::new(
            store.clone(),
            super::receiver::LoopbackSender::new(),
            SystemEntropy,
            keys.cipher(),
            DeliverySettings::default(),
        ),
        keys: keys.clone(),
        key_k1,
        workers: WorkerService::new(
            store.clone(),
            SystemClock,
            engine.clone(),
            SystemEntropy,
            settings,
        )
        .with_vault(keys.cipher()),
        scheduler: Scheduler::new(
            store.clone(),
            engine,
            SystemEntropy,
            SchedulerSettings::default(),
        )
        .with_children(cache.clone()),
        dispatcher: EventDispatcher::new(
            store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            cache.clone(),
            EventDispatchSettings::default(),
        )
        .with_cron(Arc::new(sagat_infra_ir::cron::CronCalendar)),
        cache,
        worker: Principal::new(t.admin_a.tenant_id(), subject("worker-1"), Role::Worker),
        admin: t.admin_a,
        other_admin: t.admin_b,
        namespace,
        other_namespace,
        store,
        db,
    };
    // The worker identity of the first tenant is bound to its namespace, as an operator would.
    world.bind_everything(&world.worker).await;
    world
}

async fn namespace_in(
    identity: &sagat_infra_postgres::PgIdentityStore,
    p: &Principal,
    name: &str,
    environment: Environment,
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
        environment,
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

/// A `saga` node whose body starts at `begin` and which continues at `next` when it completes.
pub fn saga_node(id: &str, begin: &str, next: Option<&str>) -> Value {
    let mut node = json!({ "id": id, "type": "saga", "begin": begin });
    if let Some(next) = next {
        node["next"] = json!(next);
    }
    node
}

/// An activity `act.<id>` with a compensation `undo.<id>` and, when asked, a reconciliation
/// `lookup.<id>`.
pub fn effect_node(id: &str, next: Option<&str>, reconcile: bool) -> Value {
    let mut node = with(activity(id), "compensateWith", json!(format!("undo.{id}")));
    if reconcile {
        node["reconcileWith"] = json!(format!("lookup.{id}"));
    }
    if let Some(next) = next {
        node["next"] = json!(next);
    }
    node
}

/// Every activity name `effect_node` can produce for `ids`: what a worker has to advertise.
pub fn effect_activities(ids: &[&str]) -> Vec<String> {
    ids.iter()
        .flat_map(|id| {
            [
                format!("act.{id}"),
                format!("undo.{id}"),
                format!("lookup.{id}"),
            ]
        })
        .collect()
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
        self.try_publish_as(&self.admin, doc)
            .await
            .expect("publish")
    }

    /// Creates the workflow and tries to publish it as `who`; the creation must succeed.
    pub async fn try_publish_as(
        &self,
        who: &Principal,
        doc: Value,
    ) -> Result<(WorkflowId, WorkflowVersionId), sagat_application::error::AppError> {
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
                who,
                id,
                workflow.workflow.draft_revision(),
                None,
                &format!("publish-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await?;
        Ok((id, published.version.id()))
    }

    /// A member of the first tenant with `role`, added the way an operator adds one.
    pub async fn member(&self, name: &str, role: Role) -> Principal {
        let tenant = self.admin.tenant_id();
        sagat_infra_postgres::PgIdentityStore::new(self.db.pool.clone())
            .insert_membership(&sagat_domain::identity::Membership::restore(
                tenant,
                subject(name),
                role,
                now(),
            ))
            .await
            .expect("add the member");
        Principal::new(tenant, subject(name), role)
    }

    /// Stores `value` as the next version of the secret `name` of the first tenant's namespace.
    pub async fn put_secret(&self, name: &str, value: &str) {
        self.secrets
            .put(
                &self.admin,
                self.namespace.id(),
                name,
                value.as_bytes().to_vec(),
                &ctx(),
            )
            .await
            .expect("store the secret");
    }

    /// An event dispatcher with other limits than the default.
    pub fn dispatcher_with(&self, settings: EventDispatchSettings) -> EventLoop {
        EventDispatcher::new(
            self.store.clone(),
            Engine::new(JsonEvaluator),
            SystemEntropy,
            self.cache.clone(),
            settings,
        )
        .with_cron(Arc::new(sagat_infra_ir::cron::CronCalendar))
    }

    /// [`Self::put_secret`] for a namespace other than `payments`.
    pub async fn put_secret_in(&self, namespace: NamespaceId, name: &str, value: &str) {
        self.secrets
            .put(
                &self.admin,
                namespace,
                name,
                value.as_bytes().to_vec(),
                &ctx(),
            )
            .await
            .expect("store the secret");
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

    /// A run of the *second* tenant (the workflow must have been published there).
    pub async fn start_as_other_tenant(
        &self,
        workflow: WorkflowId,
        input: &Value,
    ) -> Result<RunId, sagat_application::error::AppError> {
        let bytes = serde_json::to_vec(input).expect("json");
        self.runs
            .start(
                &self.other_admin,
                &StartRun {
                    namespace: "payments",
                    workflow,
                    version: None,
                    input: &bytes,
                    correlation_id: None,
                    deadline: None,
                    key: &format!("start-{}", Uuid::now_v7()),
                },
                &ctx(),
            )
            .await
            .map(|r| r.info.run_id)
    }

    /// Another replica of the control plane: its own connection pool and its own services over the
    /// same database, as a second process would have.
    pub async fn replica(&self) -> Replica {
        self.replica_with_pool(10).await
    }

    /// A replica whose pool holds at most `max_connections`, to show that no call needs a second
    /// connection while it holds one.
    pub async fn replica_with_pool(&self, max_connections: u32) -> Replica {
        let mut config = PoolConfig::new(if super::as_runtime_role() {
            self.db
                .url_for(super::RUNTIME_ROLE, super::RUNTIME_ROLE_PASSWORD)
        } else {
            self.db.url.clone()
        });
        config.max_connections = max_connections;
        config.acquire_timeout = std::time::Duration::from_secs(2);
        let pool = connect(&config).await.expect("a second pool");
        let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 64);
        let store = PgRunStore::new(pool, cache.clone());
        Replica {
            runs: RunService::new(
                store.clone(),
                SystemClock,
                Engine::new(JsonEvaluator),
                cache,
            ),
            workers: WorkerService::new(
                store.clone(),
                SystemClock,
                Engine::new(JsonEvaluator),
                SystemEntropy,
                WorkerSettings::default(),
            ),
            store,
        }
    }

    /// Another namespace of the first tenant (and its workspace).
    pub async fn add_namespace(&self, name: &str) -> Namespace {
        self.add_namespace_in(name, Environment::Dev).await
    }

    /// A namespace of the first tenant in the given environment, with a workspace of its own.
    pub async fn add_namespace_in(&self, name: &str, environment: Environment) -> Namespace {
        namespace_in(
            &sagat_infra_postgres::PgIdentityStore::new(self.db.pool.clone()),
            &self.admin,
            name,
            environment,
        )
        .await
    }

    /// Binds a worker identity to every activity of its tenant's namespace `payments` (a worker
    /// without a binding is refused at registration). Idempotent, and never narrows.
    pub async fn bind_everything(&self, principal: &Principal) {
        let (admin, namespace) = if principal.tenant_id() == self.admin.tenant_id() {
            (&self.admin, &self.namespace)
        } else {
            (&self.other_admin, &self.other_namespace)
        };
        self.bindings
            .bind(
                admin,
                namespace.id(),
                principal.subject().as_str(),
                &["*".to_owned()],
                &ctx(),
            )
            .await
            .expect("bind the worker");
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
        self.bind_everything(principal).await;
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

    /// A reconciliation's answer: what the provider says about the effect.
    pub async fn answer_reconciliation(
        &self,
        session: &Session,
        task: &ClaimedTask,
        effect: &str,
        output: Option<Value>,
    ) -> Result<StepAck, sagat_application::error::AppError> {
        let mut answer = json!({ "effect": effect });
        if let Some(output) = output {
            answer["output"] = output;
        }
        self.complete(session, task, &answer).await
    }

    /// An operator acts on a step that waits for a decision, as the first tenant's administrator.
    pub async fn review(
        &self,
        run: RunId,
        node: &str,
        action: sagat_application::runs::ReviewAction,
        reason: &str,
    ) -> Result<sagat_application::execution::RunRecord, sagat_application::error::AppError> {
        self.review_as(&self.admin, run, node, action, reason).await
    }

    pub async fn review_as(
        &self,
        principal: &Principal,
        run: RunId,
        node: &str,
        action: sagat_application::runs::ReviewAction,
        reason: &str,
    ) -> Result<sagat_application::execution::RunRecord, sagat_application::error::AppError> {
        self.runs
            .review(
                principal,
                run,
                &sagat_domain::workflow::NodeId::parse_stored(node).expect("a node id"),
                &action,
                reason,
                None,
                &format!("review-{}", Uuid::now_v7()),
                &ctx(),
            )
            .await
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

    /// Polls and returns the task for `activity` among those leased (panics when there is none).
    pub async fn one_task_of(&self, session: &Session, activity: &str) -> ClaimedTask {
        let tasks = self.poll(session, 8).await;
        let seen: Vec<String> = tasks
            .iter()
            .map(|t| t.activity.as_str().to_owned())
            .collect();
        tasks
            .into_iter()
            .find(|t| t.activity.as_str() == activity)
            .unwrap_or_else(|| panic!("no task for {activity}, got {seen:?}"))
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

    /// A failure with a code of the worker's choosing (`OUTCOME_UNKNOWN` is the reserved one).
    pub async fn fail_with(
        &self,
        session: &Session,
        task: &ClaimedTask,
        code: &str,
        retryable: bool,
    ) -> Result<StepAck, sagat_application::error::AppError> {
        self.workers
            .fail(
                &session.principal,
                &task_ref(task),
                code,
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

/// A second process over the same database (see [`World::replica`]).
pub struct Replica {
    pub store: PgRunStore,
    pub runs: Runs,
    pub workers: Workers,
}

pub struct Session {
    pub principal: Principal,
    pub worker_id: Uuid,
    pub token: String,
}

pub fn task_ref(task: &ClaimedTask) -> TaskRef<'_> {
    TaskRef {
        run_id: task.run_id,
        instance: &task.instance,
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
