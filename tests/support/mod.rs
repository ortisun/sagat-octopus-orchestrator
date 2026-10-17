//! Shared test support: a real PostgreSQL per test process and a fresh database per test.
//!
//! One PostgreSQL 16 container is shared by the tests of a process. It is reference counted: every
//! [`TestDb`] holds an `Arc` of the server, and the container is removed when the last one is
//! dropped (a `static` would never run its destructor and would leak a container per run). The
//! Testcontainers `watchdog` feature also removes it on SIGINT/SIGTERM/SIGQUIT.
//!
//! Set `SAGAT_TEST_DATABASE_URL` to an admin connection URL (e.g.
//! `postgres://postgres:postgres@127.0.0.1:55432/postgres`) to reuse an existing server instead;
//! test databases are then left behind for inspection.
#![allow(dead_code, clippy::expect_used, clippy::panic)]

pub mod api;
pub mod blob_collections;
pub mod brokers;
pub mod collector;
pub mod contract;
pub mod dx;
pub mod engine;
pub mod logs;
pub mod nats;
pub mod oidc;
pub mod receiver;
pub mod scale;
pub mod scope_flow;
pub mod scripts;
pub mod signed;
pub mod worker;

use std::sync::{Arc, LazyLock, Mutex, Weak};

use sagat_domain::identity::{Membership, Principal, Slug, Subject, Tenant, TenantId};
use sagat_infra_postgres::{PgIdentityStore, PgPool, migrate};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Connection, PgConnection};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use testcontainers_modules::testcontainers::{ContainerAsync, ImageExt};
use time::OffsetDateTime;
use tokio::runtime::Runtime;

/// Long-lived runtime that owns the container's background tasks; per-test runtimes come and go.
static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("support runtime")
});

struct Server {
    _container: Option<ContainerAsync<Postgres>>,
    admin_url: String,
}

/// Weak so the container's lifetime is owned by the tests that use it, not by the process.
static SERVER: Mutex<Weak<Server>> = Mutex::new(Weak::new());

