//! The real `control-plane` and `execution-engine` executables as operating-system processes of their own, started with
//! the flow-control switches and the digest key file (`support::flow_control::process_env`, D022-057) and killed with
//! `SIGKILL`: nothing of theirs runs after the signal, no pool is closed and no lease is given back, as after `kill -9`.
//!
//! The executables are the ones Cargo built beside this test (`cargo build -p sagat-control-plane --bin control-plane
//! -p sagat-execution-engine --bin execution-engine`), or those `SAGAT_CONTROL_PLANE_BIN` and `SAGAT_EXECUTION_ENGINE_BIN`
//! name, as the database runner's process tests find theirs. Each process logs to a file of the test's directory, which
//! a failure prints.
//!
//! What a test waits for is observable state, polled with a bound that names it ([`until`]): a process's probe, its
//! registration, a backend of it waiting for a lock the test holds, the end of its backends after a kill.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::support::oidc::{AUDIENCE, ISSUER};
use crate::support::worker::{Platform, engine_env};
use crate::support::{RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD, as_runtime_role};

/// How long a process may take to serve, and a state the test waits for to appear: generous, for a loaded machine.
const START_LIMIT: Duration = Duration::from_secs(90);
/// How long a state the test waits for may take to appear.
pub const STATE_LIMIT: Duration = Duration::from_secs(90);
/// The shortest shard lease an engine accepts (`SAGAT_SHARD_LEASE_SECS`): a restarted engine takes the shards of the one
/// that was killed once its leases run out, so a restart costs this much.
const SHARD_LEASE_SECS: &str = "3";

/// Which executable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    ControlPlane,
    Engine,
}

impl Kind {
    const fn binary(self) -> (&'static str, &'static str) {
        match self {
            Self::ControlPlane => ("SAGAT_CONTROL_PLANE_BIN", "control-plane"),
            Self::Engine => ("SAGAT_EXECUTION_ENGINE_BIN", "execution-engine"),
        }
    }

    const fn registered_as(self) -> &'static str {
        match self {
            Self::ControlPlane => "CONTROL_PLANE",
            Self::Engine => "EXECUTION_ENGINE",
        }
    }
}

/// A running process of the platform.
pub struct Process {
    kind: Kind,
    name: String,
    child: Option<tokio::process::Child>,
    log: PathBuf,
    /// The control plane's REST base (`http://127.0.0.1:port/v1`), or the engine's worker API (`http://127.0.0.1:port`).
    pub url: String,
    /// The `application_name` of its database connections (`sagat-octopus-orchestrator/<instance id>`, design 041).
    pub connections: String,
}

/// Polls `check` until it yields a value, failing with `what` after `limit`. Every wait of these tests is one: on a state
/// the database or a process shows, never on time standing in for an order.
pub async fn until<T, F, Fut>(what: &str, limit: Duration, mut check: F) -> T
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Option<T>>,
{
    let deadline = Instant::now() + limit;
    loop {
        if let Some(value) = check().await {
            return value;
        }
        assert!(Instant::now() < deadline, "not within {limit:?}: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port()
}

fn executable(kind: Kind) -> PathBuf {
    let (variable, name) = kind.binary();
    let path = std::env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::support::built_binary(name));
    assert!(
        path.is_file(),
        "{} is not built: run `cargo build -p sagat-control-plane --bin control-plane -p sagat-execution-engine \
         --bin execution-engine` with this test's target directory, or name it with {variable}",
        path.display()
    );
    path
}

/// The database URL the processes connect with: the runtime role under `SAGAT_TEST_AS_RUNTIME_ROLE=1`, as production.
fn database_url(p: &Platform) -> String {
    if as_runtime_role() {
        p.api.db.url_for(RUNTIME_ROLE, RUNTIME_ROLE_PASSWORD)
    } else {
        p.api.db.url.clone()
    }
}

/// What both processes are told: the database, the identity provider of the platform, a generous budget per caller,
/// and both flow-control switches with the digest key file (D022-057).
fn common_env(p: &Platform) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = [
        ("SAGAT_ENV", "dev"),
        ("SAGAT_LOG_FORMAT", "json"),
        ("SAGAT_OIDC_ISSUER", ISSUER),
        ("SAGAT_OIDC_AUDIENCE", AUDIENCE),
        ("SAGAT_ALLOW_INSECURE_JWKS_HTTP", "true"),
        ("SAGAT_SUBJECT_RATE_LIMIT_RPS", "1000"),
        ("SAGAT_SUBJECT_RATE_LIMIT_BURST", "2000"),
        ("SAGAT_RATE_LIMIT_RPS", "1000"),
        ("SAGAT_RATE_LIMIT_BURST", "2000"),
    ]
    .into_iter()
    .map(|(name, value)| (name.to_owned(), value.to_owned()))
    .collect();
    env.insert("SAGAT_OIDC_JWKS_URI".to_owned(), p.api.idp.jwks_url());
    env.insert("DATABASE_URL".to_owned(), database_url(p));
    env.insert(
        "RUST_LOG".to_owned(),
        std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_owned()),
    );
    for (name, value) in crate::support::flow_control::process_env() {
        env.insert(name.to_owned(), value);
    }
    env
}

/// The instances of `kind` the registry lists.
async fn instances(p: &Platform, kind: Kind) -> Vec<Uuid> {
    sqlx::query_scalar("SELECT instance_id FROM serving_instances WHERE kind = $1")
        .bind(kind.registered_as())
        .fetch_all(&p.api.db.pool)
        .await
        .expect("the registry")
}

