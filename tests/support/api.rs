//! Boots the real HTTP API (router, verifier, service, PostgreSQL store) on an ephemeral port,
//! wired to the fake OIDC provider. Tests talk to it with a plain HTTP client.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, Response};
use sagat_api_grpc::{GrpcConfig, WorkerGrpc, serve as serve_grpc, worker_service};
use sagat_api_http::{
    AppState, AuthConfig, ExecState, GovState, HttpJwksFetcher, HttpLimits, RateLimitConfig,
    ServerConfig, TokenVerifier, auth::install_crypto_provider, router_with_governance,
};
use sagat_application::bindings::BindingService;
use sagat_application::clock::SystemClock;
use sagat_application::connectors::ConnectorService;
use sagat_application::definitions::DefinitionService;
use sagat_application::egress::EgressService;
use sagat_application::entropy::SystemEntropy;
use sagat_application::error::StoreError;
use sagat_application::events::EventService;
use sagat_application::execution::{DefinitionCache, Engine};
use sagat_application::identity::{IdentityService, IdentityStore};
use sagat_application::outbound::OutboundService;
use sagat_application::quotas::QuotaService;
use sagat_application::runs::RunService;
use sagat_application::script_signing::Signer;
use sagat_application::scripts::{RunnerService, ScriptService};
use sagat_application::secrets::SecretService;
use sagat_application::worker::{WorkerService, WorkerSettings};
use sagat_domain::audit::AuditRecord;
use sagat_domain::identity::{
    Membership, Namespace, NamespaceId, Slug, Subject, Tenant, TenantId, Workspace, WorkspaceId,
};
use sagat_infra_ir::{JsonEvaluator, JsonIrCodec};
use sagat_infra_postgres::{PgIdentityStore, PgPool, PgRunStore, PoolConfig, connect};
use serde_json::{Value, json};

use super::oidc::{AUDIENCE, ISSUER, TestIdp};
use super::{TestDb, migrated_db, provision};

pub struct TestApi {
    pub base: String,
    pub client: Client,
    pub idp: TestIdp,
    pub db: TestDb,
    pub store: PgIdentityStore,
    /// Provisioning handle that always runs as the database owner (the API may not be allowed to).
    pub owner_store: PgIdentityStore,
    /// The pool the API itself uses (`db.pool` is the tests' own administrative handle).
    pub api_pool: PgPool,
    /// The run store the API (and the gRPC worker service) use.
    pub executions: PgRunStore,
    /// `http://host:port` of the gRPC worker service.
    pub grpc_url: String,
    /// The key ring the API seals secrets with and the worker service opens them with.
    pub keys: sagat_infra_secrets::SecretsConfig,
    /// The base64 material of key `k1`, so an engine started for this API can open its secrets.
    pub key_k1: String,
    /// The public half of the key that signs the artifacts this API builds.
    pub script_key: Vec<u8>,
    lookups: Arc<AtomicUsize>,
    pings: Arc<AtomicUsize>,
    /// The REST and gRPC servers, which have nothing to make them stop: they end with the API.
    servers: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for TestApi {
    /// A suite that boots platform after platform in one process (the SDK conformance suite) would otherwise
    /// keep every platform's servers, and the database pools they hold, until the process ends.
    fn drop(&mut self) {
        for server in &self.servers {
            server.abort();
        }
    }
}

/// The store the API uses, counting the authentication lookups (tenant by slug, membership) and the
/// readiness pings so tests can prove that a throttled request never reached the database and that
/// a flood of probes costs one ping. It can also make the ping slow.
#[derive(Clone)]
struct CountingStore {
    inner: PgIdentityStore,
    lookups: Arc<AtomicUsize>,
    pings: Arc<AtomicUsize>,
    ping_delay: Duration,
}

impl IdentityStore for CountingStore {
    async fn find_tenant_by_slug(&self, slug: &Slug) -> Result<Option<Tenant>, StoreError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.inner.find_tenant_by_slug(slug).await
    }

