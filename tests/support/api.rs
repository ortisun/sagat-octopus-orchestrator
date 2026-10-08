//! Boots the real HTTP API (router, verifier, service, PostgreSQL store) on an ephemeral port,
//! wired to the fake OIDC provider. Tests talk to it with a plain HTTP client.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use reqwest::{Client, Method, RequestBuilder, Response};
use sagat_api_http::{
    AppState, AuthConfig, HttpJwksFetcher, HttpLimits, RateLimitConfig, ServerConfig,
    TokenVerifier, auth::install_crypto_provider, router,
};
use sagat_application::clock::SystemClock;
use sagat_application::identity::IdentityService;
use sagat_infra_postgres::{PgIdentityStore, PgPool, PoolConfig, connect};
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
}

impl Default for Options {
    fn default() -> Self {
        Self {
            auth: Box::new(|_| {}),
            limits: HttpLimits::default(),
            statement_timeout: Duration::from_secs(2),
            server: ServerConfig::default(),
            runtime_role: false,
            rate_limit: RateLimitConfig::default(),
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
    pool_config.acquire_timeout = Duration::from_secs(2);
    pool_config.statement_timeout = options.statement_timeout;
    let api_pool = connect(&pool_config).await.expect("api pool");
    let store = PgIdentityStore::new(api_pool.clone());
    let fetcher = HttpJwksFetcher::new(&auth).expect("fetcher");
    let verifier = TokenVerifier::new(Arc::new(fetcher), &auth);
    let service = IdentityService::new(store.clone(), SystemClock);
    let app = router(
        AppState::with_rate_limit(service, verifier, options.rate_limit),
        options.limits,
    );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind api");
    let addr = listener.local_addr().expect("addr");
    let server = options.server;
    tokio::spawn(async move {
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
    }
}

impl TestApi {
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