/// Keeps the shared PostgreSQL server alive while it is held. A suite that boots many platforms one after
/// another holds it for its whole play: without that the container is torn down between two platforms and
/// another one started, seconds each time and a chance each time to meet a port that is not ready.
pub struct HeldServer(#[allow(dead_code)] Arc<Server>);

/// Starts the shared server if it is not running and holds it until the guard is dropped.
pub fn hold_server() -> HeldServer {
    HeldServer(server())
}

/// Returns the running server, starting one if no test currently holds it.
fn server() -> Arc<Server> {
    let mut slot = SERVER.lock().expect("server lock");
    if let Some(running) = slot.upgrade() {
        return running;
    }
    // Started on a helper thread so this also works when first touched from inside a runtime.
    // Holding the lock meanwhile makes concurrent tests wait for the one container instead of
    // each starting their own.
    let started = Arc::new(std::thread::scope(|scope| {
        scope
            .spawn(|| RUNTIME.block_on(start_server()))
            .join()
            .expect("server thread")
    }));
    *slot = Arc::downgrade(&started);
    started
}

async fn start_server() -> Server {
    if let Ok(admin_url) = std::env::var("SAGAT_TEST_DATABASE_URL") {
        ensure_runtime_role(&admin_url).await;
        return Server {
            _container: None,
            admin_url,
        };
    }
    let container = Postgres::default()
        .with_tag("16-alpine")
        .start()
        .await
        .expect("start PostgreSQL (is Docker running? or set SAGAT_TEST_DATABASE_URL)");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("mapped port");
    let admin_url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    wait_until_queryable(&admin_url).await;
    ensure_runtime_role(&admin_url).await;
    Server {
        admin_url,
        _container: Some(container),
    }
}

/// Blocks until a real PostgreSQL session works.
///
/// The container's "ready to accept connections" log line is printed first by the temporary
/// server that the image's entrypoint runs during initialisation (it has no TCP listener), and the
/// host port is already open through Docker's proxy. Tests that start in that window got
/// `unexpected response from SSLRequest` (about 1 cold start in 8). Only an actual query proves
/// the real server is up.
async fn wait_until_queryable(url: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let attempt = async {
            let mut conn = PgConnection::connect(url).await?;
            sqlx::query("SELECT 1").execute(&mut conn).await?;
            conn.close().await
        };
        match attempt.await {
            Ok(()) => return,
            Err(error) if std::time::Instant::now() < deadline => {
                let _ = error; // not ready yet; keep polling
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            Err(error) => panic!("PostgreSQL never became queryable: {error}"),
        }
    }
}

/// A database created just for one test.
pub struct TestDb {
    /// Keeps the shared server (and its container) alive while this database is in use.
    _server: Arc<Server>,
    pub pool: PgPool,
    pub url: String,
    pub admin_url: String,
    pub name: String,
}

/// Creates an empty database (no schema) and connects a pool to it.
pub async fn fresh_db() -> TestDb {
    let server = server();
    let admin_url = server.admin_url.clone();
    let name = format!("t_{}", TenantId::generate().as_uuid().simple());
    let mut admin = PgConnection::connect(&admin_url)
        .await
        .expect("admin connection");
    // `name` is `t_` + a hex UUID generated above, never user input, so this dynamic SQL is safe.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE \"{name}\"")))
        .execute(&mut admin)
        .await
        .expect("create database");
    let base = admin_url.rsplit_once('/').expect("url with database").0;
    let url = format!("{base}/{name}");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("pool");
    TestDb {
        _server: server,
        pool,
        url,
        admin_url,
        name,
    }
}

/// Creates a database with all migrations applied.
pub async fn migrated_db() -> TestDb {
    let db = fresh_db().await;
    migrate(&db.pool).await.expect("migrate");
    db
}

/// Current time truncated to microseconds, the precision PostgreSQL stores.
pub fn now() -> OffsetDateTime {
    let t = OffsetDateTime::now_utc();
    t.replace_nanosecond(t.nanosecond() / 1_000 * 1_000)
        .expect("valid nanosecond")
}

pub fn slug(value: &str) -> Slug {
    Slug::parse(value).expect("valid slug")
}

pub fn subject(value: &str) -> Subject {
    Subject::parse(value).expect("valid subject")
}

/// Two isolated tenants, each with an administrator, persisted through the real store.
pub struct TwoTenants {
    pub store: PgIdentityStore,
    pub tenant_a: Tenant,
    pub admin_a: Principal,
    pub tenant_b: Tenant,
    pub admin_b: Principal,
}

pub async fn two_tenants(db: &TestDb) -> TwoTenants {
    let store = PgIdentityStore::new(db.pool.clone());
    let (tenant_a, admin_a) = provision(&store, "acme", "alice").await;
    let (tenant_b, admin_b) = provision(&store, "globex", "bob").await;
    TwoTenants {
        store,
        tenant_a,
        admin_a,
        tenant_b,
        admin_b,
    }
}

pub async fn provision(
    store: &PgIdentityStore,
    tenant_slug: &str,
    admin: &str,
) -> (Tenant, Principal) {
    let tenant = Tenant::provision(TenantId::generate(), slug(tenant_slug), now());
    let membership = Membership::bootstrap_admin(&tenant, subject(admin), now());
    store
        .provision_tenant(&tenant, &membership)
        .await
        .expect("provision tenant");
    (tenant, membership.principal())
}

/// SQLSTATE and constraint name of a database error; panics for any other error kind.
pub fn db_error(err: sqlx::Error) -> (String, Option<String>) {
    match err {
        sqlx::Error::Database(db) => (
            db.code().map(|c| c.to_string()).unwrap_or_default(),
            db.constraint().map(str::to_owned),
        ),
        other => panic!("expected a database error, got {other:?}"),
    }
}

/// Polls `check` until it yields a value, failing the test after five seconds.
///
/// For effects that are deliberately written off the request path (the cross-tenant denial audit).
pub async fn eventually<T, F, Fut>(what: &str, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(value) = check().await {
            return value;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for: {what}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

/// Password of the least-privilege runtime role in the test cluster (test-only value).
pub const RUNTIME_ROLE: &str = "sagat_app";
pub const RUNTIME_ROLE_PASSWORD: &str = "runtime-role-test-password";

/// Whether the suites were asked to run the services under test as the runtime role
/// (`SAGAT_TEST_AS_RUNTIME_ROLE=1`): a sweep for privilege gaps the owner-connected suites cannot
/// see (G294). The fixtures are still seeded by the owner.
pub fn as_runtime_role() -> bool {
    std::env::var_os("SAGAT_TEST_AS_RUNTIME_ROLE").is_some()
}

/// Creates the runtime role once per cluster. It must exist before migrations run so they can grant
/// to it, exactly as an operator would provision it.
async fn ensure_runtime_role(admin_url: &str) {
    let mut conn = PgConnection::connect(admin_url)
        .await
        .expect("admin connection");
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DO $$ BEGIN CREATE ROLE {RUNTIME_ROLE} LOGIN PASSWORD '{RUNTIME_ROLE_PASSWORD}' \
         NOSUPERUSER NOCREATEDB NOCREATEROLE; EXCEPTION WHEN duplicate_object THEN NULL; END $$"
    )))
    .execute(&mut conn)
    .await
    .expect("create the runtime role");
}

impl TestDb {
    /// A pool connected as the least-privilege runtime role (`sagat_app`), the role the control
    /// plane and the engine are required to run as. Seed with `pool` (the owner), run the code under
    /// test on this one: the owner can do anything, so a statement the runtime role may not run
    /// passes every test that connects as the owner (G294).
    pub async fn runtime_pool(&self) -> PgPool {
        self.runtime_pool_with(2).await
    }

    /// [`Self::runtime_pool`] with room for `max_connections`.
    pub async fn runtime_pool_with(&self, max_connections: u32) -> PgPool {
        PgPoolOptions::new()
            .max_connections(max_connections)
            .connect(&self.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD))
            .await
            .expect("connect as the runtime role")
    }

    /// The connection URL of this database for another user.
    pub fn url_for(&self, user: &str, password: &str) -> String {
        let (_, rest) = self.url.split_once('@').expect("credentials in url");
        format!("postgres://{user}:{password}@{rest}")
    }
}