    async fn find_membership(
        &self,
        tenant: TenantId,
        subject: &Subject,
    ) -> Result<Option<Membership>, StoreError> {
        self.lookups.fetch_add(1, Ordering::SeqCst);
        self.inner.find_membership(tenant, subject).await
    }

    async fn create_workspace(
        &self,
        workspace: &Workspace,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        self.inner.create_workspace(workspace, audit).await
    }

    async fn get_workspace(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> Result<Option<Workspace>, StoreError> {
        self.inner.get_workspace(tenant, id).await
    }

    async fn list_workspaces(
        &self,
        tenant: TenantId,
        after: Option<WorkspaceId>,
        limit: u32,
    ) -> Result<Vec<Workspace>, StoreError> {
        self.inner.list_workspaces(tenant, after, limit).await
    }

    async fn create_namespace(
        &self,
        namespace: &Namespace,
        audit: &AuditRecord,
    ) -> Result<(), StoreError> {
        self.inner.create_namespace(namespace, audit).await
    }

    async fn get_namespace(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<Option<Namespace>, StoreError> {
        self.inner.get_namespace(tenant, id).await
    }

    async fn list_namespaces(
        &self,
        tenant: TenantId,
        after: Option<NamespaceId>,
        limit: u32,
    ) -> Result<Vec<Namespace>, StoreError> {
        self.inner.list_namespaces(tenant, after, limit).await
    }

    async fn workspace_exists_outside(
        &self,
        tenant: TenantId,
        id: WorkspaceId,
    ) -> Result<bool, StoreError> {
        self.inner.workspace_exists_outside(tenant, id).await
    }

    async fn namespace_exists_outside(
        &self,
        tenant: TenantId,
        id: NamespaceId,
    ) -> Result<bool, StoreError> {
        self.inner.namespace_exists_outside(tenant, id).await
    }

    async fn record_denial(&self, audit: &AuditRecord) -> Result<(), StoreError> {
        self.inner.record_denial(audit).await
    }

    async fn ping(&self) -> Result<(), StoreError> {
        self.pings.fetch_add(1, Ordering::SeqCst);
        if !self.ping_delay.is_zero() {
            tokio::time::sleep(self.ping_delay).await;
        }
        self.inner.ping().await
    }
}

/// Settings tests may tighten before the API starts.
pub struct Options {
    pub auth: Box<dyn FnOnce(&mut AuthConfig)>,
    pub limits: HttpLimits,
    /// Server-side cap per statement for the API's database pool.
    pub statement_timeout: Duration,
    /// Connection-level limits of the HTTP server.
    pub server: ServerConfig,
    /// Connect the API as the least-privilege runtime role instead of the database owner.
    pub runtime_role: bool,
    /// Per-tenant request budget.
    pub rate_limit: RateLimitConfig,
    /// Per-subject request budget, charged before the database is queried. Generous by default
    /// because most tests make many calls as one subject; the tests about it set their own.
    pub subject_rate_limit: RateLimitConfig,
    /// Makes every readiness ping take at least this long (simulates a slow or hanging database).
    pub ping_delay: Duration,
    /// Lease and session timings of the worker protocol.
    pub worker: WorkerSettings,
    /// The most any script may be given.
    pub script_ceiling: sagat_domain::scripts::Policy,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            auth: Box::new(|_| {}),
            limits: HttpLimits::default(),
            // Generous: the suites also run on loaded machines, and the tests about timeouts set their own.
            statement_timeout: Duration::from_secs(10),
            server: ServerConfig::default(),
            runtime_role: false,
            rate_limit: RateLimitConfig::default(),
            subject_rate_limit: RateLimitConfig {
                per_second: 1000.0,
                burst: 2000.0,
            },
            ping_delay: Duration::ZERO,
            worker: WorkerSettings::default(),
            script_ceiling: sagat_domain::scripts::Policy::default(),
        }
    }
}