impl Process {
    /// Starts a control plane on a port of its own and returns once it serves `GET /v1/health`.
    pub async fn control_plane(p: &Platform, dir: &Path, name: &str) -> Self {
        let port = free_port();
        let mut env = common_env(p);
        env.insert("SAGAT_BIND_ADDR".to_owned(), format!("127.0.0.1:{port}"));
        env.insert("SAGAT_ADMIN_ADDR".to_owned(), "off".to_owned());
        let url = format!("http://127.0.0.1:{port}/v1");
        let probe = format!("{url}/health");
        Self::start(p, Kind::ControlPlane, dir, name, env, url, probe).await
    }

    /// Starts an engine (scheduler loop, worker API and dispatchers) whose loop ticks every 50 ms and whose shard leases
    /// last the shortest time allowed, and returns once it is ready.
    pub async fn engine(p: &Platform, dir: &Path, name: &str) -> Self {
        let (grpc, admin) = (free_port(), free_port());
        let grpc_addr = format!("127.0.0.1:{grpc}");
        let admin_addr = format!("127.0.0.1:{admin}");
        let base = engine_env(
            &p.api.db,
            &p.api.idp,
            &[
                ("SAGAT_GRPC_ADDR", grpc_addr.as_str()),
                ("SAGAT_ADMIN_ADDR", admin_addr.as_str()),
                ("SAGAT_SHARD_LEASE_SECS", SHARD_LEASE_SECS),
                ("SAGAT_REPLICA_ID", name),
                // a tick blocked on a lock the test holds is what a test of a kill waits for, not a liveness failure
                ("SAGAT_LIVENESS_GRACE_SECS", "600"),
                // a step leased before a kill is completed after the restart: its lease outlives both
                ("SAGAT_LEASE_TTL_SECS", "600"),
            ],
        );
        let mut env = common_env(p);
        for (key, value) in base {
            // the database of `common_env` (the runtime role when asked) wins over the owner URL of `engine_env`
            env.entry(key).or_insert(value);
        }
        let url = format!("http://{grpc_addr}");
        let probe = format!("http://{admin_addr}/readyz");
        Self::start(p, Kind::Engine, dir, name, env, url, probe).await
    }

    async fn start(
        p: &Platform,
        kind: Kind,
        dir: &Path,
        name: &str,
        env: HashMap<String, String>,
        url: String,
        probe: String,
    ) -> Self {
        let before = instances(p, kind).await;
        let log = dir.join(format!("{name}.log"));
        let file = std::fs::File::create(&log).expect("a log file");
        let child = tokio::process::Command::new(executable(kind))
            .arg("serve")
            .env_clear()
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::from(file.try_clone().expect("the log file")))
            .stderr(Stdio::from(file))
            .kill_on_drop(true)
            .spawn()
            .unwrap_or_else(|error| panic!("{name} did not start: {error}"));
        let mut process = Self {
            kind,
            name: name.to_owned(),
            child: Some(child),
            log,
            url,
            connections: String::new(),
        };
        let client = reqwest::Client::new();
        let deadline = Instant::now() + START_LIMIT;
        loop {
            if let Some(child) = process.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                panic!(
                    "{name} exited before it served ({status}):\n{}",
                    process.log_text()
                );
            }
            let served = client
                .get(&probe)
                .timeout(Duration::from_secs(2))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            if served {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{name} did not serve within {START_LIMIT:?}:\n{}",
                process.log_text()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        // the process registered before it served (FR-018-052): the new row of its kind is it, and names its connections
        let instance = until(&format!("{name} in the registry"), STATE_LIMIT, || async {
            instances(p, kind)
                .await
                .into_iter()
                .find(|id| !before.contains(id))
        })
        .await;
        process.connections = format!("sagat-octopus-orchestrator/{instance}");
        process
    }

    /// What the process logged so far (for a failure's message).
    pub fn log_text(&self) -> String {
        std::fs::read_to_string(&self.log).unwrap_or_default()
    }

    /// `kill -9`: `SIGKILL`, then the process is reaped. Its database backends end once they next read from or write to
    /// it ([`Self::until_gone`]).
    pub async fn kill(&mut self) {
        if let Some(mut child) = self.child.take() {
            child.start_kill().expect("SIGKILL");
            let status = child.wait().await.expect("reaped");
            assert!(
                !status.success(),
                "{} was killed, not stopped: {status}",
                self.name
            );
        }
    }

    /// How many backends of this process wait for a lock: a statement of it blocked behind a lock the test holds.
    pub async fn waiting_for_a_lock(&self, p: &Platform) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM pg_locks l JOIN pg_stat_activity a ON a.pid = l.pid \
              WHERE NOT l.granted AND a.application_name = $1 AND a.datname = current_database()",
        )
        .bind(&self.connections)
        .fetch_one(&p.api.db.pool)
        .await
        .expect("the locks")
    }

    /// Returns once a backend of this process waits for a lock.
    pub async fn until_blocked(&self, p: &Platform) {
        until(
            &format!(
                "{} ({:?}) blocked on a lock the test holds",
                self.name, self.kind
            ),
            STATE_LIMIT,
            || async { (self.waiting_for_a_lock(p).await > 0).then_some(()) },
        )
        .await;
    }

    /// Returns once no backend of this (killed) process is left: what a killed process had begun is rolled back, and
    /// nothing of it can commit any more.
    pub async fn until_gone(&self, p: &Platform) {
        until(
            &format!("the backends of the killed {} ended", self.name),
            STATE_LIMIT,
            || async {
                let left: i64 = sqlx::query_scalar(
                    "SELECT count(*) FROM pg_stat_activity WHERE application_name = $1",
                )
                .bind(&self.connections)
                .fetch_one(&p.api.db.pool)
                .await
                .expect("the backends");
                (left == 0).then_some(())
            },
        )
        .await;
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.start_kill();
        }
        if std::thread::panicking() {
            eprintln!("--- log of {} ---\n{}", self.name, self.log_text());
        }
    }
}