pub async fn start() -> TestApi {
    start_with(Options::default()).await
}

pub async fn start_with(options: Options) -> TestApi {
    install_crypto_provider();
    let db = migrated_db().await;
    let idp = TestIdp::start().await;

    let mut auth = AuthConfig::new(ISSUER, AUDIENCE, idp.jwks_url());
    auth.allow_insecure_jwks_http = true;
    (options.auth)(&mut auth);

    let api_url = if options.runtime_role {
        db.url_for(super::RUNTIME_ROLE, super::RUNTIME_ROLE_PASSWORD)
    } else {
        db.url.clone()
    };
    let mut pool_config = PoolConfig::new(api_url);
    pool_config.max_connections = 8;
    pool_config.acquire_timeout = Duration::from_secs(5);
    pool_config.statement_timeout = options.statement_timeout;
    let api_pool = connect(&pool_config).await.expect("api pool");
    let store = PgIdentityStore::new(api_pool.clone());
    let fetcher = HttpJwksFetcher::new(&auth).expect("fetcher");
    let verifier = TokenVerifier::new(Arc::new(fetcher), &auth);
    let lookups = Arc::new(AtomicUsize::new(0));
    let pings = Arc::new(AtomicUsize::new(0));
    let service = IdentityService::new(
        CountingStore {
            inner: store.clone(),
            lookups: Arc::clone(&lookups),
            pings: Arc::clone(&pings),
            ping_delay: options.ping_delay,
        },
        SystemClock,
    );
    let state = AppState::with_rate_limit(service, verifier, options.rate_limit)
        .expect("valid budget")
        .with_subject_rate_limit(options.subject_rate_limit)
        .expect("valid subject budget");
    let cache = DefinitionCache::new(Arc::new(JsonIrCodec), 64);
    let executions = PgRunStore::new(api_pool.clone(), cache.clone());
    let key_k1 = sagat_infra_secrets::generate_key().expect("a key");
    let keys =
        sagat_infra_secrets::SecretsConfig::parse(&format!("k1:{key_k1}")).expect("a key ring");
    let signer =
        Signer::from_pkcs8(&Signer::generate_pkcs8().expect("a signing key")).expect("a signer");
    let script_key = signer.public_key();
    let governance = GovState::new(
        state.clone(),
        QuotaService::new(executions.clone(), SystemClock),
        BindingService::new(executions.clone(), SystemClock),
        EgressService::new(executions.clone(), SystemClock),
    )
    .with_secrets(SecretService::new(
        executions.clone(),
        SystemClock,
        keys.cipher(),
    ))
    .with_events(EventService::new(
        executions.clone(),
        SystemClock,
        keys.cipher(),
        Arc::new(JsonEvaluator),
    ))
    .with_outbound(OutboundService::new(
        executions.clone(),
        SystemClock,
        EgressService::new(executions.clone(), SystemClock),
    ))
    .with_connectors(ConnectorService::new(
        executions.clone(),
        SystemClock,
        EgressService::new(executions.clone(), SystemClock),
        Arc::new(JsonEvaluator),
    ))
    .with_scripts(
        ScriptService::new(
            executions.clone(),
            SystemClock,
            sagat_infra_schema::Schemas,
            options.script_ceiling,
        ),
        RunnerService::new(
            executions.clone(),
            SystemClock,
            SystemEntropy,
            signer,
            sagat_infra_schema::bundle::Bundles,
            options.script_ceiling,
        ),
    );
    let exec = ExecState::new(
        state,
        DefinitionService::new(executions.clone(), SystemClock, JsonIrCodec),
        RunService::new(
            executions.clone(),
            SystemClock,
            Engine::new(JsonEvaluator),
            cache,
        ),
    );
    // The worker protocol shares the REST API's identity, budgets and database.
    let grpc = WorkerGrpc::new(
        exec.clone(),
        WorkerService::new(
            executions.clone(),
            SystemClock,
            Engine::new(JsonEvaluator),
            SystemEntropy,
            options.worker,
        )
        .with_vault(keys.cipher()),
    );
    let grpc_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind grpc");
    let grpc_addr = grpc_listener.local_addr().expect("grpc addr");
    let grpc_server = tokio::spawn(async move {
        let _ = serve_grpc(
            grpc_listener,
            worker_service(grpc),
            GrpcConfig::default(),
            std::future::pending(),
        )
        .await;
    });
    let app = router_with_governance(exec, governance, options.limits);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind api");
    let addr = listener.local_addr().expect("addr");
    let server = options.server;
    let http_server = tokio::spawn(async move {
        let _ = sagat_api_http::serve(listener, app, server, std::future::pending()).await;
    });

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("client");
    let owner_store = PgIdentityStore::new(db.pool.clone());
    TestApi {
        base: format!("http://{addr}/v1"),
        client,
        idp,
        db,
        store,
        owner_store,
        api_pool,
        executions,
        grpc_url: format!("http://{grpc_addr}"),
        keys,
        key_k1,
        script_key,
        lookups,
        pings,
        servers: vec![grpc_server, http_server],
    }
}

impl TestApi {
    /// Tenant and membership lookups the API has made so far while authenticating requests.
    pub fn authentication_lookups(&self) -> usize {
        self.lookups.load(Ordering::SeqCst)
    }

    /// Readiness pings that reached the store so far.
    pub fn pings(&self) -> usize {
        self.pings.load(Ordering::SeqCst)
    }

    /// Creates a tenant and its first admin directly in the database.
    pub async fn tenant(&self, slug: &str, admin: &str) -> sagat_domain::identity::TenantId {
        provision(&self.owner_store, slug, admin).await.0.id()
    }

    pub fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client.request(method, format!("{}{path}", self.base))
    }

    /// A request authenticated as `subject` of `tenant` with a freshly signed token.
    pub fn as_user(
        &self,
        method: Method,
        path: &str,
        subject: &str,
        tenant: &str,
    ) -> RequestBuilder {
        let token = self.idp.token(subject, tenant).build();
        self.request(method, path).bearer_auth(token)
    }

    pub async fn get_as(&self, path: &str, subject: &str, tenant: &str) -> Response {
        self.as_user(Method::GET, path, subject, tenant)
            .send()
            .await
            .expect("send")
    }

    pub async fn post_as(&self, path: &str, subject: &str, tenant: &str, body: &Value) -> Response {
        self.as_user(Method::POST, path, subject, tenant)
            .json(body)
            .send()
            .await
            .expect("send")
    }

    /// Creates a workspace as an admin and returns its id.
    pub async fn create_workspace(&self, admin: &str, tenant: &str, slug: &str) -> String {
        let response = self
            .post_as("/workspaces", admin, tenant, &json!({ "slug": slug }))
            .await;
        assert_eq!(response.status(), 201, "create workspace");
        response.json::<Value>().await.expect("json")["id"]
            .as_str()
            .expect("id")
            .to_owned()
    }

    /// Creates a namespace as an admin and returns its id.
    pub async fn create_namespace(
        &self,
        admin: &str,
        tenant: &str,
        workspace_id: &str,
        slug: &str,
        environment: &str,
    ) -> String {
        let response = self
            .post_as(
                &format!("/workspaces/{workspace_id}/namespaces"),
                admin,
                tenant,
                &json!({ "slug": slug, "environment": environment }),
            )
            .await;
        assert_eq!(response.status(), 201, "create namespace");
        response.json::<Value>().await.expect("json")["id"]
            .as_str()
            .expect("id")
            .to_owned()
    }
}

/// Parses a response body as JSON.
pub async fn json_of(response: Response) -> Value {
    response.json::<Value>().await.expect("json body")
}
